#!/usr/bin/env bash
# 模块脚手架的模板（由 scripts/scaffold-module.sh source 进来用，别单独跑）。
#
# 每段都是**不带引号**的 heredoc：`$` 与反引号会被 bash 展开 —— 生成物里要写字面量
# 的话得转义（TS 的模板串写成 \` 与 \${id}）。段里的占位符就是脚手架里的那些变量。

tpl_model() { cat <<EOF
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
// 还没接进 main.rs / seed.rs（见 docs/scaffold.md 的接线清单①③）之前，本模块没人引用，
// 编译期会刷 dead_code 警告；接完线就把下面这行删掉。
#![allow(dead_code)]
use bee_orm::Model;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// $LABEL（脚手架生成，字段按需增删；改完要跑一遍迁移，见 docs/scaffold.md）。
/// 建表/补列由 \`seed::migrate\` 里的 \`migrate::sync::<$CAMEL, _>\` 负责。
#[derive(Model, Serialize, Deserialize, Clone, Debug)]
#[bee(table = "$MODULE")]
pub struct $CAMEL {
    #[bee(pk)]
    #[serde(serialize_with = "crate::hid::ser_id", deserialize_with = "crate::hid::de_id")]
    pub id: i64,
$MODEL_FIELDS    #[serde(serialize_with = "crate::util::ser_dt")]
    pub created_at: NaiveDateTime,
    #[serde(serialize_with = "crate::util::ser_dt")]
    pub updated_at: NaiveDateTime,
}
EOF
}

tpl_api() { cat <<EOF
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! $LABEL（脚手架生成）：列表 / 新建 / 编辑 / 删除。
//! 权限码 \`$PERM:list|add|edit|remove\` 由 seed.rs 的菜单种子发出来（片段见 docs/scaffold.md）；
//! 没粘种子之前只有超管能调（超管短路），普通角色一律 403。
// 还没挂进 main.rs（接线清单③）之前没人引用这几个 handler，会刷 dead_code；挂完删掉这行。
#![allow(dead_code)]
use crate::api::{PagingExt, check_len, page_size};
use crate::auth::Auth;
use crate::error::{ApiError, AppJson, AppPath, AppQuery, ok};
use crate::hid;
use crate::models::$CAMEL;
use crate::state::AppState;
use crate::util::now;
use axum::Json;
use axum::extract::State;
use bee_orm::Model;
use serde::Deserialize;
use serde_json::{Value, json};

// 文本列逐个卡长度（常量 + 校验函数）：超长要在这里回 400，别让 MySQL 报 1406 变成 500。
$CHECKS
fn check_${NAME_FIELD}(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest("$NAME_ZH不能为空".into()));
    }
    check_len("$NAME_FIELD", value.trim(), ${NAME_FIELD^^}_MAX)
}

#[derive(Deserialize)]
pub struct ${CAMEL}Query {
    pub page: Option<u32>,
    pub size: Option<u32>,
$QUERY_FIELDS}

#[derive(Deserialize)]
pub struct ${CAMEL}Create {
$BODY_FIELDS}

/// 与 Create 一样是全量字段（PUT 是整体替换，不做部分更新）
#[derive(Deserialize)]
pub struct ${CAMEL}Update {
$BODY_FIELDS}

#[apidoc::title("$LABEL列表")]
#[apidoc::desc("分页列表")]
#[apidoc::url("/api/v1$API_PATH")]
#[apidoc::method("GET")]
#[apidoc::tag("$LABEL")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::query(name = "page", ty = "int", desc = "页码，从 1 开始，默认 1")]
#[apidoc::query(name = "size", ty = "int", desc = "每页条数，默认 10，上限 100")]
$ADOC_QUERIES#[apidoc::response_status("200")]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    AppQuery(q): AppQuery<${CAMEL}Query>,
) -> Result<Json<Value>, ApiError> {
    auth.require("$PERM:list")?;
    let (page, size) = page_size(q.page, q.size);
    let mut qs = ${CAMEL}::query();
$QUERY_FILTERS    let (rows, total) = qs
        .order_by("id DESC")
        .fetch_page(&state.db, page, size)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(json!({ "list": rows, "total": total })))
}

#[apidoc::title("新建$LABEL")]
#[apidoc::desc("$NAME_ZH必填且有长度上限，超了回 400")]
#[apidoc::url("/api/v1$API_PATH")]
#[apidoc::method("POST")]
#[apidoc::tag("$LABEL")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
$ADOC_PARAMS#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    AppJson(body): AppJson<${CAMEL}Create>,
) -> Result<Json<Value>, ApiError> {
    auth.require("$PERM:add")?;
    check_${NAME_FIELD}(&body.$NAME_FIELD)?;
$CHECK_CALLS
    let row = $CAMEL {
        id: state.next_id()?,
$CREATE_ASSIGN        created_at: now(),
        updated_at: now(),
    };
    // id 已在上面发号，insert 就够（create 的读回是给自增主键用的）
    row.insert(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "id": hid::enc(row.id) })))
}

