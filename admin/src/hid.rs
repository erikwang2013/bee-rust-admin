// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 对外 id 的编解码（hashids）：库里的主键是 i64（雪花），**出入 JSON 的 id 一律是短串**。
//!
//! 进程级 `OnceLock<Guard>`：serde 辅助函数（`ser_id` / `de_id` …）挂在字段上，
//! 拿不到 `AppState`，只能走全局。`init` 由 main 在启动时调一次，参数取自 `[app]`。
//!
//! 全局单例的代价：同一个进程里只能有一份盐——本项目只有一个 API 服务，
//! 没有第二个调用方；测试进程里第一次 `init` 生效（见本文件测试）。
//!
//! 无效输入一律 400「无效的 id」（`ApiError::BadRequest`），不是 500 也不是 panic：
//! `Guard::decode` 解不出东西时返回**空 Vec**（不是 Result），所以「空」或「多个值」
//! 都判无效；`enc(0)` 解回来仍是 `0`（无部门 / 根节点的哨兵值要走得了往返）。
use crate::error::ApiError;
use hashids::{Config, ConnectionConfig, Guard, HashidsManager};
use serde::{Deserialize, Deserializer, Serializer};
use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// 无效 id 的统一文案（400）。
pub const INVALID_ID: &str = "无效的 id";

/// hashids 单例。`init` 之前调用编解码 = 启动流程写错了，直接炸掉更快暴露。
static GUARD: OnceLock<Guard> = OnceLock::new();

fn guard() -> &'static Guard {
    GUARD.get().expect("hid::init 未调用（main 启动时必须先调）")
}

/// 启动时调一次：盐与最小长度来自 `[app] hashids_salt` / `hashids_min_len`。
pub fn init(salt: &str, min_len: usize) -> Result<(), String> {
    let config = Config::new().default_connection("main").connection(
        "main",
        ConnectionConfig::new().salt(salt).min_hash_length(min_len),
    );
    let guard = Guard::from_manager(Arc::new(HashidsManager::new(config)))
        .map_err(|e| format!("hashids 初始化失败: {e}"))?;
    GUARD.set(guard).map_err(|_| "hashids 已初始化".to_string())
}

/// i64 → 短串。
pub fn enc(id: i64) -> String {
    guard().encode(&[id as u64])
}

/// 一批 i64 → 一批短串（数组字段用：与逐个 `dec` 对称）。
pub fn enc_vec(ids: &[i64]) -> Vec<String> {
    ids.iter().map(|i| enc(*i)).collect()
}

/// 短串 → i64。解不出单个值（空 Vec / 多个值 / 乱码）→ 400。
pub fn dec(s: &str) -> Result<i64, ApiError> {
    let v = guard().decode(s.trim());
    if v.len() == 1 {
        return Ok(v[0] as i64);
    }
    Err(ApiError::BadRequest(INVALID_ID.into()))
}

// ── serde 辅助 ──────────────────────────────────────────────
//
// 挂在模型与 DTO 的 id 字段上：响应自动编、请求自动解。
// 输入除「短串」外**也认裸数字**（含 0 哨兵与 `null`）：无部门 / 根节点的 0
// 前端没法自己编码（它没有 hashids），编辑表单把列表里回显的值原样回传只是其中一条路径；
// 数字一并收下，避免「改个昵称因为 dept_id=0 而被 400」。输出永远只有短串一种形状。

/// 一个 JSON 值 → i64。短串走 `dec`，数字直接用（0 哨兵/迁移期客户端），null = 0。
fn parse_one(v: &Value) -> Result<i64, String> {
    match v {
        Value::String(s) => dec(s).map_err(|_| INVALID_ID.to_string()),
        Value::Number(n) => n.as_i64().ok_or_else(|| INVALID_ID.to_string()),
        Value::Null => Ok(0),
        _ => Err(INVALID_ID.to_string()),
    }
}

/// 空串 / 全空白（未选择的下拉、清空了的输入框）在**单值字段**里等价于 0。
fn is_blank(v: &Value) -> bool {
    matches!(v, Value::String(s) if s.trim().is_empty())
}

fn de_err<E: serde::de::Error>(msg: String) -> E {
    E::custom(msg)
}

/// `i64` 字段 → 短串。
pub fn ser_id<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&enc(*v))
}

/// 短串 → `i64` 字段（`null` / 空串 = 0：无部门、根节点）。
pub fn de_id<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v = Value::deserialize(d)?;
    if is_blank(&v) {
        return Ok(0);
    }
    parse_one(&v).map_err(de_err::<D::Error>)
}

/// null / 空串 / 缺省 → `None`（筛选项不生效）；其余同 [`de_id`]。
pub fn de_opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = Value::deserialize(d)?;
    if v.is_null() || is_blank(&v) {
        return Ok(None);
    }
    parse_one(&v).map(Some).map_err(de_err::<D::Error>)
}

