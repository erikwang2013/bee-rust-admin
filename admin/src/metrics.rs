// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! `/metrics` 指标端点（项 12）：Prometheus 文本格式，**手写零依赖** ——
//! 这里只要几个计数器和一档固定直方图，引 prometheus 客户端等于为几十行输出
//! 拖进一整套库；格式本身（文本 0.0.4）就那几条规则。
//!
//! 口径与取舍：
//! - 路径标签用**路由模板**（`/api/v1/admins/{id}`）而不是原始路径：原始路径基数
//!   无界，任何扫描器随便打 URL 就能把这张表撑爆内存。模板取自 axum 的
//!   `MatchedPath` —— 路由匹配发生在中间件**之前**（axum 0.8 `path_router.rs` 的
//!   `set_matched_path_for_request` 先于 endpoint 调用），所以 `Router::layer` 挂的
//!   中间件拿得到它。落兜底路由的请求（404 等）没有模板，归到固定标签 `<unmatched>`。
//! - **不加鉴权**：服务只监听 127.0.0.1（`[app] http_addr`），对外转不转由 nginx 决定。
//!   这是运维端点，不是业务接口，不进 `/api/v1` 的鉴权与信封。
//! - 指标是**进程内**的：多实例部署时各算各的，聚合由抓取端（Prometheus）做。
//! - 不进 apidoc 文档站（接口清单只管 `/api/v1` 业务接口）。

use axum::extract::{MatchedPath, Request};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Instant;

/// 进程启动时间（Unix 秒）。`main` 里 [`init`] 主动 force 一次，记的是**进程启动**
/// 而不是「第一个请求」的时刻；万一忘了调，首次读取也会取到当时的时间，不会输出 0。
static START: LazyLock<u64> = LazyLock::new(crate::util::now_unix);

/// 定下启动时刻。在 `main` 启动早期调用一次。
pub fn init() {
    LazyLock::force(&START);
}

/// 状态码大类计数：下标 = 状态码/100 - 2（2xx→0，3xx→1，4xx→2，5xx→3）。
static STATUS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// 直方图上界（秒）与它渲染出来的 `le` 文本。+Inf 不在这里 —— 它恒等于总次数，
/// 渲染时单独一行。档位是给后台接口定的：正常请求都在 5–100ms 档，2s 以上基本是
/// 慢 SQL 或大导出，再细分没有决策价值。
const BUCKETS: [(&str, f64); 5] = [
    ("0.005", 0.005),
    ("0.02", 0.02),
    ("0.1", 0.1),
    ("0.5", 0.5),
    ("2", 2.0),
];

/// 兜底路由（未匹配路径）的固定标签：绝不用原始路径兜底，那正是高基数的来源。
const UNMATCHED: &str = "<unmatched>";

/// 单个路由模板的累计器。全是原子量，渲染时才读：请求路径上只做一次读锁取 `Arc`
/// 加几个原子加，互不阻塞。
struct PathStat {
    count: AtomicU64,
    /// 各档**非累积**观测数（每个观测只落进第一个装得下的档）；
    /// Prometheus 的 bucket 是累积语义，渲染时再累加。
    buckets: [AtomicU64; BUCKETS.len()],
    /// 累计耗时（微秒）。u64 微秒要 58 万年才溢出，不必担心回绕。
    sum_micros: AtomicU64,
}

impl PathStat {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; BUCKETS.len()],
            sum_micros: AtomicU64::new(0),
        }
    }
}

/// 路由模板 → 累计器。模板数量 = 注册的路由数（有界），写锁只在首次见到新模板时拿。
static PATHS: LazyLock<RwLock<BTreeMap<String, Arc<PathStat>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// 取（必要时新建）某路由模板的累计器。读锁的作用域用块显式收紧：
/// 必须在拿写锁**之前**释放，否则同线程自我死锁（std 的 RwLock 不可重入）。
fn stat_for(path: &str) -> Arc<PathStat> {
    {
        let map = PATHS.read().unwrap();
        if let Some(st) = map.get(path) {
            return st.clone();
        }
    }
    PATHS
        .write()
        .unwrap()
        .entry(path.to_string())
        .or_insert_with(|| Arc::new(PathStat::new()))
        .clone()
}

