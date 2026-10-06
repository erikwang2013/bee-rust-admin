// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! CSV 导出（B6）：BOM + 公式注入防护 + 附件响应。
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

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

/// 附件响应：UTF-8 BOM 开头（Excel 直接打开不乱码）+ `text/csv; charset=utf-8`。
pub fn response(filename: String, body: String) -> Response {
    let mut bytes = Vec::with_capacity(body.len() + 3);
    bytes.extend_from_slice("\u{feff}".as_bytes());
    bytes.extend_from_slice(body.as_bytes());

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    (headers, bytes).into_response()
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
