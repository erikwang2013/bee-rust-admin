# apidoc 注解补齐 · menu 与 dept（请求体 + 响应结构）实现计划

> 上位文档：[v2.6 之后的路线图](2026-10-11-post-v2.6-roadmap.md)（本文对应其中 **D1**，建议排期见该文 §3）。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

日期：2026-10-11 · 基线：`main @ e2b84f3` · 性质：**只加注解，不改任何运行时行为**（低风险）

**Goal:** 给 `admin/src/api/menu.rs` 与 `admin/src/api/dept.rs` 的 8 个 handler 补 `param`（请求体字段）与 `returned`（响应结构）。补完后 `/apidoc/api.json` 里这两个模块的 POST/PUT **请求体不再是空数组**，字段名 / 类型 / 必填与代码里的 DTO 逐字一致。

**Architecture:** apidoc 是**属性宏**：`emit_many()` 把原函数原样重新输出，再追加若干 `static … DocFragmentEntry`（`linkme` 分布式切片注册）。文档在**启动时物化**成一份 `api.json`，所以注解只影响文档数据 —— 函数体、DTO、SQL、权限码一行不动。

**为什么低风险（已核实）:** 本次改的每一个字都在 `#[apidoc::*]` 属性行里；宏不触碰函数体（证据：`/home/wwwroot/bag/apidoc-rust/crates/apidoc-macros/src/lib.rs` 的 `emit_many`）。`[apidoc] enabled` 仍是默认 false（生产 `admin/conf/app.conf` 里**没有 `[apidoc]` 节**，核实过），因此对 127.0.0.1:8080 的线上实例零影响。

**Tech Stack:** axum 0.8 + `apidoc-rust 1.6.2`（lib 名 `apidoc`）+ linkme 0.3；验收用仓库既有的真库集成测试脚手架（`admin/tests/common/mod.rs`）。

---

## 背景（已核实，直接采信）

`docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md` 的「阶段 5b 契约」写的是：
*「核心模块额外加深（auth / admin / role）… 其余模块照抄这套写法即可后续补」*。实际铺了 9 个模块，
**menu 与 dept 一个都没铺**：

| 模块 | `param`/`query`/`returned` 注解数 |
|---|---|
| `admin/src/api/dict.rs` | 29 |
| `admin/src/api/admin.rs` | 37 |
| `admin/src/api/role.rs` | 27 |
| `admin/src/api/notice.rs` | 12 |
| `admin/src/api/login_log.rs` / `audit_log.rs` | 各 18 |
| `admin/src/api/auth.rs` | 17 |
| `admin/src/api/job.rs` | 7 |
| `admin/src/api/avatar.rs` | 9 |
| **`admin/src/api/menu.rs`** | **0** |
| **`admin/src/api/dept.rs`** | **0** |

后果：这两个模块的 POST/PUT **请求体在 `/apidoc/api.json` 里是 `params: []`**，对接方看不到要传什么。

## 事实基础：8 个 handler（全部读自代码，不是猜的）

| 文件 | handler | 方法 + 路径 | 请求体 DTO | 响应 `data` |
|---|---|---|---|---|
| menu.rs | `tree` | GET `/api/v1/menus/tree` | — | `Menu` 全字段树（含按钮节点）+ `children` |
| menu.rs | `create` | POST `/api/v1/menus` | `MenuBody` | `{ id }` |
| menu.rs | `update` | PUT `/api/v1/menus/{id}` | `MenuBody`（**与 create 同一个结构体**） | `null` |
| menu.rs | `remove` | DELETE `/api/v1/menus/{id}` | — | `null` |
| dept.rs | `tree` | GET `/api/v1/depts/tree` | — | `Dept` 全字段树 + `children` |
| dept.rs | `create` | POST `/api/v1/depts` | `DeptBody` | `{ id }` |
| dept.rs | `update` | PUT `/api/v1/depts/{id}` | `DeptBody`（同上） | `null` |
| dept.rs | `remove` | DELETE `/api/v1/depts/{id}` | — | `null` |

