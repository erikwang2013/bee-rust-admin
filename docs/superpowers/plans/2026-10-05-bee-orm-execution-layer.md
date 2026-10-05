# bee_orm 执行层（M1+M2）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/bee_orm` 从"只有 SQL 拼接器"补成可用的异步 ORM：连接池、CRUD、QuerySet 执行、M2M 关联、syncdb 迁移，执行引擎用 sqlx(MySQL)。

**Architecture:** 现有 `QuerySet` 参数化 SQL 构造保留，新增执行方法（`fetch_all/fetch_one/count/fetch_page`）；`Db` 是唯一入口（连接池 + CRUD + 关联 + syncdb + 事务）；`#[derive(Model)]` 生成 `ModelMeta` 元数据、`sqlx` 行映射、各语句的绑定代码。SQL 语句由元数据拼装，值一律走绑定。

**Tech Stack:** Rust 1.99（edition 2024）、sqlx 0.8（runtime-tokio/mysql/chrono/json/tls-none）、chrono 0.4、proc-macro（syn 2 + quote）、MySQL 8.4。

**设计依据:** `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md` §4。

**验证用测试库:** `bee_admin_test`（本机 MySQL 已建）。集成测试靠环境变量 `BEE_ORM_TEST_DSN` 打开，未设置则跳过：

```bash
export BEE_ORM_TEST_DSN='mysql://root:<密码>@127.0.0.1:3306/bee_admin_test'
```

真密码不入库、不写进本文件。所有断言基于真库执行结果。

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `crates/bee_orm/Cargo.toml` | 依赖换 sqlx；删三个未实现的 driver optional 依赖 |
| `crates/bee_orm/src/lib.rs` | `Model` trait、`__private` 宏重导出、模块声明 |
| `crates/bee_orm/src/error.rs` | `OrmError`、错误归一化（唯一键冲突）、标识符校验 |
| `crates/bee_orm/src/meta.rs` | `ModelMeta`/`ColumnMeta`/`ColumnType`/`SyncdbMode`（纯数据） |
| `crates/bee_orm/src/query.rs` | `QuerySet`：SQL 构造 + 执行 |
| `crates/bee_orm/src/db.rs` | `Db`/`Tx`：连接、CRUD、关联、事务 |
| `crates/bee_orm/src/syncdb.rs` | DDL 生成（纯函数）+ `Db::syncdb` 实现 |
| `crates/bee_orm_macro/src/lib.rs` | `#[derive(Model)]` 重写 |
| `crates/bee_orm/tests/model_macro.rs` | 宏展开结果的断言（无库） |
| `crates/bee_orm/tests/sql_gen.rs` | SQL/DDL 生成断言（无库） |
| `crates/bee_orm/tests/mysql_integration.rs` | 真库集成测试（`BEE_ORM_TEST_DSN` 开关） |

---

### Task 1: 依赖换成 sqlx + 基础模块（error/meta/lib）

**Files:**
- Modify: `crates/bee_orm/Cargo.toml`
- Modify: `crates/bee_orm/src/lib.rs`（删除内联 `OrmError`，声明模块，重写 `Model` trait）
- Create: `crates/bee_orm/src/error.rs`
- Create: `crates/bee_orm/src/meta.rs`

- [ ] **Step 1: 改 Cargo.toml**

`[dependencies]` 段整体替换为（**删掉** `rusqlite`/`tokio-postgres`/`mysql_async` 三个从未实现的 optional 依赖及 `[features]`）：

```toml
[features]
# 目前只有 MySQL 后端（sqlx）。Postgres/SQLite 需要占位符改写与各自的 FromRow，
# 等真有项目用再加。
default = []

[dependencies]
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
bee_orm_macro = { version = "1.1.5", path = "../bee_orm_macro" }
chrono = { version = "0.4", features = ["serde"] }
sqlx = { version = "0.8", default-features = false, features = [
    "runtime-tokio",
    "mysql",
    "chrono",
    "json",
    "tls-none",
] }

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "rt-multi-thread", "macros"] }
```

- [ ] **Step 2: 写 `src/error.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use sqlx::Error as SqlxError;

#[derive(Debug, thiserror::Error)]
pub enum OrmError {
    #[error("connection error: {0}")]
    ConnectionError(String),
    #[error("query error: {0}")]
    QueryError(String),
    #[error("invalid field name: {0}")]
    InvalidField(String),
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Sqlx(#[from] SqlxError),
    #[error("row mapping error: {0}")]
    Row(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("duplicate key: {0}")]
    DuplicateKey(String),
}

/// 把 sqlx 错误归一化：唯一键冲突（MySQL 1062，SQLSTATE 23000）单独成类，
/// 上层可转成「用户名已存在」这类友好提示。
pub(crate) fn normalize(e: SqlxError) -> OrmError {
    if let SqlxError::Database(db) = &e {
        if db.code().as_deref() == Some("23000") {
            return OrmError::DuplicateKey(db.message().to_string());
        }
    }
    OrmError::Sqlx(e)
}

/// 标识符（表名/列名）只允许 `[A-Za-z_][A-Za-z0-9_]*`；其余一律拒绝，
/// 防止拼进 SQL 造成注入。
pub(crate) fn validate_ident(name: &str) -> Result<(), OrmError> {
    let mut chars = name.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(()) } else { Err(OrmError::InvalidField(name.to_string())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ident_accepts_plain_names() {
        assert!(validate_ident("admin_role").is_ok());
        assert!(validate_ident("_x1").is_ok());
    }

    #[test]
    fn validate_ident_rejects_injection() {
        assert!(validate_ident("admin; DROP TABLE x").is_err());
        assert!(validate_ident("1abc").is_err());
        assert!(validate_ident("").is_err());
    }
}
```

- [ ] **Step 3: 写 `src/meta.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
/// 列类型：Rust 侧类型到 MySQL 列的映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    U64,
    I64,
    U32,
    I32,
    I16,
    I8,
    Bool,
    String,
    F64,
    DateTime,
    Json,
}

#[derive(Debug, Clone, Copy)]
pub struct ColumnMeta {
    pub name: &'static str,
    pub ty: ColumnType,
    pub auto: bool,
    pub unique: bool,
    pub index: bool,
    pub nullable: bool,
    pub text: bool,
    pub len: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    pub table: &'static str,
    pub pk: Option<&'static str>,
    pub columns: &'static [ColumnMeta],
}

/// syncdb 模式。Safe：只建表/加列/补索引，绝不删改。Force 预留未实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncdbMode {
    Safe,
    Force,
}
```

- [ ] **Step 4: 改 `src/lib.rs`（增量改，保留现有 QuerySet）**

**不要重写整个文件。** 现有 `QuerySet`（含 `filter/filter_eq/filter_gt/filter_lt/filter_contains/params/order_by/limit/offset/to_sql`）与文件底部私有的 `validate_field` **原样保留**，Task 3 才把它们搬进 `query.rs`。本步只做四件事：

1. 删掉内联的 `pub enum OrmError`（已移到 `error.rs`）
2. 顶部加模块声明与重导出：`pub mod error; pub mod meta; pub use error::OrmError; pub use meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};`
3. 把旧的 `pub trait Model: Send + Sync + 'static {}` 换成下面的新定义（含 `__private` 模块）
4. 顶部 `use` 调整为：`use sqlx::QueryBuilder; use sqlx::mysql::{MySql, MySqlRow};`（`use std::marker::PhantomData;` 保留给 QuerySet 用）

新增/替换的内容如下（其余保持原样）：

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! bee_orm：Beego 风格的异步 ORM，执行引擎基于 sqlx(MySQL)。

pub mod error;
pub mod meta;

pub use bee_orm_macro::Model;
pub use error::OrmError;
pub use meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};

use sqlx::QueryBuilder;
use sqlx::mysql::{MySql, MySqlRow};

/// 供 `#[derive(Model)]` 展开代码使用的内部重导出，不是公开 API。
#[doc(hidden)]
pub mod __private {
    pub use sqlx;
    pub use sqlx::QueryBuilder;
    pub use sqlx::Row;
    pub use sqlx::mysql::{MySql, MySqlRow};
}

