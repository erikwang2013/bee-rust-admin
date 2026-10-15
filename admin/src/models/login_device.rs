// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 登录设备（项 7）：一条 = 一个管理员的一台设备，token 里带它的 id（`Claims::dv`），
/// 踢一台就把这一行置 0 —— 下一次请求 401，与 `token_version` 的**全局**失效是两回事。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "admin_login_device")]
pub struct AdminLoginDevice {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub admin_id: i64,
    /// 设备指纹就是 UA 原串（登录那次的，截到 255）：同账号下同 UA 视为同一台设备。
    /// 不引入指纹库 —— 用户换浏览器/升级版本本来就会变成「一台新设备」，正好借机
    /// 让他看见；IP 不进指纹（手机换基站就会跳 IP，那样列表会刷成一片）。
    #[bee(sql_type = "VARCHAR(255)")]
    pub user_agent: String,
    /// 最近一次登录/活跃时的 IP（只做展示）
    #[bee(sql_type = "VARCHAR(45)")]
    pub ip: String,
    /// 1 在用 / 0 已踢。**软删**：踢出后重新登录会新建一行，历史留痕
    pub status: i8,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    /// 最后活跃：不是每个请求都写，超过间隔才刷新一次（见 `auth::touch_device`）
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub last_active_at: NaiveDateTime,
}
