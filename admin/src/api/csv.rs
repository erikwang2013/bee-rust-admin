// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! CSV 导出（B6）：BOM + 公式注入防护 + 附件响应（流式）。
use crate::error::ApiError;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio_stream::wrappers::ReceiverStream;

/// 今天（本地时区）`YYYYMMDD`，拼文件名用。
pub fn today() -> String {
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// 单元格转义：
/// - `= + - @` 开头前置 `'`——Excel 会把这类单元格当公式执行（CSV 注入）；
/// - 含逗号/引号/换行时按 RFC4180 用双引号包裹、内部引号翻倍。
pub fn cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    if s.starts_with(['=', '+', '-', '@']) {
        out.push('\'');
    }
    out.push_str(s);
    if out.contains([',', '"', '\n', '\r']) {
        return format!("\"{}\"", out.replace('"', "\"\""));
    }
    out
}

/// 一行（含 CRLF，Excel 最兼容）。
pub fn row(cells: &[String]) -> String {
    let mut line = cells.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",");
    line.push_str("\r\n");
    line
}

/// JSON 值 → 单元格文本：字符串原样，数组用 `|` 连接，数字/布尔/null 文本化。
pub fn jcell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(jcell).collect::<Vec<_>>().join("|"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// UTF-8 BOM：Excel 直接打开不乱码。
pub const BOM: &str = "\u{feff}";

/// 附件响应头：`text/csv; charset=utf-8` + `Content-Disposition`。
fn headers(filename: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    headers
}

/// 流式导出的批大小（B6）：keyset 分页每批行数。内存占用只与它成正比，与总量无关。
pub const BATCH: usize = 5000;

/// 流式导出：`next(last_id)` 取下一批 `(id, 已渲染行)`，批空即结束
/// （调用方按 `id < last_id ORDER BY id DESC LIMIT BATCH` 取）。
///
/// 首批在返回响应前同步取：首批就失败时还能回统一错误信封，而不是 200 之后断流。
/// 之后每批渲染成一块 `Bytes` 交给 `Body::from_stream` 边产边发，BOM 在首块里。
pub async fn streamed<F, Fut>(
    filename: String,
    header_cells: Vec<String>,
    mut next: F,
) -> Result<Response, ApiError>
where
    F: FnMut(Option<i64>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Vec<(i64, String)>, ApiError>> + Send + 'static,
{
    let first = next(None).await?;
    // 通道小一点：生产端本来就是一整批一整批地送，backpressure 让它别跑太远
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);

    tokio::spawn(async move {
        let mut chunk = String::from(BOM);
        chunk.push_str(&row(&header_cells));
        let mut rows = first;
        let mut last = None;
        loop {
            let full = rows.len() == BATCH;
            for (id, line) in rows.drain(..) {
                last = Some(id);
                chunk.push_str(&line);
            }
            // 发送失败 = 客户端断开：收手，别再继续查库
            if tx.send(Ok(Bytes::from(std::mem::take(&mut chunk)))).await.is_err() {
                return;
            }
            if !full {
                return;
            }
            match next(last).await {
                Ok(batch) if !batch.is_empty() => rows = batch,
                Ok(_) => return,
                Err(e) => {
                    // 状态码早已发出、改不了：打日志并中断 body，让客户端看到
                    // 「传输失败」，而不是一份静默截断的 CSV 被当成完整导出
                    tracing::error!("导出中断: {e:?}");
                    let _ = tx.send(Err(std::io::Error::other("export aborted"))).await;
                    return;
                }
            }
        }
    });

    Ok((headers(&filename), Body::from_stream(ReceiverStream::new(rx))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_escapes_formula_starters() {
        assert_eq!(cell("=1+1"), "'=1+1");
        assert_eq!(cell("+86"), "'+86");
        assert_eq!(cell("-2"), "'-2");
        assert_eq!(cell("@x"), "'@x");
        // 公式起始符只在前缀位置才危险
        assert_eq!(cell("a=b"), "a=b");
        assert_eq!(cell(""), "");
    }

    #[test]
    fn cell_quotes_separators_and_quotes() {
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("a\"b"), "\"a\"\"b\"");
        assert_eq!(cell("a\nb"), "\"a\nb\"");
        // 前置 `'` 与引号包裹同时发生时，`'` 在引号内
        assert_eq!(cell("=a,b"), "\"'=a,b\"");
    }

    #[test]
    fn row_joins_and_terminates() {
        assert_eq!(row(&["a".into(), "b".into()]), "a,b\r\n");
    }

    #[test]
    fn jcell_flattens_arrays() {
        assert_eq!(jcell(&serde_json::json!(["a", "b"])), "a|b");
        assert_eq!(jcell(&serde_json::json!(7)), "7");
        assert_eq!(jcell(&serde_json::json!(null)), "");
    }
}
