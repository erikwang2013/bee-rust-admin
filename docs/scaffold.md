# 模块脚手架（scripts/scaffold-module.sh）

从「模块名 + 权限前缀 + 字段清单」一键生成一个后台模块的骨架：模型、4 个 CRUD 接口、
单元测试、集成测试、前端 api 封装、前端页面。**生成物一定先编译过**（脚本自己跑
`cargo check -p bee_admin --all-targets` + `pnpm -C admin/web exec tsc -b`），
剩下的接线按下面的清单手工补两口就能跑。

```bash
scripts/scaffold-module.sh message system:message name:string:64 status:bool sort:int remark:text --label 消息
```

| 参数 | 说明 |
| --- | --- |
| `<模块名>` | 小写字母/数字/下划线，字母开头；也是表名、文件名、路由段名 |
| `<权限前缀>` | 形如 `system:message`，脚本按它发 `:list` `:add` `:edit` `:remove` 四个权限码 |
| 字段 | `<名>:<类型>[:<长度>]`，可给多个；第一个 `string` 字段自动当**名称字段**。`id`/`created_at`/`updated_at`/`page`/`size` 是保留名（模型固定列与分页参数），字段也不能重名——会在写盘前拒掉 |
| `--label` | 中文名，只用在 apidoc 标题/标签、菜单种子名、生成物注释与 i18n 提示里（默认用模块名）；**校验回包的 400 文案与新建接口的 apidoc 描述取的是「名称字段」的中文**（命中现有 `field.*` 词表就用它的标签，没命中退回字段名），与 `--label` 无关 |
| `--path` | 菜单/前端路由路径，默认 `/system/<模块名>` |
| `--api-path` | 接口前缀，默认 `/<模块名>s`（模块名本身带 s 时要显式给，比如 `address` → `--api-path /addresses`） |
| `--out DIR` | 写到仓库外的目录，**跳过 mod.rs 与自检**；只用来对照/试跑，别拿它上线 |

字段类型：

| 写法 | Rust | MySQL | 前端表单 |
| --- | --- | --- | --- |
| `string:64` | `String` | `VARCHAR(64)` | `Input`（名称字段必填；每个文本列都在入口卡列长，超长回 400 而不是 500） |
| `text` | `String` | `TEXT` | `Input.TextArea` |
| `int` / `i64` | `i32` / `i64` | `INT` / `BIGINT` | `InputNumber` |
| `bool` | `i8` | `TINYINT` | `Select`（1 启用 / 0 禁用） |

## 生成哪些文件

| 文件 | 内容 |
| --- | --- |
| `admin/src/models/<模块名>.rs` | 模型（`id` / 业务字段 / `created_at` / `updated_at`） |
| `admin/src/api/<模块名>.rs` | list / create / update / remove + apidoc 注解 + 文本列长度校验（含单测） |
| `admin/tests/api_<模块名>_test.rs` | 真库真进程跑一遍 CRUD（含 401 闸门、空白名 400、删完查不到） |
| `admin/web/src/api/<模块名>.ts` | 类型 + `xxxApi` 四个方法（类型留在模块自己的文件里，不动共享 `types.ts`） |
| `admin/web/src/pages/system/<模块名>/index.tsx` | 列表 + 新建/编辑弹窗 + 删除（`Auth` 包着操作按钮） |
| `admin/src/api/mod.rs`、`admin/src/models/mod.rs` | 各**追加一行** `pub mod`（不加，上面两个 .rs 根本不参与编译，自检就是假绿） |

## 接线清单（脚本会把这份原样打印出来）

编号与脚本打印的完全一致：`①` `③` 粘完就能调接口，`④` 粘完页面才有入口。

```rust
// ① admin/src/seed.rs 的 migrate 清单：建表（只加列不删列，安全）
migrate::sync::<crate::models::Message, _>(db).await?;

// ② 同文件 BUILTIN_MENUS：菜单 + 四个权限码（不粘的话只有超管能调，普通角色一律 403）
(
    "消息",
    "/system/message",
    "system/message/index",
    "ProfileOutlined", // 图标名必须在 admin/web/src/icons.ts 的白名单里
    "system:message",
    &["list", "add", "edit", "remove"],
),

// ③ admin/src/main.rs 的路由链（位置随意，别塞进别人的 ns 里）
.ns("/api/v1/messages", |ns| {
    ns.get("", api::message::list)
        .post("", api::message::create)
        .put("/{id}", api::message::update)
        .delete("/{id}", api::message::remove)
})
```

