// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "postgres")]
//! PostgreSQL pool tests that need no server and no runtime: `Pool::connect`
//! only parses the DSN and builds a lazy pool — no connection is attempted
//! until `get()`.
use bee_orm::pool::postgres::Pool;

#[test]
fn postgres_status_is_empty_until_first_get() {
    let pool = Pool::connect("postgres://user:pass@localhost:5432/app", 7).unwrap();
    let status = pool.status();
    assert_eq!(status.max_size, 7);
    assert_eq!(status.size, 0); // connections open lazily
    assert_eq!(status.available, 0);
    assert_eq!(status.waiting, 0);
}
