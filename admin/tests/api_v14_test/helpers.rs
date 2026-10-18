// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! v1.4 集成测试的本地构造（F3 拆分：主文件只留用例）。

use crate::common;
use reqwest::Method;
use serde_json::{Value, json};

/// 统一请求：返回 (状态码, JSON body)；断言消息带上 body，失败好定位。
pub(super) async fn call(
    c: &reqwest::Client,
    method: Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut rb = c.request(method, url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let r = rb.send().await.expect("请求发送失败");
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// 取 CSV 响应体（含 BOM），校验附件头，返回去掉 BOM 的文本行。
pub(super) async fn csv_lines(c: &reqwest::Client, url: String, token: &str) -> Vec<String> {
    let r = c.get(url).bearer_auth(token).send().await.expect("导出请求失败");
    assert_eq!(r.status(), 200, "导出必须 200");
    assert_eq!(r.headers()["content-type"], "text/csv; charset=utf-8", "导出 Content-Type");
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("attachment") && cd.contains(".csv"), "附件头: {cd}");
    let body = r.bytes().await.unwrap().to_vec();
    assert_eq!(&body[..3], &[0xEF, 0xBB, 0xBF], "首字节必须是 UTF-8 BOM");
    String::from_utf8(body[3..].to_vec())
        .expect("BOM 之后是 UTF-8")
        .trim_end()
        .split("\r\n")
        .map(str::to_string)
        .collect()
}

/// 菜单树里按权限码取菜单 id（B5 造角色要勾菜单）；对外 id 是 hashid 串，原样透传。
pub(super) fn menu_id(menus: &Value, perm: &str) -> Option<String> {
    for m in menus.as_array()? {
        if m["perm"].as_str() == Some(perm) {
            return m["id"].as_str().map(str::to_string);
        }
        if let Some(id) = menu_id(&m["children"], perm) {
            return Some(id);
        }
    }
    None
}

/// 建角色 + 勾菜单，返回角色 id（B5 造场景用）。
pub(super) async fn make_role(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    scope: i8,
    status: i8,
    menu_ids: &[String],
) -> String {
    let (st, v) = call(
        c,
        Method::POST,
        format!("{base}/api/v1/roles"),
        Some(token),
        Some(json!({"name": name, "code": name, "data_scope": scope, "status": status})),
    )
    .await;
    assert_eq!(st, 200, "建角色 {name}: {v}");
    let id = common::as_id(&v["data"]["id"]);
    let (st, v) = call(
        c,
        Method::PUT,
        format!("{base}/api/v1/roles/{id}/menus"),
        Some(token),
        Some(json!({"menu_ids": menu_ids})),
    )
    .await;
    assert_eq!(st, 200, "勾菜单 {name}: {v}");
    id
}