/// 模型：元数据由 `#[derive(Model)]` 生成，CRUD 与 syncdb 都从这里取信息。
pub trait Model: Send + Sync + 'static {
    const META: ModelMeta;

    /// 从一行结果按列名映射出模型（宏生成）。
    fn from_row(row: &MySqlRow) -> Result<Self, sqlx::Error>
    where
        Self: Sized;

    /// 追加 INSERT 的绑定值，顺序与 META 中非自增列一致（宏生成）。
    fn bind_insert<'q>(&self, qb: QueryBuilder<'q, MySql>) -> QueryBuilder<'q, MySql>;

    /// 追加 UPDATE 的 `SET col = ?, ... WHERE pk = ?`（宏生成）。
    fn bind_update<'q>(&self, qb: QueryBuilder<'q, MySql>) -> QueryBuilder<'q, MySql>;

    /// 自增主键回填（宏在存在 `#[bee(auto)]` 字段时生成）。
    fn set_auto_pk(&mut self, _id: u64) {}
}
```

- [ ] **Step 5: 编译与单测验证**

Run: `cargo build -p bee_orm`
Expected: 编译**成功**（当前还没有任何地方 use 派生宏，新 trait 不要求旧宏实现完整）。若失败信息指向 sqlx feature/依赖解析，先解决（`cargo tree -p bee_orm | grep sqlx` 应显示 sqlx v0.8.6）。

Run: `cargo test -p bee_orm --lib`
Expected: error.rs 的 `validate_ident_accepts_plain_names`、`validate_ident_rejects_injection` 两个单测 PASS。若 `cargo build` 报 `tls-none` feature 不存在，说明 sqlx 版本不是 0.8.6+，用 `cargo tree -p bee_orm | grep sqlx` 确认后反馈。

- [ ] **Step 6: Commit**

```bash
git add crates/bee_orm/Cargo.toml crates/bee_orm/src/error.rs crates/bee_orm/src/meta.rs crates/bee_orm/src/lib.rs Cargo.lock
git commit -m "feat(bee_orm): sqlx 执行引擎依赖 + 错误/元数据模块与 Model trait 重定义"
```

---

### Task 2: 重写 `#[derive(Model)]`

**Files:**
- Modify: `crates/bee_orm_macro/src/lib.rs`
- Create: `crates/bee_orm/tests/model_macro.rs`

- [ ] **Step 1: 先写失败测试 `crates/bee_orm/tests/model_macro.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::{ColumnType, Model};

#[derive(Model)]
#[bee(table = "ut_admin", pk = "id")]
pub struct UtAdmin {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub username: String,
    pub status: i8,
    #[bee(index)] pub dept_id: u64,
    pub note: Option<String>,
    #[bee(text)] pub bio: String,
    pub created_at: chrono::NaiveDateTime,
}

/// 无属性：表名取 snake_case + "s"（与旧行为兼容），主键自动认 `id`，不自增。
#[derive(Model)]
pub struct DefUser {
    pub id: i64,
    pub name: String,
}

/// 无 `id` 字段：无主键（连接表用）。
#[derive(Model)]
#[bee(table = "ut_link")]
pub struct UtLink {
    pub admin_id: u64,
    pub role_id: u64,
}

#[test]
fn meta_reflects_attributes() {
    assert_eq!(UtAdmin::META.table, "ut_admin");
    assert_eq!(UtAdmin::META.pk, Some("id"));
    assert_eq!(UtAdmin::META.columns.len(), 7);

    let id = UtAdmin::META.columns[0];
    assert_eq!(id.name, "id");
    assert!(id.auto);
    assert_eq!(id.ty, ColumnType::U64);

    assert!(UtAdmin::META.columns[1].unique);
    assert_eq!(UtAdmin::META.columns[2].ty, ColumnType::I8);
    assert!(UtAdmin::META.columns[3].index);
    assert!(UtAdmin::META.columns[4].nullable);
    assert!(UtAdmin::META.columns[5].text);
    assert_eq!(UtAdmin::META.columns[6].ty, ColumnType::DateTime);
}

#[test]
fn defaults_are_sane() {
    assert_eq!(DefUser::META.table, "def_users");
    assert_eq!(DefUser::META.pk, Some("id"));
    assert!(!DefUser::META.columns[0].auto);
    assert_eq!(DefUser::META.columns[0].ty, ColumnType::I64);

    assert_eq!(UtLink::META.table, "ut_link");
    assert_eq!(UtLink::META.pk, None);
}

#[test]
fn query_uses_table_name() {
    assert_eq!(DefUser::query().to_sql(), "SELECT * FROM def_users");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p bee_orm --test model_macro`
Expected: 编译失败：`error[E0046]: not all trait items implemented, missing: META, from_row, bind_insert, bind_update`（旧宏生成的空 `impl Model` 满足不了新 trait）。同一个错误现在也出现在框架自带的 `crates/bee_orm/tests/orm_tests.rs`（Task 1 之后的预期中间态），本任务的宏重写要把它一并修好。