#[apidoc::title("编辑$LABEL")]
#[apidoc::desc("整体替换；id 不存在回 404")]
#[apidoc::url("/api/v1$API_PATH/{id}")]
#[apidoc::method("PUT")]
#[apidoc::tag("$LABEL")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "「$LABEL」的对外 id（hashids 短串）")]
$ADOC_PARAMS#[apidoc::response_status("200")]
#[apidoc::response_status("400")]
#[apidoc::response_status("404")]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
    AppJson(body): AppJson<${CAMEL}Update>,
) -> Result<Json<Value>, ApiError> {
    auth.require("$PERM:edit")?;
    let id = hid::dec(&id)?;
    check_${NAME_FIELD}(&body.$NAME_FIELD)?;
$CHECK_CALLS
    let mut row = ${CAMEL}::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
$UPDATE_ASSIGN    row.updated_at = now();
    row.update(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(Value::Null))
}

#[apidoc::title("删除$LABEL")]
#[apidoc::desc("删之前先查一次，不存在回 404")]
#[apidoc::url("/api/v1$API_PATH/{id}")]
#[apidoc::method("DELETE")]
#[apidoc::tag("$LABEL")]
#[apidoc::header(name = "Authorization", desc = "Bearer <token>")]
#[apidoc::route_param(name = "id", ty = "string", required, desc = "「$LABEL」的对外 id（hashids 短串）")]
#[apidoc::response_status("200")]
#[apidoc::response_status("404")]
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    AppPath(id): AppPath<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("$PERM:remove")?;
    let id = hid::dec(&id)?;
    let row = ${CAMEL}::query()
        .filter_eq("id", id)
        .map_err(ApiError::from)?
        .one(&state.db)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    row.delete(&state.db).await.map_err(ApiError::from)?;
    Ok(ok(json!({ "deleted": 1 })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_${NAME_FIELD}_rejects_blank_and_too_long() {
        assert!(check_${NAME_FIELD}("").is_err(), "空串要拒");
        assert!(check_${NAME_FIELD}("   ").is_err(), "只有空白也要拒");
        assert!(check_${NAME_FIELD}(&"x".repeat(${NAME_FIELD^^}_MAX)).is_ok());
        assert!(
            check_${NAME_FIELD}(&"x".repeat(${NAME_FIELD^^}_MAX + 1)).is_err(),
            "超长要在入口拦下，别让 MySQL 报 1406 变 500"
        );
        assert!(check_${NAME_FIELD}(" ok ").is_ok(), "两侧空白要 trim 后放行");
    }
$CHECK_TESTS}
EOF
}

