# 数据权限 2 档 / 5 档 集成测试 计划

> 上位文档：[v2.6 之后的路线图](2026-10-11-post-v2.6-roadmap.md)（本文对应其中 **D2**，建议排期见该文 §3）。

日期：2026-10-11 · 目标：把 `data_scope` 的 **2 本部门及以下** 与 **5 自定义** 从「零集成覆盖」补成「有实测」

## 为什么要做

`admin/src/datascope.rs::resolve` 有五档，集成测试的实际覆盖是：

| 档 | 含义 | 代码 | 集成覆盖 |
|---|---|---|---|
| 1 | 全部数据 | `datascope.rs:257` | 有（超管走 `is_super` 短路，`api_flow_test.rs` 全程） |
| 2 | 本部门及以下（子树） | `datascope.rs:261` | **0 处** |
| 3 | 本部门 | `datascope.rs:262` | 有（`api_flow_test.rs:82-97`） |
| 4 | 仅本人 | `datascope.rs:263` | 有（3 处，见下） |
| 5 | 自定义（`role_dept` 勾选） | `datascope.rs:264-270` | **0 处** |

设计文档 `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md:414` 自己写着这个缺口。
2 档是**递归子树**、5 档是**跨部门扁平集合**，方向相反，恰恰是逻辑最容易写错的两档。
数据权限写错 = 越权看到别人的数据，这是本仓库最该有测试的地方之一。

## 勘察结论（执行时可直接采信，不必重查）

### 1. `resolve` 的五档语义（`admin/src/datascope.rs:244-277`）

- 超管 → `all = true` 立即返回。
- 无启用角色 → 空 `DataScope` → 条件 `1 = 0`（**不是** 403）。
- 逐角色累加：`1` → 全返回；`2` → `extend(subtree(all_depts, 自己部门))`；`3` → `push(自己部门)`；`4` → `self_only = true`；`5` → `extend(role_dept 里勾的部门)`。
- 收尾 `sort_unstable + dedup`。
- **2 档与 3 档都只作用于部门，不含 `self_only`**；唯一加 `self_only` 的是 4 档。

`subtree`（`datascope.rs:281-297`）：BFS 含自身，`visited` 用 `out.contains` 防环；
**`root == 0`（无部门）提前返回空 Vec** —— 注释写得很清楚：否则会返回整片森林。

### 2. 两套 WHERE 生成器（`datascope.rs:30-70`）

- `condition(dept_col, self_col)` → `(dept_id IN (...) OR id = ...)`，给管理员列表/详情/导出用。
- `admin_id_condition()` → `(admin_id IN (SELECT id FROM admin WHERE dept_id IN (...)) OR admin_id = ...)`，给登录记录/操作日志用。
- 两者都内联整数（不是 `?` 占位符，理由见 `datascope.rs:22-29`），都为空时返回 `1 = 0`。

**这是两段独立代码，两档都该各打一次**，否则日志类那条子查询路径仍然没被实测。

### 3. 已有数据权限测试的骨架在哪（照抄这些）

| 位置 | 覆盖 | 可抄的东西 |
|---|---|---|
| `admin/tests/api_flow_test.rs:30-97` | **3 档 + 超管**，主骨架 | 建部门→建角色→勾菜单→建管理员→登录→`GET /admins` 断言 `total` |
| `admin/tests/api_v14_test.rs:50-92` | 辅助函数 | `menu_id(menus, perm)` 按**权限码**递归找菜单；`make_role(...)` 建角色+勾菜单；`csv_lines(...)` 读导出 |
| `admin/tests/api_v12_test.rs:183-196` | 4 档 | `data_scope: 4` 的角色 + 「无权限 403」 |
| `admin/tests/api_v15_test.rs:214-222`、`api_v16_test.rs:374-382` | 4 档 | 空角色（不勾任何菜单）的造法 |
| `admin/tests/common/mod.rs` | 脚手架 | `dsn()`（含 `_test` 护栏，`:29-46`）、`reset_db`、`start_server`、`login`、`as_id`、`dec_id`、`my_id` |