- [ ] **Step 3: 重写 `crates/bee_orm_macro/src/lib.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, LitInt, LitStr, PathArguments, Type,
    parse_macro_input,
};

#[proc_macro_derive(Model, attributes(bee))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// 与 `bee_orm::ColumnType` 的变体一一对应。
#[derive(Clone, Copy)]
enum Ty {
    U64,
    I64,
    U32,
    I32,
    I16,
    I8,
    Bool,
    String,
    F64,
    DateTime,
    Json,
}

impl Ty {
    fn ident(self) -> proc_macro2::Ident {
        let name = match self {
            Ty::U64 => "U64",
            Ty::I64 => "I64",
            Ty::U32 => "U32",
            Ty::I32 => "I32",
            Ty::I16 => "I16",
            Ty::I8 => "I8",
            Ty::Bool => "Bool",
            Ty::String => "String",
            Ty::F64 => "F64",
            Ty::DateTime => "DateTime",
            Ty::Json => "Json",
        };
        format_ident!("{}", name)
    }
}

struct Field {
    ident: syn::Ident,
    column: String,
    ty: Ty,
    nullable: bool,
    auto: bool,
    unique: bool,
    index: bool,
    text: bool,
    len: Option<u32>,
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let mut table: Option<String> = None;
    let mut pk: Option<String> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("bee") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                table = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("pk") {
                pk = Some(meta.value()?.parse::<LitStr>()?.value());
            } else {
                return Err(meta.error("未知的 bee 容器属性（支持 table / pk）"));
            }
            Ok(())
        })?;
    }
    let table = table.unwrap_or_else(|| format!("{}s", to_snake(&name.to_string())));

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => return Err(syn::Error::new_spanned(name, "Model 只支持具名字段结构体")),
        },
        _ => return Err(syn::Error::new_spanned(name, "Model 只支持结构体")),
    };

    let mut infos = Vec::new();
    for f in fields {
        let ident = f.ident.clone().expect("具名字段");
        let (ty, nullable) = type_to_col(&f.ty)?;
        let mut info = Field {
            column: ident.to_string(),
            ident,
            ty,
            nullable,
            auto: false,
            unique: false,
            index: false,
            text: false,
            len: None,
        };
        for attr in &f.attrs {
            if !attr.path().is_ident("bee") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("auto") {
                    info.auto = true;
                } else if meta.path.is_ident("unique") {
                    info.unique = true;
                } else if meta.path.is_ident("index") {
                    info.index = true;
                } else if meta.path.is_ident("text") {
                    info.text = true;
                } else if meta.path.is_ident("len") {
                    info.len = Some(meta.value()?.parse::<LitInt>()?.base10_parse()?);
                } else {
                    return Err(meta.error(
                        "未知的 bee 字段属性（支持 auto/unique/index/text/len）",
                    ));
                }
                Ok(())
            })?;
        }
        infos.push(info);
    }

    let pk = pk.or_else(|| infos.iter().find(|f| f.column == "id").map(|f| f.column.clone()));
    let table_str = table.as_str();
    let pk_tokens = match &pk {
        Some(p) => quote!(Some(#p)),
        None => quote!(None),
    };

    let col_metas = infos.iter().map(|f| {
        let (n, t) = (&f.column, f.ty.ident());
        let (auto, unique, index, text, nullable) =
            (f.auto, f.unique, f.index, f.text, f.nullable);
        let len = match f.len {
            Some(n) => quote!(Some(#n)),
            None => quote!(None),
        };
        quote! {
            bee_orm::ColumnMeta {
                name: #n, ty: bee_orm::ColumnType::#t, auto: #auto, unique: #unique,
                index: #index, nullable: #nullable, text: #text, len: #len,
            }
        }
    });

    let from_row_fields = infos.iter().map(|f| {
        let (ident, col) = (&f.ident, &f.column);
        quote! { #ident: bee_orm::__private::Row::try_get(row, #col)? }
    });

    let insert_binds = infos.iter().filter(|f| !f.auto).map(|f| {
        let ident = &f.ident;
        quote! { sep.push_bind(self.#ident.clone()); }
    });

    let update_sets: Vec<_> = infos.iter().filter(|f| !f.auto).collect();
    // 每列一个完整片段（首列不带分隔符）——不要用两个长度不等的迭代器做 quote 重复，
    // quote 的重复要求所有迭代器等长，否则宏展开处直接报错。
    let update_chunks: Vec<_> = update_sets
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let (ident, col) = (&f.ident, &f.column);
            if i == 0 {
                quote! { qb.push(#col).push(" = ").push_bind(self.#ident.clone()); }
            } else {
                quote! { qb.push(", ").push(#col).push(" = ").push_bind(self.#ident.clone()); }
            }
        })
        .collect();

    // 主键列名用于 UPDATE 尾部；没有主键时不生成（Db::update 会先报错）。
    let update_where = match pk.as_deref().and_then(|p| infos.iter().find(|f| f.column == p)) {
        Some(f) => {
            let (ident, col) = (&f.ident, &f.column);
            quote! { qb.push(" WHERE ").push(#col).push(" = ").push_bind(self.#ident.clone()); }
        }
        None => quote! {},
    };

    let auto_setter = if infos.iter().any(|f| f.auto) {
        let f = infos.iter().find(|f| f.auto).expect("auto 字段");
        let ident = &f.ident;
        quote! { fn set_auto_pk(&mut self, id: u64) { self.#ident = id; } }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl bee_orm::Model for #name {
            const META: bee_orm::ModelMeta = bee_orm::ModelMeta {
                table: #table_str,
                pk: #pk_tokens,
                columns: &[ #(#col_metas),* ],
            };

            fn from_row(row: &bee_orm::__private::MySqlRow) -> Result<Self, bee_orm::__private::sqlx::Error> {
                Ok(Self { #(#from_row_fields),* })
            }

            fn bind_insert<'q>(
                &self,
                mut qb: bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql>,
            ) -> bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql> {
                {
                    let mut sep = qb.separated(", ");
                    #(#insert_binds)*
                }
                qb
            }

            fn bind_update<'q>(
                &self,
                mut qb: bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql>,
            ) -> bee_orm::__private::QueryBuilder<'q, bee_orm::__private::MySql> {
                qb.push("SET ");
                #(#update_chunks)*
                #update_where
                qb
            }

            #auto_setter
        }

        impl #name {
            pub fn query() -> bee_orm::QuerySet<Self> {
                bee_orm::QuerySet::new(#table_str)
            }

            pub fn table_name() -> &'static str {
                #table_str
            }
        }
    })
}

/// `Option<T>` 展开为内层类型 + 可空；未知类型报编译错误。
fn type_to_col(ty: &Type) -> syn::Result<(Ty, bool)> {
    let Type::Path(p) = ty else {
        return Err(syn::Error::new_spanned(ty, "不支持的字段类型"));
    };
    let seg = p.path.segments.last().expect("类型路径非空");
    let name = seg.ident.to_string();
    if name == "Option" {
        let PathArguments::AngleBracketed(args) = &seg.arguments else {
            return Err(syn::Error::new_spanned(ty, "Option 需要类型参数"));
        };
        let GenericArgument::Type(inner) = args.args.first().expect("Option 有类型参数") else {
            return Err(syn::Error::new_spanned(ty, "Option 需要类型参数"));
        };
        let (t, _) = type_to_col(inner)?;
        return Ok((t, true));
    }
    let t = match name.as_str() {
        "u64" => Ty::U64,
        "i64" => Ty::I64,
        "u32" => Ty::U32,
        "i32" => Ty::I32,
        "i16" => Ty::I16,
        "i8" => Ty::I8,
        "bool" => Ty::Bool,
        "String" => Ty::String,
        "f64" => Ty::F64,
        "NaiveDateTime" => Ty::DateTime,
        "Value" => Ty::Json,
        other => {
            return Err(syn::Error::new_spanned(
                ty,
                format!("不支持的字段类型 `{other}`（支持 u64/i64/u32/i32/i16/i8/bool/String/f64/NaiveDateTime/serde_json::Value 及 Option<T>）"),
            ));
        }
    };
    Ok((t, false))
}

/// `UtAdmin` → `ut_admin`。默认表名 = snake_case + "s"（`User` → `users`，与旧行为兼容；
/// 复数化是朴素加 s，不处理 `Address` → `addresss` 这类，需要就用 `#[bee(table = "…")]`）。
fn to_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
```

注意：宏展开引用 `bee_orm::__private::sqlx::Error`（Task 1 的 `__private` 已含 `pub use sqlx;`）与 `bee_orm::QuerySet`（Task 2 期间仍在 `lib.rs` 内，直接可达）。

- [ ] **Step 4: 跑测试确认通过（含框架自带测试）**

Run: `cargo test -p bee_orm --test model_macro`
Expected: 3 个测试全 PASS。

Run: `cargo test -p bee_orm`
Expected: **全部通过**，其中包括框架自带、**不得修改**的 `crates/bee_orm/tests/orm_tests.rs`（13 个测试，`test_table_name` 断言 `User` → `"users"`，正是默认表名 `snake_case + "s"` 的依据）。这个文件是本任务验收的硬指标：宏重写后它必须原样编译并通过。

- [ ] **Step 5: Commit**

```bash
git add crates/bee_orm_macro/src/lib.rs crates/bee_orm/tests/model_macro.rs crates/bee_orm/src/lib.rs
git commit -m "feat(bee_orm_macro): 重写 Model 派生（table/pk/字段属性 + 元数据 + 行映射 + 绑定）"
```

---

### Task 3: `QuerySet` 迁移 + 执行方法

**Files:**
- Create: `crates/bee_orm/src/db.rs`（最小骨架：`Db{pool}` + connect/pool/exec_sql；Task 4 在其上追加 CRUD）
- Create: `crates/bee_orm/src/query.rs`
- Modify: `crates/bee_orm/src/lib.rs`（删掉内联 QuerySet，改 `pub mod db; pub mod query;` + 重导出）
- Create: `crates/bee_orm/tests/sql_gen.rs`

- [ ] **Step 1: 写失败测试 `crates/bee_orm/tests/sql_gen.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use bee_orm::{Model, QuerySet};

#[derive(Model)]
#[bee(table = "sg_admin", pk = "id")]
pub struct SgAdmin {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub username: String,
    pub status: i8,
    pub dept_id: u64,
    pub note: Option<String>,
    #[bee(text)] pub bio: String,
    pub created_at: chrono::NaiveDateTime,
}

#[test]
fn select_sql_keeps_order_limit_offset() {
    let sql = SgAdmin::query()
        .filter_eq("status", 1)
        .unwrap()
        .order_by("id DESC")
        .limit(10)
        .offset(20)
        .to_sql();
    assert_eq!(
        sql,
        "SELECT * FROM sg_admin WHERE status = ? ORDER BY id DESC LIMIT 10 OFFSET 20"
    );
}

#[test]
fn filter_in_builds_placeholders() {
    let qs = SgAdmin::query().filter_in("dept_id", vec![1u64, 2, 3]).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE dept_id IN (?, ?, ?)");
    assert_eq!(qs.params(), ["1", "2", "3"]);
}

#[test]
fn filter_in_empty_is_always_false() {
    let qs = SgAdmin::query().filter_in("dept_id", Vec::<u64>::new()).unwrap();
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE 1 = 0");
    assert!(qs.params().is_empty());
}

#[test]
fn filter_raw_binds_params() {
    let qs = SgAdmin::query().filter_raw("(status = ? OR username = ?)", &["1", "root"]);
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE (status = ? OR username = ?)");
    assert_eq!(qs.params().len(), 2);
}

#[test]
fn page_is_one_based_and_capped() {
    assert_eq!(SgAdmin::query().page(3, 10).to_sql(), "SELECT * FROM sg_admin LIMIT 10 OFFSET 20");
    assert_eq!(SgAdmin::query().page(0, 9999).to_sql(), "SELECT * FROM sg_admin LIMIT 500 OFFSET 0");
}