/// 记一次观测：落进第一个 `secs <= 上界` 的档（都不满足就只进 +Inf）。
fn record(st: &PathStat, secs: f64) {
    st.count.fetch_add(1, Ordering::Relaxed);
    st.sum_micros.fetch_add((secs * 1e6) as u64, Ordering::Relaxed);
    for (i, (_, bound)) in BUCKETS.iter().enumerate() {
        if secs <= *bound {
            st.buckets[i].fetch_add(1, Ordering::Relaxed);
            break;
        }
    }
}

/// 记一次请求。`path` 传 [`MatchedPath`] 的模板串（取不到时传 [`UNMATCHED`]）。
fn observe(path: &str, status: u16, secs: f64) {
    if let Some(i) = class_index(status) {
        STATUS[i].fetch_add(1, Ordering::Relaxed);
    }
    record(&stat_for(path), secs);
}

/// 状态码 → 大类下标（2xx→0 … 5xx→3）。1xx 在本项目不出现（探活是 2xx），
/// 不计：与其输出一条恒为 0 的序列，不如不输出。
fn class_index(status: u16) -> Option<usize> {
    match status / 100 {
        c @ 2..=5 => Some((c - 2) as usize),
        _ => None,
    }
}

/// 统计中间件。挂在**最外层**（main.rs 里最后一个 `.layer`）：被安全扫描拦掉（403）、
/// 落到兜底路由（404）的请求也都要计数 —— 这些正是运维要看的信号。
pub async fn metrics_mw(req: Request, next: Next) -> Response {
    // 模板在进入本层之前就已由路由器写进请求扩展（见模块注释），这里只是读出来
    let path = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_string());
    let started = Instant::now();
    let resp = next.run(req).await;
    observe(
        path.as_deref().unwrap_or(UNMATCHED),
        resp.status().as_u16(),
        started.elapsed().as_secs_f64(),
    );
    resp
}

/// `GET /metrics`。文本格式 0.0.4（Prometheus 抓取端按这个 content-type 解析；
/// 不带的话它按老式 "text/plain" 也能认，但显式写上更稳）。
pub async fn endpoint() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        render(),
    )
}