`api_flow_test.rs:94-97` 就是 3 档的核心断言，**2 档用例照它的形状写，只是部门从一层变成两层**。

### 4. 造数据会不会被授权规则挡住 —— **不会**（这是最容易踩的坑，结论如下）

`ensure_roles_grantable`（`datascope.rs:151-182`）/ `block_reason`（`:132-146`）规定：
**非超管只能授予 `data_scope ∈ {3, 4}` 的角色**，授 2 档或 5 档会 403 `scope.wider_role`。

但是：

- `ensure_roles_grantable` 的**第一行就是 `if auth.is_super { return Ok(()); }`**（`datascope.rs:152-154`）。
- 建角色（`admin/src/api/role.rs::create`，`:233-255`）**不调用**任何 `ensure_roles_grantable`，只做 `require("system:role:add")` + `validate_scope`（`role.rs:77-82`，取值 1-5 全放行）。
- 设置角色的部门 `PUT /roles/{id}/depts`（`role.rs:429-452`）同样只 `require("system:role:edit")`。

**绕法：全部造数据都用超管 `admin` / `admin123` 做**（`reset_db` 后 seed 重建，密码来自 `admin/conf/app.conf.test` 末行 `[seed] initial_admin_password = admin123`）。
超管造 2 档/5 档角色、把角色授给测试管理员，全程不触发 `wider_role` 校验。**不需要任何 hack。**

**唯一要记住的**：一旦用非超管去 `POST /admins` 挂 2 档或 5 档角色，会 403 —— 所以造数据阶段必须待在超管身份里。

### 5. 5 档「一个部门都没勾」的实际行为 —— **什么都看不到（含自己）**

读代码得出，链路完整：

```rust
// datascope.rs:264-270
5 => { let ids = db.get_relations("role_dept", ("role_id", role.id), "dept_id").await?;
       scope.dept_ids.extend(ids); }          // 空 → 什么都不加
// datascope.rs:274-276  sort_unstable + dedup → 仍空
// datascope.rs:30-45    condition(): all=false, dept_ids 空, self_only=false
//                       → parts.is_empty() → Some("1 = 0")
```

`self_only` **不会**被 5 档顺带置真 —— 它是 `4 =>` 分支独占的。
所以 5 档空勾选 = `1 = 0` = 列表 0 条，**连自己刚产生的登录记录也看不到**。
这和「4 档空勾选」不是一回事，必须单独钉住。

### 6. 受数据权限影响的接口（哪档在哪打）

| 接口 | 文件 | 用的生成器 | 本计划用在哪 |
|---|---|---|---|
| `GET /admins` | `admin.rs:236` | `condition("dept_id","id")` | 两个 Task 的主断言 |
| `GET /admins/{id}` | `admin.rs:367` | `ensure_admin_in_scope` | Task 1（列表与详情同口径） |
| `GET /admins/export` | `admin.rs:313-314` | `condition`（`export_batch`） | Task 1（导出不能绕过） |
| `GET /login-logs` | `login_log.rs:66-70` | `admin_id_condition` | Task 1（子查询路径） |
| `GET /audit-logs` | `audit_log.rs:73-75` | `admin_id_condition` | 不单独打，与登录记录同代码路径 |

### 7. 两个反直觉边界（写测试时必须知道）

- **无部门（`dept_id = 0`）的人在谁的子树里都不在**。超管 dept 也是 0，所以 2 档/3 档/5 档的任何人**永远看不到超管**。`t2_root`（挂在最顶层部门）也看不到超管与无部门的人。
- **5 档是扁平集合，不是子树**。勾了「B部」不会带出「B1组」。这是它和 2 档最大的区别，也是本计划最值钱的断言。

---

## 通用约定（两个 Task 都适用）