/// 短串数组 → `Vec<i64>`。另认两种形状：单个 hashids 批量串（hashids 原生一次编多值的产物）、
/// 数字数组（迁移期客户端）；`null` = 空数组。
pub fn de_vec_id<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<i64>, D::Error> {
    let v = Value::deserialize(d)?;
    match v {
        Value::Array(a) => a.iter().map(parse_one).collect::<Result<Vec<i64>, String>>().map_err(de_err::<D::Error>),
        Value::Null => Ok(Vec::new()),
        Value::String(s) => {
            let nums = guard().decode(s.trim());
            if nums.is_empty() {
                return Err(de_err(INVALID_ID.to_string()));
            }
            Ok(nums.into_iter().map(|n| n as i64).collect())
        }
        _ => Err(de_err(INVALID_ID.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OnceLock 只能初始化一次：第一个跑到这里的测试定下盐，其余复用。
    fn setup() {
        let _ = init("hid-unit-test-salt", 8);
    }

    #[test]
    fn roundtrip_and_shape() {
        setup();
        let id = 508_047_278_033_704_960i64; // 雪花量级
        let s = enc(id);
        assert!(s.len() >= 8, "min_len=8 至少要 8 位: {s}");
        assert_eq!(dec(&s).unwrap(), id);

        // 0 哨兵（无部门 / 根节点）也要走得通
        assert_eq!(dec(&enc(0)).unwrap(), 0);
        // 批量串解回的是整批，不是单个 id：`dec`（单个 id 的入口）必须拒
        //（数组字段走 de_vec_id，那才收批量串）
        assert!(dec(&guard().encode(&[id as u64, 1, 2])).is_err(), "批量串不该被当成单个 id");
        assert_eq!(enc_vec(&[1, 2]).len(), 2);
    }

    #[test]
    fn invalid_input_is_400_not_panic() {
        setup();
        for bad in ["", "   ", "!!!", "a", "не-utf8"] {
            let e = dec(bad).unwrap_err();
            let ApiError::BadRequest(msg) = e else { panic!("应是 400") };
            assert_eq!(msg, INVALID_ID);
        }
        // 空 Vec 是「解码失败」的形状，与「解出 0」不是一回事
        assert!(guard().decode("").is_empty());
        assert_eq!(guard().decode(&enc(0)), vec![0u64]);
    }

    #[test]
    fn serde_helpers_roundtrip_the_id_shapes() {
        setup();
        let id = 123_456_789i64;

        #[derive(serde::Serialize, serde::Deserialize)]
        struct One {
            #[serde(serialize_with = "ser_id", deserialize_with = "de_id")]
            id: i64,
        }
        let v = serde_json::to_value(One { id }).unwrap();
        assert!(v["id"].is_string(), "id 必须是字符串: {v}");
        assert_eq!(serde_json::from_value::<One>(v).unwrap().id, id);
        // 数字（0 哨兵 / 迁移期客户端）也认
        assert_eq!(serde_json::from_value::<One>(serde_json::json!({"id": 0})).unwrap().id, 0);
        assert_eq!(serde_json::from_value::<One>(serde_json::json!({"id": null})).unwrap().id, 0);
        assert_eq!(serde_json::from_value::<One>(serde_json::json!({"id": ""})).unwrap().id, 0);
        assert!(serde_json::from_value::<One>(serde_json::json!({"id": "!!!"})).is_err());

        #[derive(serde::Deserialize)]
        struct Many {
            #[serde(deserialize_with = "de_vec_id", default)]
            ids: Vec<i64>,
        }
        // 响应用 `enc_vec` 直接拼短串数组（见 `api/role.rs`），入站要认这个形状
        assert_eq!(
            serde_json::from_value::<Many>(serde_json::json!({"ids": enc_vec(&[id, 1])}))
                .unwrap()
                .ids,
            vec![id, 1],
            "短串数组往返"
        );
        assert_eq!(
            serde_json::from_value::<Many>(serde_json::json!({"ids": [1, 2]})).unwrap().ids,
            vec![1, 2],
            "数字数组也认"
        );
        assert_eq!(
            serde_json::from_value::<Many>(serde_json::json!({"ids": guard().encode(&[id as u64, 1])}))
                .unwrap()
                .ids,
            vec![id, 1],
            "单个批量串也认"
        );

        #[derive(serde::Deserialize)]
        struct Opt {
            #[serde(deserialize_with = "de_opt_id", default)]
            dept_id: Option<i64>,
        }
        assert_eq!(serde_json::from_value::<Opt>(serde_json::json!({"dept_id": enc(id)})).unwrap().dept_id, Some(id));
        assert_eq!(serde_json::from_value::<Opt>(serde_json::json!({"dept_id": null})).unwrap().dept_id, None);
        assert_eq!(serde_json::from_value::<Opt>(serde_json::json!({"dept_id": ""})).unwrap().dept_id, None);
    }
}