#[test]
fn count_sql_drops_order_and_page() {
    let sql = SgAdmin::query()
        .filter_eq("status", 1)
        .unwrap()
        .order_by("id DESC")
        .limit(5)
        .count_sql();
    assert_eq!(sql, "SELECT COUNT(*) FROM sg_admin WHERE status = ?");
}

#[test]
fn invalid_field_is_rejected() {
    assert!(SgAdmin::query().filter_eq("bad field", "x").is_err());
    assert!(SgAdmin::query().filter_in("bad;drop", vec![1u64]).is_err());
}
```

约定：`filter_raw` 直接返回 `Self`（raw 片段由调用方保证是常量，无需校验，因此不像 `filter_eq` 那样返回 `Result`）。

```rust
#[test]
fn filter_raw_binds_params() {
    let qs = SgAdmin::query().filter_raw("(status = ? OR username = ?)", &["1", "root"]);
    assert_eq!(qs.to_sql(), "SELECT * FROM sg_admin WHERE (status = ? OR username = ?)");
    assert_eq!(qs.params().len(), 2);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p bee_orm --test sql_gen`
Expected: 编译失败：`filter_in` / `filter_raw` / `page` / `count_sql` 不存在。

- [ ] **Step 3: 写 `crates/bee_orm/src/db.rs`（最小骨架，Task 4 在其上追加 CRUD）**

`query.rs` 的执行方法要拿 `&Db`，所以本任务先落一个最小 `Db`（连接池 + 取池 + 裸 SQL）：

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::error::{OrmError, normalize};
use sqlx::mysql::{MySqlPool, MySqlPoolOptions};

/// 连接池句柄；Clone 廉价（内部 Arc）。
#[derive(Clone)]
pub struct Db {
    pool: MySqlPool,
}

impl Db {
    pub async fn connect(dsn: &str) -> Result<Self, OrmError> {
        let pool = MySqlPoolOptions::new()
            .max_connections(10)
            .connect(dsn)
            .await
            .map_err(|e| OrmError::ConnectionError(e.to_string()))?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    /// 执行写死的 SQL（DDL/运维/建测试库）。参数化查询一律走 QuerySet / CRUD。
    pub async fn exec_sql(&self, sql: &str) -> Result<u64, OrmError> {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(normalize)
            .map(|r| r.rows_affected())
    }
}
```

- [ ] **Step 4: 写 `src/query.rs`**

把 lib.rs 里的 `QuerySet` 整体搬过来，`validate_field` 换成 `error::validate_ident`，`filter_eq/gt/lt/contains` 的 `value: impl Into<String>` 放宽为 `impl ToString`，并补新方法：

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use crate::db::Db;
use crate::error::{OrmError, normalize, validate_ident};
use crate::Model;
use std::marker::PhantomData;

/// 流式 SQL 查询构造器。值一律参数化绑定；`to_sql()` 仅用于调试与测试。
#[derive(Clone)]
pub struct QuerySet<T: Model> {
    table: String,
    filters: Vec<String>,
    params: Vec<String>,
    order_clauses: Vec<String>,
    limit_val: Option<usize>,
    offset_val: Option<usize>,
    _marker: PhantomData<T>,
}

impl<T: Model> QuerySet<T> {
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            filters: Vec::new(),
            params: Vec::new(),
            order_clauses: Vec::new(),
            limit_val: None,
            offset_val: None,
            _marker: PhantomData,
        }
    }

    /// 原样 WHERE 片段（调用方只传写死的常量；用户输入走 filter_eq/filter_in 等）。
    pub fn filter(mut self, condition: impl Into<String>) -> Self {
        self.filters.push(condition.into());
        self
    }

    /// `field = ?`
    pub fn filter_eq(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} = ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field > ?`
    pub fn filter_gt(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} > ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field < ?`
    pub fn filter_lt(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} < ?"));
        self.params.push(value.to_string());
        Ok(self)
    }

    /// `field LIKE ?`，值两侧自动加 `%`。
    pub fn filter_contains(mut self, field: impl Into<String>, value: impl ToString) -> Result<Self, OrmError> {
        let field = field.into();
        validate_ident(&field)?;
        self.filters.push(format!("{field} LIKE ?"));
        self.params.push(format!("%{}%", value.to_string()));
        Ok(self)
    }

    /// `field IN (?, ?, ...)`；空集合生成恒假条件 `1 = 0`（语义正确且不报错）。
    pub fn filter_in<I, V>(mut self, field: impl Into<String>, values: I) -> Result<Self, OrmError>
    where
        I: IntoIterator<Item = V>,
        V: ToString,
    {
        let field = field.into();
        validate_ident(&field)?;
        let vals: Vec<String> = values.into_iter().map(|v| v.to_string()).collect();
        if vals.is_empty() {
            self.filters.push("1 = 0".into());
            return Ok(self);
        }
        let placeholders = vec!["?"; vals.len()].join(", ");
        self.filters.push(format!("{field} IN ({placeholders})"));
        self.params.extend(vals);
        Ok(self)
    }

    /// 原样 SQL 片段 + 绑定参数。片段必须是写死的常量；参数仍走绑定，不做字符串拼接。
    pub fn filter_raw<P: ToString>(mut self, sql: impl Into<String>, params: &[P]) -> Self {
        self.filters.push(sql.into());
        self.params.extend(params.iter().map(|p| p.to_string()));
        self
    }

    pub fn order_by(mut self, clause: impl Into<String>) -> Self {
        self.order_clauses.push(clause.into());
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit_val = Some(n);
        self
    }

    pub fn offset(mut self, n: usize) -> Self {
        self.offset_val = Some(n);
        self
    }

    /// 1 起始页码 + 每页条数（size 上限 500）。
    pub fn page(mut self, page: usize, size: usize) -> Self {
        let page = page.max(1);
        let size = size.clamp(1, 500);
        self.limit_val = Some(size);
        self.offset_val = Some((page - 1) * size);
        self
    }

    /// 绑定参数（`?` 占位符按顺序对应）。执行方法内部使用；测试用来断言参数。
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// 构造 SELECT SQL（调试/测试用；执行走 fetch_*，值不被拼进 SQL）。
    pub fn to_sql(&self) -> String {
        let mut sql = format!("SELECT * FROM {}", self.table);
        self.push_where(&mut sql);
        if !self.order_clauses.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order_clauses.join(", "));
        }
        if let Some(limit) = self.limit_val {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        if let Some(offset) = self.offset_val {
            sql.push_str(&format!(" OFFSET {offset}"));
        }
        sql
    }

    /// COUNT 语句：丢掉 ORDER BY / LIMIT / OFFSET。
    pub fn count_sql(&self) -> String {
        let mut sql = format!("SELECT COUNT(*) FROM {}", self.table);
        self.push_where(&mut sql);
        sql
    }

    fn push_where(&self, sql: &mut String) {
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.filters.join(" AND "));
        }
    }

    /// 执行查询，返回全部行。
    pub async fn fetch_all(&self, db: &Db) -> Result<Vec<T>, OrmError> {
        let sql = self.to_sql();
        let mut q = sqlx::query(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        q.try_map(|row| T::from_row(&row))
            .fetch_all(db.pool())
            .await
            .map_err(normalize)
    }

    /// 执行查询，返回首行。
    pub async fn fetch_one(&self, db: &Db) -> Result<Option<T>, OrmError> {
        let sql = self.to_sql();
        let mut q = sqlx::query(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        q.try_map(|row| T::from_row(&row))
            .fetch_optional(db.pool())
            .await
            .map_err(normalize)
    }

    /// 计数。
    pub async fn count(&self, db: &Db) -> Result<u64, OrmError> {
        let sql = self.count_sql();
        let mut q = sqlx::query_scalar::<_, u64>(&sql);
        for p in &self.params {
            q = q.bind(p.clone());
        }
        q.fetch_one(db.pool()).await.map_err(normalize)
    }

    /// 分页：返回（当前页数据, 总数）。`page` 为 1 起始页码。
    pub async fn fetch_page(&self, db: &Db, page: usize, size: usize) -> Result<(Vec<T>, u64), OrmError> {
        let total = self.count(db).await?;
        let rows = self.clone().page(page, size).fetch_all(db).await?;
        Ok((rows, total))
    }
}
```

- [ ] **Step 5: 改 `src/lib.rs`**

删掉内联的 `QuerySet` 与 `validate_field`，改为：

```rust
pub mod db;
pub mod query;

pub use db::Db;
pub use query::QuerySet;
```

（`pub use db::Tx;` 到 Task 4 有 Tx 之后再加。）

- [ ] **Step 6: 跑测试**

Run: `cargo test -p bee_orm -p bee_orm_macro`
Expected: `model_macro`(6)、`sql_gen`、`orm_tests`(12)、宏 crate(3) 全 PASS；`mysql_integration` 因未设 DSN 跳过。

- [ ] **Step 7: Commit**

```bash
git add crates/bee_orm/src/db.rs crates/bee_orm/src/query.rs crates/bee_orm/src/lib.rs crates/bee_orm/tests/sql_gen.rs
git commit -m "feat(bee_orm): Db 连接骨架 + QuerySet 迁至 query 模块（执行方法/filter_in/filter_raw/page）"
```

---

### Task 4: `Db` 连接与 CRUD

**Files:**
- Create: `crates/bee_orm/src/db.rs`
- Modify: `crates/bee_orm/src/lib.rs`（`pub mod db;` + 重导出 `Db`）
- Create: `crates/bee_orm/tests/mysql_integration.rs`

- [ ] **Step 1: 写失败测试 `crates/bee_orm/tests/mysql_integration.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 真库集成测试：设了 BEE_ORM_TEST_DSN 才跑，否则跳过（打印原因）。
use bee_orm::{Db, Model, OrmError};

const DDL: &str = "CREATE TABLE it_admin (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  username VARCHAR(255) NOT NULL DEFAULT '',
  password VARCHAR(255) NOT NULL DEFAULT '',
  status TINYINT NOT NULL DEFAULT 0,
  dept_id BIGINT UNSIGNED NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL,
  UNIQUE KEY uk_username (username)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";

#[derive(Model)]
#[bee(table = "it_admin", pk = "id")]
pub struct ItAdmin {
    #[bee(auto)] pub id: u64,
    pub username: String,
    pub password: String,
    pub status: i8,
    pub dept_id: u64,
    pub created_at: chrono::NaiveDateTime,
}

fn dsn() -> Option<String> {
    match std::env::var("BEE_ORM_TEST_DSN") {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("跳过：未设置 BEE_ORM_TEST_DSN");
            None
        }
    }
}