- **一个文件一个 `#[tokio::test]`**。`cargo` 串行跑各测试二进制，同库上的 `reset_db` 才不会互踩（`.github/workflows/test.yml:59-64` 明写）。**不要**在一个文件里放两个 `#[tokio::test]` —— 两个函数会在同进程并行跑、互相 `reset_db`。
- 每个测试文件**自带** `call` / `menu_id` / `make_role` 等辅助（照抄 `api_v14_test.rs:12-92`）。仓库里每个测试文件都是这么做的，**不要去改 `common/mod.rs`**（它被所有测试二进制共同编译，动它会波及其它同事）。
- **不断言 hashid 字面量**（id 是雪花生成的，跨轮会变）。断言**用户名集合**与 `total`；确需 id 时用 `common::as_id` / `common::dec_id` / `common::my_id` 现取。
- 断言消息一律带上 `{v}`，失败时好定位。

### 环境与命令（先读这一段，能省半小时）

本机 `admin/conf/app.conf` 指向 **`bee_admin`（生产实例，127.0.0.1:8080）**。集成测试会 `DROP` 全部表重建，
**绝不能用它**。测试库 `bee_admin_test` 本机已存在。从生产配置里取凭据、换库名：

```bash
cd /home/wwwroot/bee-rust-admin
DSN=$(sed -n 's/^dsn *= *//p' admin/conf/app.conf | sed -E 's#/[^/?]+([?].*)?$#/bee_admin_test#')
echo "$DSN" | sed -E 's#(//[^:]*:)[^@]*@#\1***@#'    # 自查：打印出来的库名必须以 _test 结尾
```

跑新增用例：

```bash
BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope2_test -- --nocapture
BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope5_test -- --nocapture
```

整包（CI 口径，`.github/workflows/test.yml:52-64`）：

```bash
BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin -- --nocapture
```

**没设 `BEE_ADMIN_DB_DSN` 时集成用例会静默跳过**（`common/mod.rs:41-45` 打印「跳过」并 `return`），
cargo 仍然报 `ok`。**判据是耗时**：

```
# 真的跑了：
test result: ok. 1 passed; ... finished in 12.34s
# 假的（没设 DSN，什么都没测）：
test result: ok. 1 passed; ... finished in 0.00s
```

`0.00s` 一律当失败处理。

---

## Task 1 · 2 档（本部门及以下）集成用例

**Files:**
- 新建 `admin/tests/api_datascope2_test.rs`（唯一改动；**不要**动 `common/mod.rs` 与任何已有测试）

### 数据形状

部门（六层，全部由超管 `POST /depts` 建，`parent_id` 传 hashid、根传 `0`）：

```
总部            parent=0
├── 研发部       parent=总部
│   ├── 前端组    parent=研发部
│   └── 后端组    parent=研发部
└── 市场部       parent=总部
    └── 华东区    parent=市场部
```

角色 `t2_role`：`data_scope = 2`，勾两个菜单 —— `system:admin:list`、`system:loginlog:list`
（用 `menu_id(&menus, "system:admin:list")` 这种**按权限码**找法，别按名字）。

管理员（全部超管建，`dept_id` 传部门 hashid，无部门传 `0`）：

| 用户名 | 部门 | 角色 | 作用 |
|---|---|---|---|
| `admin` | 无（seed 建） | — | 超管，绝不该出现在任何人的列表里 |
| `t2_rd` | 研发部 | `t2_role` | **主操作者**：本部门+两层子部门 |
| `t2_fe` | 前端组 | `t2_role` | 见证：子部门 |
| `t2_be` | 后端组 | `t2_role` | 见证：子部门 |
| `t2_mkt` | 市场部 | `t2_role` | 见证：兄弟分支，不可见 |
| `t2_east` | 华东区 | `t2_role` | 见证：兄弟分支的子部门，不可见 |
| `t2_root` | 总部 | `t2_role` | 根部门操作者：看到整片森林，仍看不到超管与无部门者 |
| `t2_none` | 无部门 | `t2_role` | **边界：`subtree(0)` 为空** |

一共 8 个管理员。登录记录靠 `common::login` 产生：**8 个用户各登录一次**（`t2_fe`/`t2_be`/`t2_mkt`/`t2_east`/`t2_root` 只为了留一条登录记录，token 可丢弃；`t2_rd`/`t2_none` 的 token 要留着）。