```tsx
// ④ admin/web/src/router.tsx
const MessagePage = lazy(() => import('./pages/system/message/index'));
{ path: 'system/message', element: <RequirePerm code="system:message:list"><L><MessagePage /></L></RequirePerm> },
```

`⑤` ②里用的图标名必须在 `admin/web/src/icons.ts` 的白名单里（`ProfileOutlined` 已有），
否则菜单是空的。

`⑥` `admin/tests/common/mod.rs` 的 `reset_db` 表清单里加 `"message"`：不加的话上一轮跑
的数据会留到下一轮（生成的测试用唯一名字，不会因此假失败）；顺带也让「删表重建」这条路径
覆盖到新表。

`⑦` 字段名不在现有 `field.*` 词表里的话，脚本会附带一份 i18n 占位片段，粘不粘都行
（不粘就是拿字段名当标签）。

另外，接完线把 `admin/src/api/message.rs` 与 `admin/src/models/message.rs` 顶上那两行
`#![allow(dead_code)]` 删掉 —— 它们是「还没接线」这段过渡期压编译警告用的，接完线就该消失。

**为什么这些不让脚本自己改**：菜单种子、路由挂载、前端路由都是结构化编辑（挂哪个 ns、
菜单摆哪一层、要不要 i18n 专属文案只有人知道），脚本改错比不改更糟 —— 一个错位的 `})`
就是一处编译不过或一处串味的路由。脚本只做两件确定安全的事：写新文件、往既有文件的
同类声明块里追加一行。

## 编译不过怎么办

**改脚本，不要改生成物把它糊过去。** 生成物是模板的输出，手工修一份只说明模板坏了，
下一个模块还会坏一次。自检失败时脚本会**先整体回滚**（还原两个 `mod.rs`、删掉本次生成的
5 个文件与新建的页面目录）再退出并提示这一点 —— 工作树回到跑脚本之前，改了脚本重跑即可。

## 演示：用脚手架重造「字典管理」对照

```bash
bash scripts/scaffold-module.sh dict system:dict \
  name:string:64 code:string:64 status:bool remark:string:255 \
  --label 字典管理 --out /tmp/sk
```

生成物与仓库里人工写的 `admin/src/api/dict.rs`、`admin/src/models/dict.rs` 逐段对得上：

- 模型：`#[bee(table)]` + `hid::ser_id/de_id` 的 `id` + `sql_type` + `created_at/updated_at` 两列，一致；
- 接口：`auth.require("<码>")` → `page_size` → `filter_contains/filter_eq` → `fetch_page` →
  `ok(json!({ "list", "total" }))` 的列表形状，`state.next_id()` + `insert` 的新建形状，
  `ok_or(ApiError::NotFound)` 的编辑/删除形状，一致；
- 差的是字典模块长在骨架之上的业务：`dict_item` 子表、`code` 的不可改语义、导出、
  `(type_code, value)` 的复合唯一键 DDL —— 这些是「往骨架里长」的部分，脚手架不生成。

`--out` 模式生成完就退出，不碰仓库、不跑自检；它只是拿来对照和试跑的。

## 不做的事

- **不覆盖任何已有文件**：任一目标已存在就整体退出（一次误跑会清掉线上模块）。
  同理不修改任何既有模块的代码，除了上面那两个 `pub mod` 追加。
- **不生成 DDL**：建表一律走 `migrate::sync`（加列不删列）。要复合唯一键、外键、
  特殊索引，自己往 `seed.rs` 的裸 DDL 清单里加，写法照着现有的 `idx_attachment_entity`。
- **不支持 `datetime` 字段**：时间列只有 `created_at/updated_at`，由服务端写。
  业务要自己的时间列（比如「生效时间」）就手工加 —— 那样就得处理前端 `DatePicker`
  和时区，属于业务，不属于骨架。
- **不生成业务逻辑**：级联删、状态机、导出、数据权限收窄……都是「长在骨架上」的东西，
  骨架只保证 CRUD 通、校验在、鉴权在、测试在。
- **不动 i18n 词表**：页面只引**已经存在**的 `field.*` 键（引新键 tsc 立刻报错），
  字段名不在词表里时先用字段名当标签。想要模块专属文案，自己往
  `admin/web/src/i18n/zh-CN.ts` 与 `en-US.ts` 各加一份（**两边键集合必须一致**）。