fn new_admin(name: &str, status: i8, dept: u64) -> ItAdmin {
    ItAdmin {
        id: 0,
        username: name.into(),
        password: "x".into(),
        status,
        dept_id: dept,
        created_at: chrono::Utc::now().naive_utc(),
    }
}

#[tokio::test]
async fn crud_and_query_flow() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_admin").await.unwrap();
    db.exec_sql(DDL).await.unwrap();

    // insert 回填自增主键
    let mut a = new_admin("alice", 1, 7);
    let id = db.insert(&mut a).await.unwrap();
    assert!(id > 0);
    assert_eq!(a.id, id);

    // read
    let got = db.read::<ItAdmin>(id).await.unwrap().unwrap();
    assert_eq!(got.username, "alice");
    assert_eq!(got.dept_id, 7);
    assert!(db.read::<ItAdmin>(999_999).await.unwrap().is_none());

    // update
    let mut got = got;
    got.status = 0;
    assert_eq!(db.update(&got).await.unwrap(), 1);
    assert_eq!(db.read::<ItAdmin>(id).await.unwrap().unwrap().status, 0);

    // 唯一键冲突 → DuplicateKey
    let mut dup = new_admin("alice", 1, 7);
    assert!(matches!(db.insert(&mut dup).await, Err(OrmError::DuplicateKey(_))));

    // 再插两条，查过滤/分页/count
    for n in ["bob", "carol"] {
        let mut u = new_admin(n, 1, 7);
        db.insert(&mut u).await.unwrap();
    }
    assert_eq!(ItAdmin::query().count(&db).await.unwrap(), 3);
    assert_eq!(ItAdmin::query().filter_eq("status", 1).unwrap().count(&db).await.unwrap(), 2);
    let all = ItAdmin::query().order_by("id ASC").fetch_all(&db).await.unwrap();
    assert_eq!(all.len(), 3);
    let (rows, total) = ItAdmin::query().filter_eq("status", 1).unwrap().fetch_page(&db, 1, 1).await.unwrap();
    assert_eq!((rows.len(), total), (1, 2));
    let one = ItAdmin::query().filter_eq("username", "bob").unwrap().fetch_one(&db).await.unwrap().unwrap();
    assert_eq!(one.username, "bob");
    assert!(ItAdmin::query().filter_eq("username", "nobody").unwrap().fetch_one(&db).await.unwrap().is_none());
    assert_eq!(ItAdmin::query().filter_in("dept_id", vec![7u64]).unwrap().count(&db).await.unwrap(), 3);
    assert_eq!(ItAdmin::query().filter_in("dept_id", Vec::<u64>::new()).unwrap().count(&db).await.unwrap(), 0);
    assert_eq!(ItAdmin::query().filter_contains("username", "ar").unwrap().count(&db).await.unwrap(), 1);

    // delete
    assert_eq!(db.delete::<ItAdmin>(id).await.unwrap(), 1);
    assert!(db.read::<ItAdmin>(id).await.unwrap().is_none());

    db.exec_sql("DROP TABLE it_admin").await.unwrap();
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm --test mysql_integration`
Expected: 编译失败（`Db`、`exec_sql`、`insert` 等不存在）。

- [ ] **Step 3: 在 `src/db.rs` 上追加 CRUD 与 Tx（保留 Task 3 已写的 `connect`/`pool`/`exec_sql`）**

`use` 行补成：

```rust
use crate::error::{OrmError, normalize, validate_ident};
use crate::Model;
use sqlx::mysql::{MySql, MySqlPool};
use sqlx::{Executor, QueryBuilder};
```

（`MySqlPoolOptions` 只在 Task 3 的 `connect` 里用到；若补 `use` 时它变得未使用，按编译器提示调整，别删 `connect`。）

新增 `Tx` 结构体：

```rust
/// 事务；由 `Db::begin` 创建，`commit`/`rollback` 消费自身。
pub struct Tx {
    tx: sqlx::Transaction<'static, MySql>,
}
```

在 `impl Db` 里追加：

```rust
    pub async fn insert<T: Model>(&self, m: &mut T) -> Result<u64, OrmError> {
        insert_with(&self.pool, m).await
    }

    pub async fn read<T: Model>(&self, id: u64) -> Result<Option<T>, OrmError> {
        read_with(&self.pool, id).await
    }

    pub async fn update<T: Model>(&self, m: &T) -> Result<u64, OrmError> {
        update_with(&self.pool, m).await
    }

    pub async fn delete<T: Model>(&self, id: u64) -> Result<u64, OrmError> {
        delete_with(&self.pool, id).await
    }

    pub async fn begin(&self) -> Result<Tx, OrmError> {
        Ok(Tx { tx: self.pool.begin().await.map_err(normalize)? })
    }
}

impl Tx {
    pub async fn commit(self) -> Result<(), OrmError> {
        self.tx.commit().await.map_err(normalize)
    }

    pub async fn rollback(self) -> Result<(), OrmError> {
        self.tx.rollback().await.map_err(normalize)
    }

    pub async fn exec_sql(&mut self, sql: &str) -> Result<u64, OrmError> {
        sqlx::query(sql)
            .execute(&mut *self.tx)
            .await
            .map_err(normalize)
            .map(|r| r.rows_affected())
    }

    pub async fn insert<T: Model>(&mut self, m: &mut T) -> Result<u64, OrmError> {
        insert_with(&mut *self.tx, m).await
    }

    pub async fn read<T: Model>(&mut self, id: u64) -> Result<Option<T>, OrmError> {
        read_with(&mut *self.tx, id).await
    }

    pub async fn update<T: Model>(&mut self, m: &T) -> Result<u64, OrmError> {
        update_with(&mut *self.tx, m).await
    }

    pub async fn delete<T: Model>(&mut self, id: u64) -> Result<u64, OrmError> {
        delete_with(&mut *self.tx, id).await
    }
}

/// 非自增列名列表。
fn insert_columns<T: Model>() -> Vec<&'static str> {
    T::META.columns.iter().filter(|c| !c.auto).map(|c| c.name).collect()
}

