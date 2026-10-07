// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "sqlite")]
//! §47 tester falsification of the m2m assembly: `related_for` must stay
//! index-aligned with its input across both `MAX_BIND_PARAMS` chunk boundaries
//! (the local-key scan and the foreign-id load), must not collapse duplicate
//! inputs, and must answer `Ok(vec![])` for an empty slice. The first test
//! crosses both boundaries for real — 1500 locals, >999 distinct foreign ids —
//! with empty and multi-element groups inside the oversized chunk.
use std::collections::BTreeSet;

use bee_orm::pool::sqlite::Pool;
use bee_orm::{Model, Value, m2m, migrate};

fn pool() -> Pool {
    Pool::connect(":memory:", 1).unwrap()
}

/// The crate keeps its constant `pub(crate)` (lib.rs), so the black-box test
/// pins the number — same as `orm_features.rs`'s 999-parameter floor.
const MAX_BIND_PARAMS: usize = 999;
const N: i64 = 1500; // > MAX_BIND_PARAMS on both sides of the assembly

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "asm_tags")]
struct AsmTag {
    #[bee(pk, auto)]
    id: i64,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "asm_users")]
#[bee(m2m(AsmTag))]
struct AsmUser {
    #[bee(pk, auto)]
    id: i64,
    name: String,
}

/// User `i`'s expected tags: every 5th has none, every 3rd has `{1, 2}`, the
/// rest carry two distinct tags of their own (`i` and `10_000 + i`) — so the
/// distinct foreign ids alone exceed the bind limit.
fn expected_tags(i: i64) -> Vec<i64> {
    if i % 5 == 0 {
        vec![]
    } else if i % 3 == 0 {
        vec![1, 2]
    } else {
        vec![i, 10_000 + i]
    }
}

fn sorted_ids(groups: Vec<Vec<AsmTag>>) -> Vec<Vec<i64>> {
    groups
        .into_iter()
        .map(|group| {
            let mut ids: Vec<i64> = group.into_iter().map(|tag| tag.id).collect();
            ids.sort_unstable();
            ids
        })
        .collect()
}

#[tokio::test]
async fn related_for_crosses_both_bind_limits_with_alignment() {
    let pool = pool();
    // Target first: the join table REFERENCES both sides (§43 ordering rule).
    migrate::create_table::<AsmTag, _>(&pool).await.unwrap();
    migrate::create_table::<AsmUser, _>(&pool).await.unwrap();

    let mut needed: BTreeSet<i64> = [1, 2].into_iter().collect();
    for i in 1..=N {
        needed.extend(expected_tags(i));
    }
    for tag in &needed {
        pool.execute("INSERT INTO asm_tags (id) VALUES (?)", &[Value::Int(*tag)]).await.unwrap();
    }
    assert!(
        needed.len() > MAX_BIND_PARAMS,
        "the foreign side must cross the limit, has {}",
        needed.len()
    );

    let mut users = Vec::new();
    for i in 1..=N {
        pool.execute(
            "INSERT INTO asm_users (id, name) VALUES (?, ?)",
            &[Value::Int(i), Value::Text(format!("u{i}"))],
        )
        .await
        .unwrap();
        let user = AsmUser { id: i, name: format!("u{i}") };
        for tag in expected_tags(i) {
            assert_eq!(m2m::attach(&pool, &user, &AsmTag { id: tag }).await.unwrap(), 1);
        }
        users.push(user);
    }
    assert!(users.len() > MAX_BIND_PARAMS, "the local side must cross the limit");

    let actual = sorted_ids(m2m::related_for(&pool, &users).await.unwrap());
    let expected: Vec<Vec<i64>> = (1..=N).map(expected_tags).collect();
    assert_eq!(actual.len(), expected.len());
    let mismatches: Vec<_> = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (actual, expected))| actual != expected)
        .take(5)
        .collect();
    assert!(
        mismatches.is_empty(),
        "index/group mismatches (index, actual, expected): {mismatches:?}"
    );

    // `related` chunks its own id list the same way: one local holding more
    // relations than one `IN` can carry must come back whole, not truncated.
    let big = AsmUser { id: 9999, name: "big".into() };
    pool.execute("INSERT INTO asm_users (id, name) VALUES (9999, 'big')", &[]).await.unwrap();
    let attached: Vec<i64> = needed.iter().copied().take(MAX_BIND_PARAMS + 1).collect();
    for tag in &attached {
        assert_eq!(m2m::attach(&pool, &big, &AsmTag { id: *tag }).await.unwrap(), 1);
    }
    let related = m2m::related::<AsmUser, AsmTag, _>(&pool, &big).await.unwrap();
    let mut got: Vec<i64> = related.iter().map(|tag| tag.id).collect();
    got.sort_unstable();
    let mismatches: Vec<_> = got.iter().zip(&attached).filter(|(a, e)| a != e).take(5).collect();
    assert!(
        got.len() == attached.len() && mismatches.is_empty(),
        "related chunking: {} of {} rows, mismatches {mismatches:?}",
        got.len(),
        attached.len()
    );
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "asm2_tags")]
struct Asm2Tag {
    #[bee(pk, auto)]
    id: i64,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[bee(table = "asm2_users")]
#[bee(m2m(Asm2Tag))]
struct Asm2User {
    #[bee(pk, auto)]
    id: i64,
}

#[tokio::test]
async fn related_for_keeps_duplicate_and_empty_inputs_aligned() {
    let pool = pool();
    migrate::create_table::<Asm2Tag, _>(&pool).await.unwrap();
    migrate::create_table::<Asm2User, _>(&pool).await.unwrap();
    for i in 1..=3 {
        pool.execute("INSERT INTO asm2_users (id) VALUES (?)", &[Value::Int(i)]).await.unwrap();
        pool.execute("INSERT INTO asm2_tags (id) VALUES (?)", &[Value::Int(i)]).await.unwrap();
    }
    let (u1, u2, u3) = (Asm2User { id: 1 }, Asm2User { id: 2 }, Asm2User { id: 3 });
    m2m::attach(&pool, &u1, &Asm2Tag { id: 1 }).await.unwrap();
    m2m::attach(&pool, &u1, &Asm2Tag { id: 2 }).await.unwrap();
    m2m::attach(&pool, &u3, &Asm2Tag { id: 1 }).await.unwrap();

    // The closure pins `R = Asm2Tag` for both calls below — `R` lives only in
    // `related_for`'s return type, so inference needs the annotation.
    let ids = |groups: Vec<Vec<Asm2Tag>>| -> Vec<Vec<i64>> {
        groups
            .into_iter()
            .map(|group| {
                let mut ids: Vec<i64> = group.into_iter().map(|tag| tag.id).collect();
                ids.sort_unstable();
                ids
            })
            .collect()
    };

    // Duplicate input, empty group and input order ≠ id order in one slice:
    // the duplicate's group must repeat at its own index, not be deduped away.
    let groups = m2m::related_for(&pool, &[u1.clone(), u2.clone(), u1, u3]).await.unwrap();
    assert_eq!(ids(groups), vec![vec![1, 2], vec![], vec![1, 2], vec![1]]);

    // Empty input: no query, `Ok(vec![])` — the guard runs before any IN list.
    // `R` lives only in the return type, so the empty call needs the turbofish.
    let groups = m2m::related_for::<Asm2User, Asm2Tag, _>(&pool, &[]).await.unwrap();
    assert!(groups.is_empty(), "empty input must produce no groups");
}
