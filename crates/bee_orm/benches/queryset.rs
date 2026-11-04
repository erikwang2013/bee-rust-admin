// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! SQL building cost for a filter/order/limit [`QuerySet`].

use std::hint::black_box;

use bee_orm::{Model, QuerySet, Result, Row, Value};
use criterion::{Criterion, criterion_group, criterion_main};

struct User;

impl Model for User {
    fn table_name() -> &'static str {
        "users"
    }

    fn pk_column() -> &'static str {
        "id"
    }

    fn from_row(_row: &Row) -> Result<Self> {
        Ok(User)
    }

    fn insert_values(&self) -> Vec<(&'static str, Value)> {
        Vec::new()
    }

    fn pk_value(&self) -> Value {
        Value::Int(1)
    }

    fn update_values(&self) -> Vec<(&'static str, Value)> {
        Vec::new()
    }
}

fn to_sql(c: &mut Criterion) {
    let query = QuerySet::<User>::new("users")
        .filter_eq("age", 30)
        .unwrap()
        .filter_gt("score", 100)
        .unwrap()
        .order_by("id DESC")
        .limit(10)
        .offset(20);

    c.bench_function("queryset_to_sql", |b| b.iter(|| black_box(&query).to_sql()));
}

criterion_group!(benches, to_sql);
criterion_main!(benches);