### 步骤

- [ ] **1.1** 新建 `admin/tests/api_datascope2_test.rs`，头部照抄 `api_flow_test.rs:1-36`：`mod common;`、`call(...)` 辅助、`api = |p| format!("{base}/api/v1{p}")`。
- [ ] **1.2** 写 `#[tokio::test] async fn datascope2_subtree_visibility()`：
  ```rust
  let Some(dsn) = common::dsn() else { return };
  common::reset_db(&dsn).await;
  let (_server, base) = common::start_server().await;
  let c = reqwest::Client::new();
  let api = |p: &str| format!("{base}/api/v1{p}");
  let admin_token = common::login(&base, "admin", "admin123").await.expect("超管登录失败");
  ```
  `let Some(dsn) = common::dsn() else { return }` 这一行是必须的（护栏 + 跳过开关），照抄。
- [ ] **1.3** 加 `make_dept(c, base, token, name, parent_hashid) -> String` 与 `make_role(c, base, token, code, scope, menu_ids) -> String`（后者照抄 `api_v14_test.rs:63-92`），建上面 6 个部门 + 1 个角色。
      断言每个 `POST /depts` 都是 `200`；`parent_id` 传父部门的 hashid 串，根传 `json!(0)`。
- [ ] **1.4** 拉 `GET /menus/tree`，用 `menu_id(&v["data"], "system:admin:list")` 与 `"system:loginlog:list"` 取两个菜单 id，
      `PUT /roles/{t2_role}/menus {"menu_ids":[...]}` 勾上。
- [ ] **1.5** 建 7 个测试管理员（超管身份），收集各自的 hashid 到一个 `HashMap<&str, String>`。
- [ ] **1.6** 依次登录 8 个用户，确认 `t2_rd`、`t2_none` 拿到 token（`expect("... 登录失败")`）。
- [ ] **1.7** 断言（**这就是本任务的全部价值**）：

  | # | 操作者 | 请求 | 期望 |
  |---|---|---|---|
  | A1 | `t2_rd` | `GET /admins?size=100` | `200`；`total == 3`；`list` 的 username 集合 **恰为** `{t2_rd, t2_fe, t2_be}` |
  | A2 | `t2_rd` | `GET /login-logs?size=100` | `200`；username 集合 **恰为** `{t2_rd, t2_fe, t2_be}` |
  | A3 | `t2_rd` | `GET /admins/export` | `csv_lines` 的数据行 = 3（`lines.len() - 1`），且每行不含 `t2_mkt`/`t2_east`/`t2_none`/`admin` |
  | A4 | `t2_rd` | `GET /admins/{t2_east}` | `403`，`msg` 含「超出你的数据权限范围」 |
  | A5 | `t2_rd` | `GET /admins/{t2_fe}` | `200`（范围内的详情照常可读，别把闸门写死） |
  | A6 | `t2_root` | `GET /admins?size=100` | `200`；`total == 6`（8 个减去超管与 `t2_none`） |
  | A7 | `t2_none` | `GET /admins?size=100` | `200`（**不是 403**）；`total == 0`；`list == []` |
  | A8 | `t2_none` | `GET /login-logs?size=100` | `200`；`total == 0`（连自己刚产生的登录记录也看不到） |
  | A9 | `t2_none` | `GET /admins/export` | 只有表头 1 行，数据行 0 |

  集合断言写法（顺序无关，`id DESC` 排序不要依赖）：
  ```rust
  let mut names: Vec<&str> = v["data"]["list"].as_array().unwrap()
      .iter().map(|r| r["username"].as_str().unwrap()).collect();
  names.sort_unstable();
  assert_eq!(names, vec!["t2_be", "t2_fe", "t2_rd"], "2 档应只见自己子树: {v}");
  ```
- [ ] **1.8** 跑 `BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope2_test -- --nocapture`。

**期望输出**：

```
running 1 test
test datascope2_subtree_visibility ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1x.xxs
```

耗时必须是**秒级**（`0.00s` = 被静默跳过了，见「环境与命令」）。

---