两个模块**都没有查询参数**，所以本批次只涉及 `param` 与 `returned`（外加一条纠错，见 Task 2 Step 4）。

**`MenuBody`（`admin/src/api/menu.rs:17-38`，JSON 名 = 注解名）**
`parent_id`(string，缺省/空/null = 0)、`name`(string，必填)、`type`(string，必填 —— 源码是
`menu_type` 带 `#[serde(rename = "type")]`)、`perm`、`path`、`component`、`icon`(string)、
`sort`(int)、`visible`(int，默认 1)、`status`(int，默认 1)。
长度上限出自 `validate_body()`：name 64 / perm 128 / path 128 / component 128 / icon 64（`check_len` 按**字符**数）。

**`DeptBody`（`admin/src/api/dept.rs:16-29`）**
`parent_id`(string，同上)、`name`(string，必填)、`sort`(int)、`leader`(string)、`phone`(string)、`status`(int，默认 1)。
长度上限：name 64 / leader 64 / phone 20。

**`parent_id` 的入参形状**（读自 `admin/src/hid.rs` 的 `de_id` / `parse_one`）：hashids 短串，
**也认裸数字**（含 0 哨兵）、`null`、空串 → 0。说明文案要写全，别只写「短串」。

**树形结构**（`build()`，两个文件同构）：每个节点是模型的全字段序列化 + 一个**无条件存在**的
`children`（叶子是 `[]`）。`auth.rs:523` 的 `/auth/menus` 是仓库里既有的树形注解样板，
本次照抄它的写法（`children` 写「同结构，递归」，不再手工展开第二层）。

## 注解写法约定（照抄现状，不重新设计）

顺序：`title` → `desc` → `url` → `method` → `tag` → `header` → `route_param` → `query` → `param…` → `response_status…` → `returned`
（样板见 `admin/src/api/admin.rs:396-413`、`admin/src/api/notice.rs:106-116`）。

插件能力边界（读自 `apidoc-macros/src/args.rs` 的 `ParamArgs`，**别用不存在的关键字**）：
`name` / `ty`（别名 `type`）/ `required`（**裸标志，不写 `= true`**）/ `default` / `desc` / `mock` / `children`。
`children = [{ … }, { … }]` —— 中括号里每个孩子用**花括号**包着，逗号分隔，可以任意层嵌套。
没写 `ty` 时默认 `"string"`。`DocParam` 序列化后字段名是 `name` / `type` / `required` / `desc` / `children`。

## 本机环境坑（都踩过，先看这段）

1. **不许碰 127.0.0.1:8080 的生产实例**（现在真的在跑：`bee_admin` pid 2800279）。验收一律用**另起的一次性实例 + 别的端口（18099）**。
2. **本机 `diff` 是坏的**（文件不同也返回 0）。要对比文件用 `cmp` 或 `git diff`。
3. **`cargo` 别放后台跑**：本机内存监控会把它杀掉。前台跑 + 大 timeout（`timeout 900 cargo …`）。
4. **集成测试必须设 `BEE_ADMIN_DB_DSN`**，否则 `common::dsn()` 直接 `return`，测试 **0.00 秒「通过」是假的**。跑完务必看一眼耗时和有没有「跳过」字样的输出。
5. 测试进程之间**串行**跑（每个 `tests/*.rs` 一个二进制）；**同一个文件里不要写第二个 `#[tokio::test]`**（同文件内的测试并发跑，会抢同一个库）。

## 不做

- 不重新设计注解风格（就照现有 9 个模块的写法）
- 不动那 9 个模块（它们是好的）
- 不改任何接口实现、DTO、SQL、鉴权码
- 不用 `md` / `table` / `ref` / `codegen` 这些进阶能力（YAGNI）

---

### Task 1: 先写验收测试，跑出「红」的基线

**Files:**
- Create: `admin/tests/api_apidoc_test.rs`