tpl_test() { cat <<EOF
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! $LABEL（脚手架生成）：CRUD 走一遍真库真进程。
//!
//! 跑之前两件事（见 docs/scaffold.md）：① 表名加进 tests/common/mod.rs 的 reset_db；
//! ② 路由挂进 main.rs。没有 BEE_ADMIN_DB_DSN 时整体跳过（集成测试的老规矩：不假绿）。
mod common;

use reqwest::Method;
use serde_json::{Value, json};

async fn call(
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

#[tokio::test]
async fn ${MODULE}_crud() {
    let Some(dsn) = common::dsn() else {
        eprintln!("跳过：没有 BEE_ADMIN_DB_DSN");
        return;
    };
    common::reset_db(&dsn).await;
    let (_server, base) = common::start_server().await;
    let token = common::login(&base, "admin", "admin123").await.expect("admin 登录失败");
    let c = reqwest::Client::new();
    let api = |p: &str| format!("{base}/api/v1$API_PATH{p}");
    // 名字带唯一后缀：本表可能不在 reset_db 清单里（那就是残留的行），别跟上次撞名
    let unique = format!("$MODULE-{}", common::new_row_id());

    // 未登录一律 401（鉴权闸门）
    let (st, _) = call(&c, Method::GET, api(""), None, None).await;
    assert_eq!(st, 401, "没有 token 必须 401");

    // 新建
    let body = json!({$DEFAULTS });
    let (st, v) = call(&c, Method::POST, api(""), Some(&token), Some(body.clone())).await;
    assert_eq!(st, 200, "新建：{v}");
    let id = v["data"]["id"].as_str().expect("新建要回 id").to_string();

    // 名称必填：空白名回 400（校验在进入库之前）
    let mut blank = body.clone();
    blank["$NAME_FIELD"] = json!("   ");
    let (st, v) = call(&c, Method::POST, api(""), Some(&token), Some(blank)).await;
    assert_eq!(st, 400, "空白 $NAME_FIELD 要 400：{v}");

    // 列表：按名字模糊查能查到刚建的那行（查询走服务端 filter_contains）
    let (st, v) = call(
        &c,
        Method::GET,
        format!("{}?$NAME_FIELD={unique}", api("")),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(st, 200, "列表：{v}");
    assert_eq!(v["data"]["total"], json!(1), "刚建的行要能查到：{v}");

    // 编辑：改名后旧名字查不到、新名字能查到
    let mut edit = body;
    edit["$NAME_FIELD"] = json!(format!("{unique}-改"));
    let (st, v) = call(&c, Method::PUT, api(&format!("/{id}")), Some(&token), Some(edit)).await;
    assert_eq!(st, 200, "编辑：{v}");
    let (_, v) = call(
        &c,
        Method::GET,
        format!("{}?$NAME_FIELD={unique}-改", api("")),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(v["data"]["total"], json!(1), "改完要能按新名字查到：{v}");

    // 删除：删掉后查不到，再删一次回 404（幂等性归调用方，接口本身如实报 404）
    let (st, v) = call(&c, Method::DELETE, api(&format!("/{id}")), Some(&token), None).await;
    assert_eq!(st, 200, "删除：{v}");
    let (st, _) = call(&c, Method::DELETE, api(&format!("/{id}")), Some(&token), None).await;
    assert_eq!(st, 404, "删过的 id 再删要 404");
    let (_, v) = call(
        &c,
        Method::GET,
        format!("{}?$NAME_FIELD={unique}", api("")),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(v["data"]["total"], json!(0), "删完不该还能查到：{v}");
}
EOF
}

tpl_web_api() { cat <<EOF
import { http } from './client';
import type { Page } from './types';

/** $LABEL（脚手架生成）。类型就放在模块自己的文件里，不去改共享的 types.ts。 */
export interface ${CAMEL}Row {
  id: string;
$TS_ROW  created_at: string;
  updated_at: string;
}

/** 新建 / 编辑的请求体（对应后端 ${CAMEL}Create / ${CAMEL}Update） */
export interface ${CAMEL}Form {
$TS_FORM}

export interface ${CAMEL}Query { page?: number; size?: number;$TS_QUERY }

export const $API_OBJ = {
  list: (q: ${CAMEL}Query) => http.get<Page<${CAMEL}Row>>('$API_PATH', q),
  create: (data: ${CAMEL}Form) => http.post<{ id: string }>('$API_PATH', data),
  update: (id: string, data: ${CAMEL}Form) => http.put<null>(\`$API_PATH/\${id}\`, data),
  remove: (id: string) => http.del<null>(\`$API_PATH/\${id}\`),
};
EOF
}

tpl_web_page() { cat <<EOF
import { useCallback, useEffect, useState } from 'react';
import { $ANTD_IMPORTS } from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { $API_OBJ, type ${CAMEL}Form, type ${CAMEL}Query, type ${CAMEL}Row } from '../../../api/$MODULE';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';

/** $LABEL（脚手架生成）：列表 + 新建/编辑弹窗 + 删除。
 *  字段标签用的是现有的通用键（脚手架只引已经存在的 i18n 键，否则 tsc 立刻报错）；
 *  想要模块专属文案，就把脚本打印的 i18n 片段粘进两张词表，再把这些 t('field.x') 换掉。 */
export default function ${CAMEL}Page() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [form] = Form.useForm<${CAMEL}Form>();
  const [query, setQuery] = useState<${CAMEL}Query>({ page: 1, size: 10 });
  const [rows, setRows] = useState<${CAMEL}Row[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<${CAMEL}Row | null>(null);
  const [modal, setModal] = useState(false);

$YESNO  const load = useCallback(async (q: ${CAMEL}Query) => {
    setLoading(true);
    try {
      const res = await $API_OBJ.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    setModal(true);
  };

  const openEdit = (row: ${CAMEL}Row) => {
    setEditing(row);
    form.setFieldsValue(row);
    setModal(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    if (editing) await $API_OBJ.update(editing.id, v);
    else await $API_OBJ.create(v);
    message.success(editing ? t('common.updated') : t('common.created'));
    setModal(false);
    void load(query);
  };

  const remove = async (row: ${CAMEL}Row) => {
    await $API_OBJ.remove(row.id);
    message.success(t('common.deleted'));
    void load(query);
  };

  const columns: ColumnsType<${CAMEL}Row> = [
    { title: t('field.id'), dataIndex: 'id', width: 140 },
$COLUMNS    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    {
      title: t('common.actions'), width: 140, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="$PERM:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="$PERM:remove">
            <Popconfirm
              title={t('common.confirm')}
              okText={t('common.delete')}
              okButtonProps={{ danger: true }}
              onConfirm={() => void remove(row)}
            >
              <Button size="small" type="link" danger>{t('common.delete')}</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder={$([ -n "$(known_zh "$NAME_FIELD")" ] && echo "t('field.$NAME_FIELD')" || echo "'$NAME_FIELD'")}
          allowClear
          style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, $NAME_FIELD: v || undefined, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Auth code="$PERM:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('common.add')}</Button>
        </Auth>
      </Space>

      <Table<${CAMEL}Row>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 900 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />

      <Modal
        title={editing ? t('common.edit') : t('common.add')}
        open={modal}
        onCancel={() => setModal(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
$FORM_ITEMS        </Form>
      </Modal>
    </>
  );
}
EOF
}
