// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Env-gated e2e: the store recovers after the server drops its connection.
//!
//! Gated by `BEE_KV_REDIS_DSN` (e.g. `redis://127.0.0.1:6379`), like the other
//! real-server suites: without it the test returns immediately. The store's
//! connection is killed from a *side* client — a raw RESP `CLIENT KILL` on a
//! throwaway socket, so this suite needs no redis dependency of its own — and
//! then polled with a bounded retry: the command that hits the break may fail,
//! the next one must succeed. Recovery is asserted, never an attempt count.

#![cfg(feature = "redis")]

use std::time::Duration;

use bee_kv::{KvStore, RedisStore};

/// The server to test against, or `None` (skip) when the env var is unset.
fn dsn() -> Option<String> {
    std::env::var("BEE_KV_REDIS_DSN").ok().filter(|s| !s.is_empty())
}

/// `host:port` out of a `redis://[user:pass@]host[:port][/db]` DSN — what the
/// side client dials. The gated DSN is a local, unauthenticated one, so the
/// credentials are ignored.
fn host_port(dsn: &str) -> (String, u16) {
    let rest = dsn.split_once("://").map_or(dsn, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().expect("redis port number")),
        None => (authority.to_string(), 6379),
    }
}

/// Kill every ordinary client's connection — the store's included — with a
/// raw `CLIENT KILL TYPE normal` on a throwaway socket. The killer is a normal
/// client too, so it dies along with the rest; the reply is read best-effort
/// and never asserted.
async fn kill_connections(dsn: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (host, port) = host_port(dsn);
    let mut socket = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("failed to open the side client");
    let kill = "*4\r\n$6\r\nCLIENT\r\n$4\r\nKILL\r\n$4\r\nTYPE\r\n$6\r\nnormal\r\n";
    socket.write_all(kill.as_bytes()).await.expect("failed to send CLIENT KILL");
    socket.flush().await.unwrap();
    let mut reply = [0u8; 64];
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut reply)).await;
}

#[tokio::test]
async fn store_recovers_after_its_connection_is_killed() {
    let Some(dsn) = dsn() else {
        return;
    };
    let store = RedisStore::new(&dsn).await.unwrap();
    let key = format!("bee_kv:reconnect:{}", std::process::id());
    store.set(&key, "before").await.unwrap();
    assert_eq!(store.get(&key).await.unwrap().as_deref(), Some("before"));

    kill_connections(&dsn).await;

    // The command that hits the break may error; the next ones hit the
    // reconnecting manager. Poll, but do not claim which attempt succeeds —
    // which one recovers is timing, not contract. (Both lines print only on
    // failure or under `--nocapture`.)
    let mut value = None;
    for attempt in 1..=10 {
        match store.get(&key).await {
            Ok(found) => {
                eprintln!("recovered on attempt {attempt}");
                value = found;
                break;
            }
            Err(e) => {
                eprintln!("attempt {attempt} failed: {e}");
                assert!(attempt < 10, "still no reply after {attempt} attempts");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    assert_eq!(value.as_deref(), Some("before"), "value lost across the reconnect");

    store.del(&key).await.unwrap();
}