- [ ] **Step 1: 写 `admin/tests/api_apidoc_test.rs`（整文件照抄）**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! apidoc 注解契约：menu / dept 的请求体字段必须与 DTO 逐字一致。
//!
//! 这两个模块在 v1.5「阶段 5b」漏了 param/returned（其余 9 个模块都补了），
//! 表现在 `/apidoc/api.json` 里它们的 POST/PUT **请求体是空数组** —— 对接方看不到要传什么。
//! 本文件把「文档里的字段」钉在代码上：注解漏一个字段 / 写错名字就红。
//!
//! 起真实进程（把 `[apidoc] enabled = true` 临时打进配置）+ 真库，需要 `BEE_ADMIN_DB_DSN`。
mod common;

use serde_json::{Value, json};

/// 文档站口令：只活在本测试进程的临时配置里，不进仓库配置。
const APIDOC_PW: &str = "apidoc-test-pw";
/// md5("apidoc-test-pw")：插件的 `/apidoc/auth` 收的是 md5(口令)，不是明文。
const APIDOC_PW_MD5: &str = "8bb1ca928d959caadb2c80ead744b75b";

/// 在 api.json 里按 method + url 定位端点。
fn endpoint<'a>(doc: &'a Value, method: &str, url: &str) -> &'a Value {
    doc["endpoints"]
        .as_array()
        .unwrap_or_else(|| panic!("api.json 没有 endpoints 数组"))
        .iter()
        .find(|e| e["method"] == method && e["url"] == url)
        .unwrap_or_else(|| panic!("api.json 里找不到 {method} {url}"))
}

/// 端点的请求体字段 → `[{name, type, required}]`（保持注解顺序 = DTO 字段顺序）。
fn params(ep: &Value) -> Value {
    Value::Array(
        ep["params"]
            .as_array()
            .unwrap_or_else(|| panic!("端点没有 params 数组: {ep}"))
            .iter()
            .map(|p| {
                json!({
                    "name": p["name"],
                    "type": p["type"],
                    "required": p["required"].as_bool().unwrap_or(false),
                })
            })
            .collect(),
    )
}

