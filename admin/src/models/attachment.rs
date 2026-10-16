// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 附件（项 5）：分片上传的落盘文件 + 多态关联（`entity_type` / `entity_id`）。
///
/// 多态关联的**唯一**语义在 `api::attach` 里：绑定了**已发布**公告的附件 = 公开
/// （公告本身就对所有登录用户可见，配图必须跟着可见），其余一律按上传人的数据权限卡。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "attachment")]
pub struct Attachment {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
    /// 原始文件名（列表展示 + 下载时的文件名）
    #[bee(sql_type = "VARCHAR(255)")]
    pub name: String,
    #[bee(sql_type = "VARCHAR(16)")]
    pub ext: String,
    #[bee(sql_type = "VARCHAR(128)")]
    pub mime: String,
    /// 落盘字节数（末片组装后实读，不信客户端报的值）
    pub size: i64,
    /// 服务端算的十六进制 sha256（查重/校验用）
    #[bee(sql_type = "CHAR(64)")]
    pub sha256: String,
    /// 插件的 `saved_path`（`attach_<月目录>_<md5>.<ext>`）——内部地址，
    /// `skip` 是**故意**的：接口层自己拼公开字段，这个值不该有被顺手序列化出去的机会。
    #[serde(skip)]
    #[bee(sql_type = "VARCHAR(255)")]
    pub stored_path: String,
    /// 上传人。列名与日志类同款（`admin_id`），这样数据权限直接复用
    /// `datascope::apply_admin_scope` / `ensure_admin_in_scope`，不用新写一套口径。
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub admin_id: i64,
    /// 多态关联的类型：空串 = 未绑定；目前只认 `notice`（公告配图）
    #[bee(sql_type = "VARCHAR(32)")]
    pub entity_type: String,
    /// 关联对象 id（0 = 未绑定）
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub entity_id: i64,
    /// 同一关联对象下的排序（配图顺序）
    pub sort: i32,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
