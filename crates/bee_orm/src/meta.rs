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
