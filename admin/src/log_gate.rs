// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 匿名写日志的进程内采样闸：同一个 key（来源 IP + 事件类别）每个窗口只放行固定条数。
//!
//! 背景：审计 / 安全扫描 / 登录失败三条路径都会为**未鉴权**请求写库。一个循环发
//! 匿名请求的客户端能把 `audit_log`、`login_log` 写成无底洞（写放大）。这里按来源
//! 限一个窗口内的写入条数：正常排查要的几条照写，刷屏的被压在上限。
//!
//! 只用于匿名 / 失败路径；已鉴权的业务写审计与成功的登录日志不经过这里。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 采样窗口。
const WINDOW: Duration = Duration::from_secs(60);

/// 每个 key 在窗口内的写入条数上限。取 20 是为了「正常运维排查够用」与「刷子被压住」
/// 之间取平衡：一个来源每分钟最多 20 条审计/扫描/失败登录行，三条路径各自独立。
const MAX_PER_WINDOW: u32 = 20;

/// 进程内表上限。IP 头是客户端可控的，不设上限就是另一个内存放大器。
const MAX_KEYS: usize = 4096;

struct Window {
    start: Instant,
    count: u32,
}

fn seen() -> &'static Mutex<HashMap<String, Window>> {
    static SEEN: OnceLock<Mutex<HashMap<String, Window>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 这个 key 在本窗口内还能不能落一行：能则计数并返回 `true`。
///
/// key 由调用方拼接（务必带来源 IP 与事件类别）；不同类别的 key 互不影响，
/// 安全扫描命中不会把同一 IP 的登录失败日志挤掉。
pub fn allow(key: String) -> bool {
    allow_at(key, Instant::now(), WINDOW, MAX_PER_WINDOW)
}

fn allow_at(key: String, now: Instant, window: Duration, max: u32) -> bool {
    let mut map = seen().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_KEYS {
        // 先清过期项；仍满说明窗口内 key 基数就超了上限，整表清空（丢一轮计数
        // 状态好过让内存无限涨）。下一批请求会从新窗口重新计数。
        map.retain(|_, w| now.saturating_duration_since(w.start) < window);
        if map.len() >= MAX_KEYS {
            map.clear();
        }
    }
    match map.get_mut(&key) {
        Some(w) if now.saturating_duration_since(w.start) < window => {
            if w.count >= max {
                return false;
            }
            w.count += 1;
            true
        }
        Some(w) => {
            *w = Window { start: now, count: 1 };
            true
        }
        None => {
            map.insert(key, Window { start: now, count: 1 });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_capped_within_one_window() {
        let key = format!("unit:{}", std::process::id());
        let t0 = Instant::now();
        for i in 0..MAX_PER_WINDOW {
            assert!(allow_at(key.clone(), t0, WINDOW, MAX_PER_WINDOW), "第 {i} 条应放行");
        }
        assert!(
            !allow_at(key.clone(), t0 + Duration::from_secs(1), WINDOW, MAX_PER_WINDOW),
            "额度用完后窗口内要丢弃"
        );
        assert!(
            allow_at(key, t0 + WINDOW, WINDOW, MAX_PER_WINDOW),
            "窗口滑过后重新计数"
        );
    }

    #[test]
    fn different_keys_are_independent() {
        let base = format!("unit-diff:{}", std::process::id());
        let t0 = Instant::now();
        for i in 0..MAX_PER_WINDOW {
            assert!(allow_at(format!("audit:{base}"), t0, WINDOW, MAX_PER_WINDOW), "{i}");
        }
        assert!(!allow_at(format!("audit:{base}"), t0, WINDOW, MAX_PER_WINDOW));
        // 同一来源的另一类事件有自己的额度
        assert!(allow_at(format!("scan:{base}"), t0, WINDOW, MAX_PER_WINDOW));
        assert!(allow_at(format!("login-fail:{base}"), t0, WINDOW, MAX_PER_WINDOW));
    }
}