## Task 2 · 5 档（自定义）集成用例

**Files:**
- 新建 `admin/tests/api_datascope5_test.rs`（唯一改动）

### 数据形状

部门（五个，全部超管建）：

```
总部        parent=0
├── A部      parent=总部     ← 操作者所在的「自己分支」
│   └── A1组  parent=A部
└── B部      parent=总部     ← 勾选的目标「别人的分支」
    └── B1组  parent=B部
```

角色（全部 `data_scope = 5` 或指定值，全部勾 `system:admin:list`）：

| 角色 | scope | `PUT /roles/{id}/depts` 勾的部门 | 用意 |
|---|---|---|---|
| `t5_cross` | 5 | `{B部, B1组}` | 跨分支勾选 = 本任务主角 |
| `t5_bonly` | 5 | `{B部}` | **只勾父不勾子**：证明 5 档不是子树 |
| `t5_none` | 5 | `{}`（不调 depts 接口，或传空数组） | **一个都没勾** |
| `t5_dept3` | 3 | —（不调） | 多角色并集用 |
| `t5_all` | 1 | —（不调） | 任一 1 档 → 全部 |

管理员（全部超管建）：

| 用户名 | 部门 | 角色 | 作用 |
|---|---|---|---|
| `admin` | 无 | — | 超管，只有 1 档者看得见 |
| `t5_op` | A部 | `[t5_cross]` | **主操作者**：自己在 A 部，只能看 B 部子树 |
| `t5_b` | B部 | `[t5_cross]` | 见证：可见 |
| `t5_b1` | B1组 | `[t5_cross]` | 见证：可见 |
| `t5_a` | A部 | `[t5_cross]` | 见证：与操作者同部门，**不可见** |
| `t5_a1` | A1组 | `[t5_cross]` | 见证：不可见 |
| `t5_bonly_op` | A部 | `[t5_bonly]` | **边界：只勾父不勾子** |
| `t5_none_op` | B部 | `[t5_none]` | **边界：一个都没勾** |
| `t5_multi` | A部 | `[t5_dept3, t5_cross]` | **多角色并集** |
| `t5_all_op` | A1组 | `[t5_cross, t5_all]` | **任一 1 档 → 全部** |

一共 **10 个管理员**。部门归属：A部 = `{t5_op, t5_a, t5_bonly_op, t5_multi}`（4）、B部 = `{t5_b, t5_none_op}`（2）、B1组 = `{t5_b1}`（1）、A1组 = `{t5_a1, t5_all_op}`（2）、无部门 = `{admin}`（1）。

登录：只需 5 个操作者 `t5_op` / `t5_bonly_op` / `t5_none_op` / `t5_multi` / `t5_all_op` 拿 token（见证者不用登录，也不需要角色给的权限 —— 它们照样出现在列表里）。

### 步骤

- [ ] **2.1** 新建 `admin/tests/api_datascope5_test.rs`，头部与辅助同 Task 1（`call` / `make_dept` / `make_role` / `menu_id`）。
- [ ] **2.2** `#[tokio::test] async fn datascope5_custom_depts_visibility()`，同样以 `common::dsn()` 护栏 + `reset_db` + `start_server` + 超管登录开场。
- [ ] **2.3** 建 5 个部门；建 5 个角色（`POST /roles`），每个都勾 `system:admin:list`。
- [ ] **2.4** 调 `PUT /roles/{t5_cross}/depts {"dept_ids":[B部, B1组]}`、`PUT /roles/{t5_bonly}/depts {"dept_ids":[B部]}`、
      `PUT /roles/{t5_none}/depts {"dept_ids":[]}`。**每次都断言 `200`**，并 `GET` 回读一次断言勾选落库
      （`GET /roles/{id}/depts` 的 `data` 就是 id 短串数组，`api_flow_test.rs:178-180` 是现成写法）。
      `t5_dept3` / `t5_all` **不要**调 depts 接口（它们不是 5 档）。
