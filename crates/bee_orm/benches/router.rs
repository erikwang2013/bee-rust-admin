// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Routing cost for the FNV-1a [`HashRouter`]: one hash over an Int or Text
//! key, then the database / table ordinal split.

use std::hint::black_box;

use bee_orm::{HashRouter, ShardRouter, Value};
use criterion::{Criterion, criterion_group, criterion_main};

fn routing(c: &mut Criterion) {
    let router = HashRouter::new(4, 8).unwrap();
    let int_key = Value::Int(42);
    let text_key = Value::Text("user:42".into());
    let table = "users";

    let mut group = c.benchmark_group("router");
    group.bench_function("db_index_int", |b| b.iter(|| router.db_index(black_box(&int_key))));
    group.bench_function("db_index_text", |b| b.iter(|| router.db_index(black_box(&text_key))));
    group.bench_function("table_name_int", |b| {
        b.iter(|| router.table_name(black_box(table), black_box(&int_key)))
    });
    group.bench_function("table_name_text", |b| {
        b.iter(|| router.table_name(black_box(table), black_box(&text_key)))
    });
    group.finish();
}

criterion_group!(benches, routing);
criterion_main!(benches);