/// 树形响应 `returned[0]`（data）的 children：节点字段名，按注解顺序。
fn node_fields(ep: &Value) -> Vec<String> {
    ep["returned"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("树形响应没有 children 节点: {ep}"))
        .iter()
        .map(|f| f["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn apidoc_menu_dept_bodies_are_documented() {
    let Some(dsn) = common::dsn() else { return };
    common::reset_db(&dsn).await;
    // 配置在启动时读取：要开文档站，必须换一个进程（start_server_conf 就是干这个的）
    let (_server, base) = common::start_server_conf(|cfg| {
        format!("{cfg}\n[apidoc]\nenabled = true\npassword = {APIDOC_PW}\n")
    })
    .await;
    let c = reqwest::Client::new();

    // 口令换 token（提交 md5；数据路由的 token 走 query，reqwest 负责 URL 编码）
    let v: Value = c
        .get(format!("{base}/apidoc/auth"))
        .query(&[("password", APIDOC_PW_MD5)])
        .send()
        .await
        .expect("apidoc auth 请求失败")
        .json()
        .await
        .expect("apidoc auth 不是 JSON");
    let token = v["token"].as_str().unwrap_or_else(|| panic!("换 token 失败: {v}")).to_string();

    let doc: Value = c
        .get(format!("{base}/apidoc/api.json"))
        .query(&[("token", token.as_str())])
        .send()
        .await
        .expect("api.json 请求失败")
        .json()
        .await
        .expect("api.json 不是 JSON");

    // ── menu：请求体逐字等于 MenuBody（含 serde rename 后的 type）──
    let menu_fields = json!([
        {"name": "parent_id", "type": "string", "required": false},
        {"name": "name", "type": "string", "required": true},
        {"name": "type", "type": "string", "required": true},
        {"name": "perm", "type": "string", "required": false},
        {"name": "path", "type": "string", "required": false},
        {"name": "component", "type": "string", "required": false},
        {"name": "icon", "type": "string", "required": false},
        {"name": "sort", "type": "int", "required": false},
        {"name": "visible", "type": "int", "required": false},
        {"name": "status", "type": "int", "required": false},
    ]);
    assert_eq!(
        params(endpoint(&doc, "POST", "/api/v1/menus")),
        menu_fields,
        "POST /menus 的请求体注解与 MenuBody 不一致"
    );
    assert_eq!(
        params(endpoint(&doc, "PUT", "/api/v1/menus/{id}")),
        menu_fields,
        "PUT /menus/{{id}} 与 create 共用 MenuBody，字段必须一致"
    );

    // ── dept：同理 ──
    let dept_fields = json!([
        {"name": "parent_id", "type": "string", "required": false},
        {"name": "name", "type": "string", "required": true},
        {"name": "sort", "type": "int", "required": false},
        {"name": "leader", "type": "string", "required": false},
        {"name": "phone", "type": "string", "required": false},
        {"name": "status", "type": "int", "required": false},
    ]);
    assert_eq!(
        params(endpoint(&doc, "POST", "/api/v1/depts")),
        dept_fields,
        "POST /depts 的请求体注解与 DeptBody 不一致"
    );
    assert_eq!(
        params(endpoint(&doc, "PUT", "/api/v1/depts/{id}")),
        dept_fields,
        "PUT /depts/{{id}} 与 create 共用 DeptBody，字段必须一致"
    );

    // ── 两个树的响应结构：节点字段 + `children`（递归那一环不能漏）──
    assert_eq!(
        node_fields(endpoint(&doc, "GET", "/api/v1/menus/tree")),
        [
            "id", "parent_id", "name", "type", "perm", "path", "component", "icon", "sort",
            "visible", "status", "created_at", "updated_at", "children"
        ],
        "GET /menus/tree 的节点字段与 Menu 模型不一致"
    );
    assert_eq!(
        node_fields(endpoint(&doc, "GET", "/api/v1/depts/tree")),
        [
            "id", "parent_id", "name", "sort", "leader", "phone", "status", "created_at",
            "updated_at", "children"
        ],
        "GET /depts/tree 的节点字段与 Dept 模型不一致"
    );
}
```

- [ ] **Step 2: 跑一次，**确认它现在就是红的（这就是本批次的负控基线）

```bash
cd /home/wwwroot/bee-rust-admin
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 cargo test -p bee_admin --test api_apidoc_test -- --nocapture
```

期望输出（**故意失败**）：

```
---- apidoc_menu_dept_bodies_are_documented stdout ----
thread '…' panicked at admin/tests/api_apidoc_test.rs:…
assertion `left == right` failed: POST /menus 的请求体注解与 MenuBody 不一致
  left: []
 right: [{"name":"parent_id",…}, …]
```

**要是这里显示 `test result: ok` 且耗时 `0.00s`** → `BEE_ADMIN_DB_DSN` 没设上（静默跳过），重设后再跑。
要是显示 `找不到 POST /api/v1/menus` → 连 api.json 都没取到，先查 `/apidoc/auth` 那步是不是 401。

---

### Task 2: `admin/src/api/menu.rs` 补注解

**Files:**
- Modify: `admin/src/api/menu.rs`

> 行号按基线 `e2b84f3`。插入位置按**内容**定位（`#[apidoc::header(...)]` 之后、
> `pub async fn` 之前），改完前面的 handler 行号会顺移。

- [ ] **Step 1: `tree`（GET `/api/v1/menus/tree`）加 `returned`，插在 `#[apidoc::response_status("200")]` 之后**

```rust
#[apidoc::returned(name = "data", ty = "array", desc = "菜单树（顶层节点数组；含按钮节点）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 顶层）"},
            {name = "name", ty = "string", required, desc = "菜单名"},
            {name = "type", ty = "string", required, desc = "类型：M 目录 / C 菜单 / F 按钮"},
            {name = "perm", ty = "string", desc = "权限码（M 目录为空串）"},
            {name = "path", ty = "string", desc = "前端路由路径（F 按钮为空串）"},
            {name = "component", ty = "string", desc = "前端组件路径（F 按钮为空串）"},
            {name = "icon", ty = "string", desc = "图标名（F 按钮为空串）"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "visible", ty = "int", desc = "是否显示：1 显示 / 0 隐藏"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "children", ty = "array", desc = "子节点（同结构，递归；叶子为空数组 []）"},
        ])]
```

- [ ] **Step 2: `create`（POST `/api/v1/menus`）加 `param` + `returned`**

`param` 十行插在 `#[apidoc::header(...)]` 之后（顺序 = `MenuBody` 字段顺序）：

```rust
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级菜单短串（也认裸数字）；空串 / null / 缺省 = 顶层（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "菜单名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "type", ty = "string", required, desc = "类型：M 目录 / C 菜单 / F 按钮（其它值回 400）")]
#[apidoc::param(name = "perm", ty = "string", desc = "权限码，最长 128 字符（M 目录留空）")]
#[apidoc::param(name = "path", ty = "string", desc = "前端路由路径，最长 128 字符（M / C 用）")]
#[apidoc::param(name = "component", ty = "string", desc = "前端组件路径，最长 128 字符（C 用）")]
#[apidoc::param(name = "icon", ty = "string", desc = "图标名，最长 64 字符（M / C 用）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "visible", ty = "int", desc = "是否显示：1 显示 / 0 隐藏，默认 1")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
```

`returned` 插在 `#[apidoc::response_status("400")]` 之后：

```rust
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新菜单对外 id（hashids 短串）"},
        ])]
```

- [ ] **Step 3: `update`（PUT `/api/v1/menus/{id}`）加同样的 10 行 `param` + `returned`**

`param` 插在 `#[apidoc::route_param(name = "id", …)]` 之后（10 行与 Step 2 **逐字相同** ——
它用的是同一个 `MenuBody`）。`returned` 插在 `#[apidoc::response_status("400")]` 之后：

```rust
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
```

- [ ] **Step 4: `remove`（DELETE `/api/v1/menus/{id}`）加 `returned`，并修掉一行写错的 `response_status`**

`returned`（插在 `response_status` 之后）：

```rust
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
```

**顺带纠错（一行，改动仅限注解，与「不改实现」不冲突）**：现有注解写的是
`#[apidoc::response_status("404")]`，但 `remove` 的实现**从来不返回 404** ——
有子菜单 / 被角色引用走的都是 `ApiError::BadRequest`（400），删除本身不校验存在性（删 0 行也回 200）。
把这一行改成：

```rust
#[apidoc::response_status("400")]
```

> 评审若认为这算超范围，删掉这行改动**不影响**本批次任何验收 —— 但留着它，文档就是错的。

- [ ] **Step 5: 编译**

```bash
cd /home/wwwroot/bee-rust-admin
timeout 900 cargo build -p bee_admin 2>&1 | tail -5
```

期望：`Finished \`dev\` profile [unoptimized + debuginfo] target(s) in …`，无 warning/error。
（注解写错关键字会在**编译期**报 `unknown argument \`xxx\`; expected name, ty, required, default, desc, mock or children`。）

- [ ] **Step 6: 重跑 Task 1 的测试** —— 现在应该**只**在 dept 的断言上红

```bash
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 cargo test -p bee_admin --test api_apidoc_test -- --nocapture
```

期望失败信息变成 `POST /depts 的请求体注解与 DeptBody 不一致`（menu 的四条断言已过）。

---

### Task 3: `admin/src/api/dept.rs` 补注解

**Files:**
- Modify: `admin/src/api/dept.rs`

- [ ] **Step 1: `tree`（GET `/api/v1/depts/tree`）加 `returned`，插在 `#[apidoc::response_status("200")]` 之后**

```rust
#[apidoc::returned(name = "data", ty = "array", desc = "部门树（顶层节点数组）", children = [
            {name = "id", ty = "string", required, desc = "对外 id（hashids 短串）"},
            {name = "parent_id", ty = "string", required, desc = "父节点短串（0 = 根）"},
            {name = "name", ty = "string", required, desc = "部门名"},
            {name = "sort", ty = "int", desc = "排序（升序）"},
            {name = "leader", ty = "string", desc = "负责人（可为空串）"},
            {name = "phone", ty = "string", desc = "联系电话（可为空串）"},
            {name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用"},
            {name = "created_at", ty = "string", desc = "创建时间"},
            {name = "updated_at", ty = "string", desc = "更新时间"},
            {name = "children", ty = "array", desc = "子部门（同结构，递归；叶子为空数组 []）"},
        ])]
```

- [ ] **Step 2: `create`（POST `/api/v1/depts`）加 `param` + `returned`**

`param` 六行插在 `#[apidoc::header(...)]` 之后（顺序 = `DeptBody` 字段顺序）：

```rust
#[apidoc::param(name = "parent_id", ty = "string", desc = "上级部门短串（也认裸数字）；空串 / null / 缺省 = 根（0）")]
#[apidoc::param(name = "name", ty = "string", required, desc = "部门名，最长 64 字符（落库前 trim）")]
#[apidoc::param(name = "sort", ty = "int", desc = "排序（升序），默认 0")]
#[apidoc::param(name = "leader", ty = "string", desc = "负责人，最长 64 字符")]
#[apidoc::param(name = "phone", ty = "string", desc = "联系电话，最长 20 字符")]
#[apidoc::param(name = "status", ty = "int", desc = "状态：1 启用 / 0 禁用，默认 1")]
```

`returned` 插在 `#[apidoc::response_status("400")]` 之后：

```rust
#[apidoc::returned(name = "data", ty = "object", desc = "新建结果", children = [
            {name = "id", ty = "string", required, desc = "新部门对外 id（hashids 短串）"},
        ])]
```

- [ ] **Step 3: `update`（PUT `/api/v1/depts/{id}`）加同样的 6 行 `param` + `returned`**

`param` 插在 `#[apidoc::route_param(name = "id", …)]` 之后（与 Step 2 逐字相同，共用 `DeptBody`）。
`returned` 插在 `#[apidoc::response_status("400")]` 之后：

```rust
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
```

- [ ] **Step 4: `remove`（DELETE `/api/v1/depts/{id}`）加 `returned`**

```rust
#[apidoc::returned(name = "data", ty = "null", desc = "无数据（成功时固定为 null）")]
```

（`remove` 现有的 `response_status("200")` / `("400")` 是对的：有子部门 / 挂着管理员都回 400，不返回 404 —— 不用动。）

- [ ] **Step 5: 编译 + 跑测试，这次要绿**

```bash
cd /home/wwwroot/bee-rust-admin
timeout 900 cargo build -p bee_admin 2>&1 | tail -3
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 cargo test -p bee_admin --test api_apidoc_test -- --nocapture
```

期望输出：

```
test apidoc_menu_dept_bodies_are_documented ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.xx s
```

**`finished in 0.00s` 或输出里有「跳过」= DSN 没生效（假绿灯），重跑。**

---

### Task 4: 手工验收（真 instance + curl + DTO 交叉核对 + 负控）

**Files:**
- 无（只读仓库；临时文件写 `/tmp`）

- [ ] **Step 1: 起一个一次性实例（换端口，别碰 8080）**

```bash
cd /home/wwwroot/bee-rust-admin/admin
{ cat conf/app.conf.test; printf '\n[apidoc]\nenabled = true\npassword = apidoc-test-pw\n'; } \
  | sed 's|127.0.0.1:0|127.0.0.1:18099|' > /tmp/apidoc-check.conf

# 前台跑（大 timeout；Ctrl-C 结束）
BEE_ADMIN_CONF=/tmp/apidoc-check.conf \
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 /home/wwwroot/bee-rust-admin/target/debug/bee_admin
```

期望日志：`[apidoc] enabled=true，文档站挂在 /apidoc（口令鉴权；仅限可信网络）`，
随后 `GET /api/v1/health` 能回 `OK`。（另开一个终端跑下面的命令。）

- [ ] **Step 2: 口令换 token，再抓 api.json**

```bash
PW_MD5=$(printf '%s' 'apidoc-test-pw' | md5sum | cut -d' ' -f1)   # 8bb1ca928d959caadb2c80ead744b75b
TOKEN=$(curl -s "http://127.0.0.1:18099/apidoc/auth?password=$PW_MD5" | jq -r .token)
echo "token 长度: ${#TOKEN}"        # 期望 60+，空串 = 口令 md5 不对

# token 是标准 base64（含 + / ），**必须 --data-urlencode**，手拼 query 会 401
curl -sG "http://127.0.0.1:18099/apidoc/api.json" --data-urlencode "token=$TOKEN" > /tmp/api.json
```

- [ ] **Step 3: 断言请求体不再为空，且逐字是 DTO 的字段**

```bash
# 四个写接口的请求体字段（期望 4 行都不为空）
jq -r '.endpoints[]
  | select(.url | startswith("/api/v1/menus") or startswith("/api/v1/depts"))
  | select(.method != "GET")
  | "\(.method) \(.url) → params=[\([.params[].name] | join(","))]"' /tmp/api.json

# 硬断言：这 4 个端点里「params 为空」的个数必须是 0（退出码 0 = 通过）
jq -e '[.endpoints[]
  | select(.url | startswith("/api/v1/menus") or startswith("/api/v1/depts"))
  | select(.method != "GET") | select((.params | length) == 0)] | length == 0' /tmp/api.json

# 两个树的响应结构（递归的 children 必须在）
jq -r '.endpoints[] | select(.url | endswith("/tree"))
  | "\(.method) \(.url) → 节点字段=[\([.returned[0].children[].name] | join(","))]"' /tmp/api.json
```

期望输出：

```
POST /api/v1/menus → params=[parent_id,name,type,perm,path,component,icon,sort,visible,status]
PUT /api/v1/menus/{id} → params=[parent_id,name,type,perm,path,component,icon,sort,visible,status]
POST /api/v1/depts → params=[parent_id,name,sort,leader,phone,status]
PUT /api/v1/depts/{id} → params=[parent_id,name,sort,leader,phone,status]
true
GET /api/v1/menus/tree → 节点字段=[id,parent_id,name,type,perm,path,component,icon,sort,visible,status,created_at,updated_at,children]
GET /api/v1/depts/tree → 节点字段=[id,parent_id,name,sort,leader,phone,status,created_at,updated_at,children]
```

- [ ] **Step 4: 机器核对「注解 == DTO 源码」（防手抄错）**

这一步是**独立的**第二双眼睛：直接从两个结构体源码里抽字段（含 `#[serde(rename)]` 后的对外名），
和文件里 `#[apidoc::param(name = …)]` 的集合比。**在仓库根目录跑**：

```bash
cd /home/wwwroot/bee-rust-admin
python3 - <<'PY'
import re, sys, pathlib

def dto_fields(src, struct):
    body = re.search(r"pub struct %s \{(.*?)\n\}" % struct, src, re.S).group(1)
    out = []
    for m in re.finditer(r"((?:#\[[^\]]*\]\s*)*)pub ([a-z_][a-z0-9_]*):", body):
        attrs, name = m.group(1), m.group(2)
        rn = re.search(r'rename = "([^"]+)"', attrs)
        out.append(rn.group(1) if rn else name)   # JSON 名（rename 优先）
    return out

ok = True
for path, struct in [("admin/src/api/menu.rs", "MenuBody"),
                     ("admin/src/api/dept.rs", "DeptBody")]:
    src = pathlib.Path(path).read_text()
    dto = dto_fields(src, struct)
    ann = re.findall(r'#\[apidoc::param\(name = "([^"]+)"', src)
    if sorted(set(dto)) != sorted(set(ann)):
        print(f"{path}: 字段集合不一致\n  DTO: {dto}\n  注解: {sorted(set(ann))}"); ok = False
    elif len(ann) != len(dto) * 2:
        print(f"{path}: 注解 {len(ann)} 条 != DTO {len(dto)} 字段 × 2（create+update）"); ok = False
    else:
        print(f"{path}: OK（{len(dto)} 字段 × 2 处注解，逐字一致）")
sys.exit(0 if ok else 1)
PY
```

期望输出（退出码 0）：

```
admin/src/api/menu.rs: OK（10 字段 × 2 处注解，逐字一致）
admin/src/api/dept.rs: OK（6 字段 × 2 处注解，逐字一致）
```

> 基线（改之前）跑这个脚本会红：`注解: []`。这就是「漏了」的机器证明。
> 这个脚本是一次性核对，**不进 CI**（它 regex 源码，脆；长期靠 Task 1 的测试钉）。

- [ ] **Step 5: 负控 —— 证明 Task 1 的断言真的会失败（单个字段级）**

Task 1 Step 2 的「红」证明的是**空请求体**会被抓到；这一步证明**漏一个字段**也会被抓到：

```bash
cd /home/wwwroot/bee-rust-admin
# 1) 临时删掉 menu create 的一行注解（icon 那行，模式唯一）
sed -i '/param(name = "icon", ty = "string", desc = "图标名，最长 64 字符（M \/ C 用）")/d' admin/src/api/menu.rs
grep -c '#\[apidoc::param' admin/src/api/menu.rs     # 期望 19（原 20 - 1）

# 2) 重跑：**必须红**，且失败信息里能看出少了 icon
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 cargo test -p bee_admin --test api_apidoc_test -- --nocapture
```

期望失败输出（`left` 只有 9 项、缺 `{"name":"icon",…}`）：

```
assertion `left == right` failed: POST /menus 的请求体注解与 MenuBody 不一致
```

```bash
# 3) 还原那一行（**别用 git checkout** —— 会连本次的其它改动一起冲掉），
#    然后复核条数与 grep 精确匹配
grep -c '#\[apidoc::param' admin/src/api/menu.rs     # 还原后必须回到 20
timeout 900 cargo build -p bee_admin 2>&1 | tail -3  # 无 error/warning
```

还原方式：把 Step 2 里那行 `icon` 的 `#[apidoc::param(...)]` 原样插回 `component` 与 `sort` 两行之间。
插完用 `git diff --stat admin/src/api/menu.rs` 看改动规模（**本机 `diff` 命令是坏的，用 `git diff`**），
再重跑一次测试确认变绿：

```bash
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 900 cargo test -p bee_admin --test api_apidoc_test
# 期望：test result: ok. 1 passed; 0 failed; … finished in 1.xx s
```

- [ ] **Step 6: 收尾 —— 整包跑一遍，收掉一次性实例**

```bash
cd /home/wwwroot/bee-rust-admin
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  timeout 1800 cargo test -p bee_admin
# 期望：所有 tests/*.rs 全绿（含新加的 api_apidoc_test）
```

外加：
- Ctrl-C 掉 Step 1 的一次性实例，`rm -f /tmp/apidoc-check.conf /tmp/api.json`
- `git status --short` 应只有三个文件：`admin/src/api/menu.rs`、`admin/src/api/dept.rs`、`admin/tests/api_apidoc_test.rs`（外加 `admin/src/api/menu.rs` 里那一条 404→400 的纠错）
- 确认没碰 127.0.0.1:8080：`ss -ltnp | grep 8080` 里那个 pid 从头到尾没变过

## 完成标准

- [ ] `admin/tests/api_apidoc_test.rs` 全绿且**耗时不为 0.00s**（真跑了）
- [ ] `/apidoc/api.json` 里 `POST|PUT /api/v1/menus`、`POST|PUT /api/v1/depts` 的 `params` 非空，字段与 DTO 逐字一致
- [ ] 两个 `/tree` 的 `returned[0].children` 里都有 `children` 节点
- [ ] 负控跑过：删一行注解 → 测试红 → 还原 → 测试绿
- [ ] `BEE_ADMIN_DB_DSN=… cargo test -p bee_admin` 整包绿
- [ ] 无任何接口实现 / DTO / SQL / 鉴权码的改动（本批次只改 `#[apidoc::*]` 属性行）

条数自查（改完 `param` + `returned` 应为 menu 24、dept 16；`param` 单独是 20 / 12）：

```bash
cd /home/wwwroot/bee-rust-admin/admin/src/api
for f in menu dept; do echo "$f.rs: 深注解 $(grep -cE '#\[apidoc::(param|returned)\(' $f.rs)"; done
# 期望：menu.rs: 深注解 24 / dept.rs: 深注解 16
```