- [ ] **2.5** 建 9 个测试管理员（超管身份），`role_ids` 按上表。
- [ ] **2.6** 登录 5 个操作者拿 token。
- [ ] **2.7** 断言：

  | # | 操作者 | 请求 | 期望 | 钉住什么 |
  |---|---|---|---|---|
  | B1 | `t5_op` | `GET /admins?size=100` | `200`；`total == 3`；集合 = `{t5_b, t5_b1, t5_none_op}` | **跨分支能看 + 看不到自己所在的 A 部** |
  | B2 | `t5_bonly_op` | `GET /admins?size=100` | `200`；`total == 2`；集合 = `{t5_b, t5_none_op}`（**不含 `t5_b1`**） | **5 档是扁平集合，不是子树** |
  | B3 | `t5_none_op` | `GET /admins?size=100` | `200`（**不是 403**）；`total == 0`；`list == []` | **一个都没勾 = 什么都看不到（含自己）** |
  | B4 | `t5_multi` | `GET /admins?size=100` | `200`；`total == 7`；集合 = `{t5_op, t5_a, t5_bonly_op, t5_multi}` ∪ `{t5_b, t5_none_op}` ∪ `{t5_b1}`（**不含 `t5_a1`、不含超管**） | **多角色并集；3 档同样非递归** |
  | B5 | `t5_all_op` | `GET /admins?size=100` | `200`；`total == 10`（含超管） | **任一角色 1 档 → 全部** |
  | B6 | `t5_op` | `GET /admins?size=100&dept_id={A部}` | `200`；`total == 0` | **筛选参数不能放大数据范围** |
  | B7 | `t5_op` | `GET /admins?size=100&dept_id={B部}` | `200`；`total == 2` | 筛选与范围是「与」的关系 |

  B1 与 B2 是一对：**同一个 A 部的人，勾了 B1组就能看到它、只勾 B部就看不到** —— 这一对同时排除了
  「5 档被写成子树」和「5 档被写成 2 档」两种实现错误。B1 里 `t5_op` **看不到自己**是本计划最反直觉的一条，
  它证明 5 档不会隐式带上自己的部门。

  > **并集去重的说明**：`resolve` 收尾的 `sort_unstable + dedup` 在外部**不可观测**（`IN (7,7)` 与 `IN (7)` 结果相同），
  > 不为它编造断言。B4 钉的是可观测的并集语义；去重由 `datascope.rs:274-276` 的代码本身保证。

- [ ] **2.8** 跑 `BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope5_test -- --nocapture`。

**期望输出**：与 Task 1 同形，`running 1 test` / `test datascope5_custom_depts_visibility ... ok` / `1 passed`，耗时秒级。

---

## 验收

- [ ] 两个新文件各含**且仅含一个** `#[tokio::test]`：
      `grep -c '#\[tokio::test\]' admin/tests/api_datascope2_test.rs admin/tests/api_datascope5_test.rs` → 各为 `1`。
- [ ] 两个新用例单独跑都 `1 passed` 且**耗时秒级**（`0.00s` 视为未跑）：
      ```bash
      BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope2_test -- --nocapture
      BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin --test api_datascope5_test -- --nocapture
      ```
- [ ] 整包不带回归：
      ```bash
      BEE_ADMIN_DB_DSN="$DSN" cargo test -p bee_admin -- --nocapture
      ```
      与改动前相比 **passed 总数 +2**（每个新文件一个 `#[tokio::test]`），`0 failed`。
      两个新二进制必须出现在输出里（`Running tests/api_datascope2_test.rs` 等）——
      没出现 = 文件名不在 `tests/` 下或没编译进去。
- [ ] 未改动 `common/mod.rs`、`datascope.rs` 与任何已有测试文件：
      `git status --porcelain` 里**没有任何 ` M` / `M ` 开头的行**（改动只应是 `??` 新增）。
      你应看到的两行 `??` 是 `admin/tests/api_datascope2_test.rs` 与 `admin/tests/api_datascope5_test.rs`；
      同目录下可能还有同事在写的其它 `docs/superpowers/plans/*.md`（`??`），与你无关。
      校验文件内容有没有被顺手改动用 `git diff`；**本机的 `diff` 命令是坏的**（不同文件也返回 0），
      要对比文件用 `cmp`。此处只新增文件，`git status` 足够。