/// 通用执行体：pool 与事务共用。`&MySqlPool` 与 `&mut MySqlConnection` 都满足 Executor。
pub(crate) async fn insert_with<'e, E, T>(ex: E, m: &mut T) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    validate_ident(meta.table)?;
    let cols = insert_columns::<T>();
    if cols.is_empty() {
        return Err(OrmError::Unsupported(format!("模型 {} 没有可插入的列", meta.table)));
    }
    let mut qb = QueryBuilder::<MySql>::new(format!(
        "INSERT INTO {} ({}) VALUES (",
        meta.table,
        cols.join(", ")
    ));
    qb = m.bind_insert(qb);
    qb.push(")");
    let res = qb.build().execute(ex).await.map_err(normalize)?;
    let id = res.last_insert_id();
    if meta.columns.iter().any(|c| c.auto) {
        m.set_auto_pk(id);
    }
    Ok(id)
}

pub(crate) async fn read_with<'e, E, T>(ex: E, id: u64) -> Result<Option<T>, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 read", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let sql = format!("SELECT * FROM {} WHERE {} = ?", meta.table, pk);
    let row = sqlx::query(&sql).bind(id).fetch_optional(ex).await.map_err(normalize)?;
    match row {
        Some(r) => T::from_row(&r).map(Some).map_err(OrmError::Sqlx),
        None => Ok(None),
    }
}

pub(crate) async fn update_with<'e, E, T>(ex: E, m: &T) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 update", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let mut qb = QueryBuilder::<MySql>::new(format!("UPDATE {} ", meta.table));
    qb = m.bind_update(qb);
    let res = qb.build().execute(ex).await.map_err(normalize)?;
    Ok(res.rows_affected())
}

pub(crate) async fn delete_with<'e, E, T>(ex: E, id: u64) -> Result<u64, OrmError>
where
    E: Executor<'e, Database = MySql>,
    T: Model,
{
    let meta = T::META;
    let pk = meta
        .pk
        .ok_or_else(|| OrmError::Unsupported(format!("模型 {} 没有主键，不能用 delete", meta.table)))?;
    validate_ident(meta.table)?;
    validate_ident(pk)?;
    let sql = format!("DELETE FROM {} WHERE {} = ?", meta.table, pk);
    let res = sqlx::query(&sql).bind(id).execute(ex).await.map_err(normalize)?;
    Ok(res.rows_affected())
}
```

- [ ] **Step 4: 改 `src/lib.rs`**

把 Task 3 写的 `pub use db::Db;` 改为 `pub use db::{Db, Tx};`（`pub mod db;` 已存在）。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p bee_orm`（带 DSN 时 `mysql_integration::crud_and_query_flow` 必须 PASS）
Expected: 全绿。若报 Executor 生命周期错误，把 `insert_with(&self.pool, m)` 改成 `insert_with(&*self.pool, m)`（显式 reborrow）再试。

- [ ] **Step 6: Commit**

```bash
git add crates/bee_orm/src/db.rs crates/bee_orm/src/lib.rs crates/bee_orm/tests/mysql_integration.rs
git commit -m "feat(bee_orm): Db/Tx 连接池与 CRUD（insert 回填自增、read/update/delete、唯一键归一化）"
```

---

### Task 5: M2M 关联三方法

**Files:**
- Modify: `crates/bee_orm/src/db.rs`（`impl Db` 加三个方法）
- Modify: `crates/bee_orm/tests/mysql_integration.rs`（追加测试）

- [ ] **Step 1: 追加失败测试**

`tests/mysql_integration.rs` 末尾追加：

```rust
/// 连接表：无主键、无自增。
#[derive(Model)]
#[bee(table = "it_admin_role")]
pub struct ItAdminRole {
    pub admin_id: u64,
    pub role_id: u64,
}

#[tokio::test]
async fn relations_flow() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    assert_eq!(ItAdminRole::META.pk, None);
    db.exec_sql("DROP TABLE IF EXISTS it_admin_role").await.unwrap();
    db.exec_sql(
        "CREATE TABLE it_admin_role (
           admin_id BIGINT UNSIGNED NOT NULL,
           role_id BIGINT UNSIGNED NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB",
    )
    .await
    .unwrap();

    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[10, 20, 30]).await.unwrap();
    let mut got = db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap();
    got.sort();
    assert_eq!(got, vec![10, 20, 30]);

    // set = 删旧插新（事务内）
    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[20, 40]).await.unwrap();
    let mut got = db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap();
    got.sort();
    assert_eq!(got, vec![20, 40]);

    // 空集合 = 清空
    db.set_relations("it_admin_role", ("admin_id", 1), "role_id", &[]).await.unwrap();
    assert!(db.get_relations("it_admin_role", ("admin_id", 1), "role_id").await.unwrap().is_empty());

    // 其他 owner 不受影响
    db.set_relations("it_admin_role", ("admin_id", 2), "role_id", &[10]).await.unwrap();
    assert_eq!(db.del_relations("it_admin_role", "admin_id", 2).await.unwrap(), 1);

    // 非法标识符拒绝
    assert!(matches!(
        db.set_relations("it_admin_role; DROP TABLE x", ("admin_id", 1), "role_id", &[1]).await,
        Err(OrmError::InvalidField(_))
    ));

    db.exec_sql("DROP TABLE it_admin_role").await.unwrap();
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm --test mysql_integration relations_flow`
Expected: 编译失败（`set_relations` 等不存在）。

- [ ] **Step 3: 实现**

`src/db.rs` 的 `impl Db` 中追加：

```rust
    /// 重设关联：事务内「删旧 + 逐条插入新值」。空集合 = 清空。
    /// `owner` 是 (关联列, 值)，`target` 是对端列名（如 admin_id/role_id、role_id/menu_id）。
    pub async fn set_relations(
        &self,
        table: &str,
        owner: (&str, u64),
        target: &str,
        ids: &[u64],
    ) -> Result<(), OrmError> {
        validate_ident(table)?;
        validate_ident(owner.0)?;
        validate_ident(target)?;
        let mut tx = self.pool.begin().await.map_err(normalize)?;
        sqlx::query(&format!("DELETE FROM {} WHERE {} = ?", table, owner.0))
            .bind(owner.1)
            .execute(&mut *tx)
            .await
            .map_err(normalize)?;
        if !ids.is_empty() {
            let sql = format!("INSERT INTO {} ({}, {}) VALUES (?, ?)", table, owner.0, target);
            for id in ids {
                sqlx::query(&sql)
                    .bind(owner.1)
                    .bind(*id)
                    .execute(&mut *tx)
                    .await
                    .map_err(normalize)?;
            }
        }
        tx.commit().await.map_err(normalize)?;
        Ok(())
    }

    /// 取关联目标 id 列表。
    pub async fn get_relations(
        &self,
        table: &str,
        owner: (&str, u64),
        target: &str,
    ) -> Result<Vec<u64>, OrmError> {
        validate_ident(table)?;
        validate_ident(owner.0)?;
        validate_ident(target)?;
        let sql = format!("SELECT {} FROM {} WHERE {} = ?", target, table, owner.0);
        let rows: Vec<(u64,)> = sqlx::query_as(&sql)
            .bind(owner.1)
            .fetch_all(&self.pool)
            .await
            .map_err(normalize)?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// 按列删关联，返回删除行数。
    pub async fn del_relations(&self, table: &str, col: &str, id: u64) -> Result<u64, OrmError> {
        validate_ident(table)?;
        validate_ident(col)?;
        let sql = format!("DELETE FROM {} WHERE {} = ?", table, col);
        let res = sqlx::query(&sql).bind(id).execute(&self.pool).await.map_err(normalize)?;
        Ok(res.rows_affected())
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm --test mysql_integration`
Expected: `crud_and_query_flow`、`relations_flow` 全 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/bee_orm/src/db.rs crates/bee_orm/tests/mysql_integration.rs
git commit -m "feat(bee_orm): M2M 关联三方法（set/get/del_relations，set 走事务）"
```

---

### Task 6: syncdb（DDL 生成 + 表结构同步）

**Files:**
- Create: `crates/bee_orm/src/syncdb.rs`
- Modify: `crates/bee_orm/src/lib.rs`（`pub mod syncdb;`）
- Modify: `crates/bee_orm/tests/sql_gen.rs`（DDL 单测）
- Modify: `crates/bee_orm/tests/mysql_integration.rs`（同步流程测试）

- [ ] **Step 1: 追加失败测试（`tests/sql_gen.rs`）**

```rust
#[test]
fn create_table_ddl() {
    let ddl = bee_orm::syncdb::create_table_sql(&SgAdmin::META);
    assert!(ddl.contains("CREATE TABLE IF NOT EXISTS sg_admin"));
    assert!(ddl.contains("id BIGINT UNSIGNED AUTO_INCREMENT NOT NULL"));
    assert!(ddl.contains("PRIMARY KEY (id)"));
    assert!(ddl.contains("username VARCHAR(255) NOT NULL DEFAULT ''"));
    assert!(ddl.contains("UNIQUE KEY uk_sg_admin_username (username)"));
    assert!(ddl.contains("status TINYINT NOT NULL DEFAULT 0"));
    assert!(ddl.contains("note VARCHAR(255)")); // 可空，不加 NOT NULL
    assert!(!ddl.contains("note VARCHAR(255) NOT NULL"));
    assert!(ddl.contains("bio TEXT NOT NULL"));
    assert!(ddl.contains("KEY idx_sg_admin_dept_id (dept_id)"));
    assert!(ddl.contains("created_at DATETIME NOT NULL"));
    assert!(ddl.contains("ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"));
}