/// 渲染全部指标。路径按字典序（BTreeMap）输出，抓取结果稳定、好 diff。
fn render() -> String {
    let mut out = String::with_capacity(2048);

    out.push_str("# HELP process_start_time_seconds 进程启动时间（Unix 秒）\n");
    out.push_str("# TYPE process_start_time_seconds gauge\n");
    out.push_str(&format!("process_start_time_seconds {}\n", *START));

    out.push_str("# HELP http_requests_total 按状态码大类的请求总数\n");
    out.push_str("# TYPE http_requests_total counter\n");
    for (i, class) in ["2xx", "3xx", "4xx", "5xx"].iter().enumerate() {
        out.push_str(&format!(
            "http_requests_total{{status=\"{class}\"}} {}\n",
            STATUS[i].load(Ordering::Relaxed)
        ));
    }

    // 快照一次：渲染期间不再持锁（新模板照旧被中间件记进表里，下一次抓取看得到）
    let paths: Vec<(String, Arc<PathStat>)> = {
        let map = PATHS.read().unwrap();
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    out.push_str("# HELP http_requests_by_path_total 按路由模板的请求数（模板而非原始路径，防高基数）\n");
    out.push_str("# TYPE http_requests_by_path_total counter\n");
    for (path, st) in &paths {
        out.push_str(&format!(
            "http_requests_by_path_total{{path=\"{path}\"}} {}\n",
            st.count.load(Ordering::Relaxed)
        ));
    }

    out.push_str("# HELP http_request_duration_seconds 请求耗时直方图（按路由模板）\n");
    out.push_str("# TYPE http_request_duration_seconds histogram\n");
    for (path, st) in &paths {
        write_histogram(&mut out, path, st);
    }
    out
}

/// 直方图一个模板的三件套（bucket 累积 / sum / count）。纯函数：喂一个 [`PathStat`]
/// 就出文本 —— 单测直接构造累计器验证累积语义，不必碰全局表。
fn write_histogram(out: &mut String, path: &str, st: &PathStat) {
    let mut cumulative = 0u64;
    for (i, (le, _)) in BUCKETS.iter().enumerate() {
        cumulative += st.buckets[i].load(Ordering::Relaxed);
        out.push_str(&format!(
            "http_request_duration_seconds_bucket{{le=\"{le}\",path=\"{path}\"}} {cumulative}\n"
        ));
    }
    // +Inf 恒等于观测总数（count）：超过最大档的观测不进任何有限档，但都进 count
    let count = st.count.load(Ordering::Relaxed);
    out.push_str(&format!(
        "http_request_duration_seconds_bucket{{le=\"+Inf\",path=\"{path}\"}} {count}\n"
    ));
    let micros = st.sum_micros.load(Ordering::Relaxed);
    out.push_str(&format!(
        "http_request_duration_seconds_sum{{path=\"{path}\"}} {}.{:06}\n",
        micros / 1_000_000,
        micros % 1_000_000
    ));
    out.push_str(&format!(
        "http_request_duration_seconds_count{{path=\"{path}\"}} {count}\n"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只认 2xx–5xx：1xx（本项目不出现）与其它的不给序列，免得输出恒为 0 的噪音。
    #[test]
    fn status_class_maps_only_2xx_to_5xx() {
        assert_eq!(class_index(200), Some(0));
        assert_eq!(class_index(204), Some(0));
        assert_eq!(class_index(301), Some(1));
        assert_eq!(class_index(401), Some(2));
        assert_eq!(class_index(404), Some(2));
        assert_eq!(class_index(500), Some(3));
        assert_eq!(class_index(100), None);
        assert_eq!(class_index(0), None);
    }

    /// 直方图：bucket 是**累积**的、+Inf 等于观测总数、sum 以秒渲染（6 位小数）。
    /// 三个观测分别落在 0.005 / 0.1 / +Inf 三档（0.03 装不进 0.02）。
    #[test]
    fn histogram_is_cumulative_and_inf_matches_count() {
        let st = PathStat::new();
        record(&st, 0.001);
        record(&st, 0.03);
        record(&st, 3.0);

        let mut out = String::new();
        write_histogram(&mut out, "/api/v1/health", &st);
        let line = |le: &str| {
            format!("http_request_duration_seconds_bucket{{le=\"{le}\",path=\"/api/v1/health\"}} ")
        };
        assert!(out.contains(&format!("{}1\n", line("0.005"))), "0.001 进第一档:\n{out}");
        assert!(out.contains(&format!("{}1\n", line("0.02"))), "0.03 装不进 0.02，不累加:\n{out}");
        assert!(out.contains(&format!("{}2\n", line("0.1"))), "0.001+0.03 累积:\n{out}");
        assert!(out.contains(&format!("{}2\n", line("2"))), "3s 不进有限档:\n{out}");
        assert!(
            out.contains("http_request_duration_seconds_bucket{le=\"+Inf\",path=\"/api/v1/health\"} 3\n"),
            "+Inf = 观测总数:\n{out}"
        );
        assert!(out.contains("_sum{path=\"/api/v1/health\"} 3.031000\n"), "sum = 0.001+0.03+3:\n{out}");
        assert!(out.contains("_count{path=\"/api/v1/health\"} 3\n"), "+Inf 与 count 一致:\n{out}");
    }

    /// 渲染出来的骨架要有：启动时间 gauge、四个状态码序列、末尾换行
    /// （文本格式的最后一行也要有 `\n`，少了解析器会吞掉最后一行）。
    #[test]
    fn render_has_required_families() {
        let out = render();
        assert!(out.contains("# TYPE process_start_time_seconds gauge\n"));
        assert!(out.contains("\nprocess_start_time_seconds "));
        for class in ["2xx", "3xx", "4xx", "5xx"] {
            assert!(
                out.contains(&format!("http_requests_total{{status=\"{class}\"}} ")),
                "缺 {class} 序列:\n{out}"
            );
        }
        assert!(out.contains("# TYPE http_requests_by_path_total counter\n"));
        assert!(out.contains("# TYPE http_request_duration_seconds histogram\n"));
        assert!(out.ends_with('\n'));
    }

    /// 同一个模板只建一条累计器；新模板按需新建（读锁 → 写锁的自锁问题由这个用例兜底：
    /// 真死锁了这里会挂住）。
    #[test]
    fn stat_for_reuses_and_creates_entries() {
        let a = stat_for("/api/v1/health");
        let b = stat_for("/api/v1/health");
        record(&a, 0.001);
        assert_eq!(b.count.load(Ordering::Relaxed), 1, "同一模板共用一条累计器");
        assert_eq!(b.sum_micros.load(Ordering::Relaxed), 1000);
        let _ = stat_for("/api/v1/x/{id}");
        assert!(PATHS.read().unwrap().contains_key("/api/v1/x/{id}"));
    }
}