## 负控（证明断言真的有牙，跑完必须还原）

每条都是**故意把 `admin/src/datascope.rs` 改坏 → 重跑对应 Task → 断言必须红 → 还原**。
不跑负控就无法区分「断言通过」与「断言根本不可能失败」。

| # | 改法 | 重跑 | 期望的失败 |
|---|---|---|---|
| NC1 | `datascope.rs:261` 的 `2 => scope.dept_ids.extend(subtree(&all_depts, auth.admin.dept_id)),` 改成 `2 => scope.dept_ids.push(auth.admin.dept_id),`（子树退化成只有自己） | Task 1 | A1 断言红：`total` 应为 3 实际 1 |
| NC2 | 删掉 `subtree` 开头的 `if root == 0 { return Vec::new(); }`（`datascope.rs:282-284`） | Task 1 | A7 断言红：`t2_none` 的 `total` 应为 0 实际 6（整片森林） |
| NC3 | `datascope.rs:264-270` 的 `5 => { ... }` 整段换成 `5 => scope.dept_ids.push(auth.admin.dept_id),`（5 档被写成本部门） | Task 2 | B1 断言红：`total` 应为 3 实际 4（A 部的人） |
| NC4 | `5 => { let ids = ...; scope.dept_ids.extend(ids); }` 改成对每个勾中的部门取子树：`for d in ids { scope.dept_ids.extend(subtree(&all_depts, d)); }` | Task 2 | B2 断言红：`total` 应为 2 实际 3（B1组 被带出来了）—— 这一条专门证明「2 档 vs 5 档」那对断言不是同义反复 |

还原：`git checkout -- admin/src/datascope.rs`，然后 `git status --porcelain` 确认干净
（**别用 `diff` 核对**，本机 `diff` 是坏的；用 `git diff` 或 `cmp`）。

## 本机的坑（血泪史，逐条对应）

1. **集成测试必须设 `BEE_ADMIN_DB_DSN`**，否则静默跳过、`0.00s` 报 `ok`（`common/mod.rs:41-45`）。
2. **库名必须以 `_test` 结尾**，`common::dsn()` 有护栏会直接 panic（`common/mod.rs:34-38`）。
   本机 `admin/conf/app.conf` 指向的是**生产库 `bee_admin`**，一定要换库名，见「环境与命令」。
3. **测试库要预先存在** —— 全仓库没有 `CREATE DATABASE`，应用只建表不建库。本机 `bee_admin_test` 已存在。
4. **后台跑 cargo 会被内存监控杀掉**，必须前台跑并给大 `timeout`（首次编译整包要几分钟，
   Bash 工具默认 120s 不够 —— 把 `timeout` 设到 600000）。
5. **不许碰 127.0.0.1:8080 的生产实例**（`admin/conf/app.conf` 那套）。集成测试自己起随机端口进程
   （`common/mod.rs:166-170`），不占 8080；只要别手动去 `cargo run` 生产配置就行。
6. **本机 `diff` 命令是坏的**（不同文件也返回 0）。核对文件用 `git diff` 或 `cmp`。
7. **cargo 串行跑测试二进制**，所以「一文件一 `#[tokio::test]`」是硬约束，不是一个偏好。

## 明确不做

- 不改 `admin/src/datascope.rs` 的实现（负控阶段除外，且必须还原）。
- 不改 `common/mod.rs`（被所有测试二进制共同编译，动它会波及别的分支/同事）。
- 不引入新测试框架、不加依赖。
- 不重写已有的 1 / 3 / 4 档用例（它们在 `api_flow_test.rs` / `api_v12_test.rs` / `api_v15_test.rs` / `api_v16_test.rs` 里）。
- 不为 2 档 / 5 档单独写 `condition` 的单元测试 —— `datascope.rs:299-353` 已有的单测覆盖了字符串渲染，
  本计划补的是**端到端**（真库 + 真进程）那一段，两者不重复。