#[test]
fn add_column_ddl() {
    let ddl = bee_orm::syncdb::add_column_sql("sg_admin", &SgAdmin::META.columns[2]);
    assert_eq!(ddl, "ALTER TABLE sg_admin ADD COLUMN status TINYINT NOT NULL DEFAULT 0");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p bee_orm --test sql_gen`
Expected: 编译失败（`bee_orm::syncdb` 不存在）。

- [ ] **Step 3: 写 `src/syncdb.rs`**

```rust
// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! syncdb：模型元数据 ↔ information_schema 对比，只增不删（Safe 模式）。
use crate::db::Db;
use crate::error::{OrmError, normalize, validate_ident};
use crate::meta::{ColumnMeta, ColumnType, ModelMeta, SyncdbMode};
use std::collections::HashSet;

pub fn index_name(table: &str, col: &str) -> String {
    format!("idx_{table}_{col}")
}

pub fn unique_index_name(table: &str, col: &str) -> String {
    format!("uk_{table}_{col}")
}

/// 单列 DDL 类型 + 约束后缀。
fn column_def(c: &ColumnMeta) -> String {
    let mut s = format!("{} {}", c.name, column_type_sql(c));
    if c.auto {
        s.push_str(" AUTO_INCREMENT NOT NULL");
        return s;
    }
    if c.nullable {
        return s;
    }
    match c.ty {
        // TEXT/JSON/DATETIME 不给默认值（JSON 列 MySQL 8 不允许字面量默认）
        ColumnType::String if c.text | c.ty == ColumnType::String && c.text => s.push_str(" NOT NULL"),
        _ => {}
    }
    s
}

fn column_type_sql(c: &ColumnMeta) -> String {
    match c.ty {
        ColumnType::U64 => "BIGINT UNSIGNED".into(),
        ColumnType::I64 => "BIGINT".into(),
        ColumnType::U32 => "INT UNSIGNED".into(),
        ColumnType::I32 => "INT".into(),
        ColumnType::I16 => "SMALLINT".into(),
        ColumnType::I8 => "TINYINT".into(),
        ColumnType::Bool => "TINYINT(1)".into(),
        ColumnType::String => {
            if c.text {
                "TEXT".into()
            } else {
                format!("VARCHAR({})", c.len.unwrap_or(255))
            }
        }
        ColumnType::F64 => "DOUBLE".into(),
        ColumnType::DateTime => "DATETIME".into(),
        ColumnType::Json => "JSON".into(),
    }
}

fn column_def(c: &ColumnMeta) -> String {
    let mut s = format!("{} {}", c.name, column_type_sql(c));
    if c.auto {
        s.push_str(" AUTO_INCREMENT NOT NULL");
    } else if c.nullable {
        // 可空列不加约束
    } else {
        match c.ty {
            ColumnType::String if c.text => s.push_str(" NOT NULL"),
            ColumnType::String => s.push_str(" NOT NULL DEFAULT ''"),
            ColumnType::Json | ColumnType::DateTime => s.push_str(" NOT NULL"),
            _ => s.push_str(" NOT NULL DEFAULT 0"),
        }
    }
    s
}

pub fn create_table_sql(m: &ModelMeta) -> String {
    let mut parts: Vec<String> = m.columns.iter().map(column_def).collect();
    if let Some(pk) = m.pk {
        parts.push(format!("PRIMARY KEY ({pk})"));
    }
    for c in m.columns.iter().filter(|c| c.unique) {
        parts.push(format!("UNIQUE KEY {} ({})", unique_index_name(m.table, c.name), c.name));
    }
    for c in m.columns.iter().filter(|c| c.index && !c.unique) {
        parts.push(format!("KEY {} ({})", index_name(m.table, c.name), c.name));
    }
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n  {}\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        m.table,
        parts.join(",\n  ")
    )
}

pub fn add_column_sql(table: &str, c: &ColumnMeta) -> String {
    format!("ALTER TABLE {table} ADD COLUMN {}", column_def(c))
}

pub fn create_index_sql(table: &str, c: &ColumnMeta) -> String {
    if c.unique {
        format!("CREATE UNIQUE INDEX {} ON {} ({})", unique_index_name(table, c.name), table, c.name)
    } else {
        format!("CREATE INDEX {} ON {} ({})", index_name(table, c.name), table, c.name)
    }
}

impl Db {
    /// 把一批模型同步到当前库（Safe：建表 / 加列 / 补索引，绝不删改）。
    /// 返回实际执行的 DDL 列表（空 = 已是最新）。
    pub async fn syncdb(&self, metas: &[ModelMeta], mode: SyncdbMode) -> Result<Vec<String>, OrmError> {
        if mode == SyncdbMode::Force {
            return Err(OrmError::Unsupported("SyncdbMode::Force 未实现（本期 Safe）".into()));
        }
        let mut executed = Vec::new();
        for m in metas {
            validate_ident(m.table)?;
            let exists: Option<(String,)> = sqlx::query_as(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_optional(self.pool())
            .await
            .map_err(normalize)?;

            if exists.is_none() {
                let ddl = create_table_sql(m);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
                continue;
            }

            let cols: Vec<(String,)> = sqlx::query_as(
                "SELECT column_name FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_all(self.pool())
            .await
            .map_err(normalize)?;
            let have: HashSet<String> = cols.into_iter().map(|c| c.0.to_ascii_lowercase()).collect();
            for c in m.columns.iter().filter(|c| !have.contains(&c.name.to_ascii_lowercase())) {
                if c.auto {
                    return Err(OrmError::Unsupported(format!(
                        "表 {} 缺自增列 {}，Safe 模式不自动补（请人工处理）",
                        m.table, c.name
                    )));
                }
                let ddl = add_column_sql(m.table, c);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
            }

            let idx: Vec<(String,)> = sqlx::query_as(
                "SELECT DISTINCT index_name FROM information_schema.statistics \
                 WHERE table_schema = DATABASE() AND table_name = ?",
            )
            .bind(m.table)
            .fetch_all(self.pool())
            .await
            .map_err(normalize)?;
            let have_idx: HashSet<String> = idx.into_iter().map(|i| i.0.to_ascii_lowercase()).collect();
            for c in m.columns.iter().filter(|c| c.unique || c.index) {
                let name = if c.unique { unique_index_name(m.table, c.name) } else { index_name(m.table, c.name) };
                if have_idx.contains(&name.to_ascii_lowercase()) {
                    continue;
                }
                let ddl = create_index_sql(m.table, c);
                self.exec_sql(&ddl).await?;
                executed.push(ddl);
            }
        }
        Ok(executed)
    }
}
```

- [ ] **Step 4: 追加集成测试（`tests/mysql_integration.rs`）**

```rust
#[derive(Model)]
#[bee(table = "it_sync", pk = "id")]
pub struct ItSyncV1 {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub name: String,
    pub status: i8,
}

#[derive(Model)]
#[bee(table = "it_sync", pk = "id")]
pub struct ItSyncV2 {
    #[bee(auto)] pub id: u64,
    #[bee(unique)] pub name: String,
    pub status: i8,
    pub remark: String,
    #[bee(index)] pub dept_id: u64,
}

#[tokio::test]
async fn syncdb_creates_alters_and_is_idempotent() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_sync").await.unwrap();

    let ddl = db.syncdb(&[ItSyncV1::META], SyncdbMode::Safe).await.unwrap();
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].starts_with("CREATE TABLE IF NOT EXISTS it_sync"));

    // 幂等：再跑一次不产生 DDL
    assert!(db.syncdb(&[ItSyncV1::META], SyncdbMode::Safe).await.unwrap().is_empty());

    // 模型加列 → ALTER 补列 + 补索引
    let ddl = db.syncdb(&[ItSyncV2::META], SyncdbMode::Safe).await.unwrap();
    assert!(ddl.iter().any(|d| d.contains("ADD COLUMN remark VARCHAR(255) NOT NULL DEFAULT ''")));
    assert!(ddl.iter().any(|d| d.contains("CREATE INDEX idx_it_sync_dept_id")));

    // 新列可用
    let mut r = ItSyncV2 { id: 0, name: "x".into(), status: 1, remark: "hi".into(), dept_id: 3 };
    assert!(db.insert(&mut r).await.unwrap() > 0);

    // Force 未实现
    assert!(matches!(
        db.syncdb(&[ItSyncV1::META], SyncdbMode::Force).await,
        Err(OrmError::Unsupported(_))
    ));

    db.exec_sql("DROP TABLE it_sync").await.unwrap();
}
```

`tests/mysql_integration.rs` 顶部 import 改为 `use bee_orm::{Db, Model, OrmError, SyncdbMode};`

- [ ] **Step 5: 跑测试**

Run: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/bee_orm/src/syncdb.rs crates/bee_orm/src/lib.rs crates/bee_orm/tests/
git commit -m "feat(bee_orm): syncdb 迁移（Safe 建表/加列/补索引，幂等）"
```

---

### Task 7: 事务集成测试 + 全量收尾

**Files:**
- Modify: `crates/bee_orm/tests/mysql_integration.rs`（事务测试）
- Modify: `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md`（事务 API 措辞与实际一致：`begin/commit/rollback`）

- [ ] **Step 1: 追加失败测试**

```rust
#[derive(Model)]
#[bee(table = "it_tx", pk = "id")]
pub struct ItTx {
    #[bee(auto)] pub id: u64,
    pub name: String,
}

#[tokio::test]
async fn transaction_commit_and_rollback() {
    let Some(dsn) = dsn() else { return };
    let db = Db::connect(&dsn).await.unwrap();
    db.exec_sql("DROP TABLE IF EXISTS it_tx").await.unwrap();
    db.exec_sql(
        "CREATE TABLE it_tx (
           id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
           name VARCHAR(255) NOT NULL DEFAULT ''
         ) ENGINE=InnoDB",
    )
    .await
    .unwrap();

    let mut tx = db.begin().await.unwrap();
    let mut a = ItTx { id: 0, name: "committed".into() };
    tx.insert(&mut a).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(a.id, 1);

    let mut tx = db.begin().await.unwrap();
    let mut b = ItTx { id: 0, name: "rolled".into() };
    tx.insert(&mut b).await.unwrap();
    tx.rollback().await.unwrap();

    let rows = ItTx::query().fetch_all(&db).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "committed");

    db.exec_sql("DROP TABLE it_tx").await.unwrap();
}
```

- [ ] **Step 2: 跑测试确认通过（Tx 已在 Task 4 实现）**

Run: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm --test mysql_integration transaction_commit_and_rollback`
Expected: PASS。若失败说明事务实现有 bug，修 `db.rs` 而不是改测试。

- [ ] **Step 3: 设计文档对齐**

`docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md` §4.3 里的
`db.transaction(|tx| ...)` 改为：

```rust
let mut tx = db.begin().await?;   // Tx 上同样有 insert/update/delete/exec_sql
tx.insert(&mut admin).await?;
tx.commit().await?;               // 或 tx.rollback().await?
```

（Rust 里把 `&mut Tx` 交给异步闭包会撞生命周期/HRTB，`begin/commit/rollback` 是等价且更简单的形式。）

- [ ] **Step 4: 全量验证**

Run: `cargo test -p bee_orm -p bee_orm_macro && cargo build --workspace`
Expected: bee_orm 全部测试 PASS（无 DSN 时集成测试跳过）；工作区编译通过。

Run（带 DSN，完整验证）: `BEE_ORM_TEST_DSN='...' cargo test -p bee_orm`
Expected: 4 个集成测试 + 单测全 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/bee_orm/tests/mysql_integration.rs docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md
git commit -m "test(bee_orm): 事务提交/回滚集成测试；设计文档事务 API 对齐 begin/commit/rollback"
```

---

### Task 8: bee_cli 模板与文档适配（派生宏路径约束）

**背景:** `#[derive(Model)]` 展开的代码引用 `bee_orm::Model / bee_orm::ModelMeta / bee_orm::__private::…`，因此**使用派生宏的模块必须让 `bee_orm` 这个名字在作用域内**（直接依赖 `bee_orm`，或 `use bee_rust::bee_orm::{self, Model};`）。`bee_cli` 现在生成的模板是 `use bee_rust::bee_orm::Model;`——只导入了 `Model`，用它生成的模型会编译失败。

**Files:**
- Modify: `crates/bee_cli/src/lib.rs`（`generate_model` 的模板字符串 + 对应测试断言，约 132 行与 460 行）
- Modify: `crates/bee_orm/src/lib.rs`（`Model` trait 的文档注释里写明这条约束）

- [ ] **Step 1: 改模板**

模板里的 `use bee_rust::bee_orm::Model;` 改为 `use bee_rust::bee_orm::{self, Model};`；同时更新 `crates/bee_cli/src/lib.rs` 中断言 `content.contains("use bee_rust::bee_orm::Model;")` 的测试为断言新字符串（`assert!(content.contains("use bee_rust::bee_orm::{self, Model};"))`）。

- [ ] **Step 2: 文档约束**

`crates/bee_orm/src/lib.rs` 的 `Model` trait 文档加一段：

```rust
/// # 使用约束
///
/// 派生宏展开的代码引用 `bee_orm::…` 路径，因此使用 `#[derive(Model)]` 的模块里
/// `bee_orm` 必须在作用域内：要么本 crate 直接依赖 `bee_orm`（`use bee_orm::Model;`），
/// 要么 `use bee_rust::bee_orm::{self, Model};`。
```

- [ ] **Step 3: 验证**

Run: `cargo test -p bee_cli && cargo build -p bee_orm`
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add crates/bee_cli/src/lib.rs crates/bee_orm/src/lib.rs
git commit -m "fix(bee_cli): 模型模板导入 bee_orm 本体，适配派生宏的路径约束"
```

---

## 明知取舍（本计划范围内不做）

- **Postgres/SQLite**：占位符 `?` → `$n` 改写与各自 FromRow 未做；`sqlx` feature 已就位，加库时补。
- **复合主键**：`ModelMeta` 只支持单列主键；连接表（admin_role 等）走 `exec_sql` 建表 + `set_relations` 操作。
- **Force 模式 syncdb**：明确返回 `Unsupported`，不悄悄删列。
- **参数类型**：QuerySet 参数统一按字符串绑定（MySQL 常量折叠后仍走索引）；需要类型精确绑定的场景用 CRUD/绑定 API。
- **`#[bee(len)]` 用未知类型**：编译期报错（宏里 `syn::Error`），不是运行期。

## Self-Review 记录

- 覆盖设计文档 §4.1–4.7 全部条目：依赖（T1）、宏（T2）、Db API + filter_in/filter_raw/page（T3/T4）、类型映射（T1 meta + T6 DDL）、syncdb（T6）、错误（T1 normalize）、测试（T2/T3 单测 + T4/T5/T6/T7 集成）。
- 事务 API 由设计稿的 `db.transaction(闭包)` 调整为 `begin/commit/rollback`（Rust 借用限制），T7 同步改设计文档。
- 表名默认值由旧的 `小写+s` 改为 snake_case + `s`（`User` → `users` 不变以兼容框架自带测试；`UtAdmin` → `ut_admins`），T2 测试与框架自带 `orm_tests.rs` 共同固化。
- `filter_eq/gt/lt/contains` 值参数由 `impl Into<String>` 放宽到 `impl ToString`（`filter_eq("status", 1)` 可用），向后兼容。
- 计划内所有类型/方法名一致：`Db::{connect,pool,exec_sql,insert,read,update,delete,begin,set_relations,get_relations,del_relations,syncdb}`、`Tx::{commit,rollback,exec_sql,insert,read,update,delete}`、`QuerySet::{page,count_sql,fetch_all,fetch_one,count,fetch_page,filter_in,filter_raw}`。
