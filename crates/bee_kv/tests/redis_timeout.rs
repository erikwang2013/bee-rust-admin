// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! The configurable command bound, proven without a Redis server: a local
//! socket answers connection setup and then swallows a `GET`.

#![cfg(feature = "redis")]

use std::time::{Duration, Instant};

use bee_kv::{KvError, KvStore, RedisOptions, RedisStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Read one complete RESP array of bulk strings from `buf`, returning its
/// arguments and the number of bytes consumed. `None` when more bytes are
/// needed.
fn next_command(buf: &[u8]) -> Option<(Vec<String>, usize)> {
    let line = |from: usize| buf[from..].windows(2).position(|w| w == b"\r\n").map(|p| p + from);
    let header_end = line(0)?;
    if buf.first() != Some(&b'*') {
        return None;
    }
    let count: usize = std::str::from_utf8(&buf[1..header_end]).ok()?.parse().ok()?;
    let mut pos = header_end + 2;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let end = line(pos)?;
        if buf.get(pos) != Some(&b'$') {
            return None;
        }
        let len: usize = std::str::from_utf8(&buf[pos + 1..end]).ok()?.parse().ok()?;
        let start = end + 2;
        let stop = start.checked_add(len)?;
        if buf.len() < stop + 2 {
            return None;
        }
        args.push(String::from_utf8_lossy(&buf[start..stop]).into_owned());
        pos = stop + 2;
    }
    Some((args, pos))
}

#[tokio::test]
async fn a_server_that_never_replies_to_get_fails_within_the_response_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // The client is connected; the listener's job is done.
        drop(listener);
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            buf.extend_from_slice(&chunk[..read]);
            while let Some((args, used)) = next_command(&buf) {
                buf.drain(..used);
                match args.first().map(String::as_str) {
                    // The point of the test: this reply never arrives.
                    Some("GET") => {}
                    Some("PING") => {
                        // deadpool recycles with `PING <n>` and expects the
                        // argument echoed back; the eager check is a bare PING.
                        let reply = args.get(1).map_or("PONG", String::as_str);
                        let _ = socket.write_all(format!("+{reply}\r\n").as_bytes()).await;
                    }
                    // CLIENT SETINFO on connection setup, UNWATCH on recycle.
                    _ => {
                        let _ = socket.write_all(b"+OK\r\n").await;
                    }
                }
            }
        }
    });

    let store = RedisStore::with_options(
        &format!("redis://{addr}"),
        RedisOptions { max_size: 1, response_timeout: Duration::from_millis(50) },
    )
    .await
    .expect("the fake server answers the eager PING");

    let start = Instant::now();
    let err = store.get("missing").await.expect_err("GET must not hang");
    assert!(matches!(err, KvError::OperationFailed(_)), "{err:?}");
    assert!(start.elapsed() < Duration::from_secs(1), "GET was not bounded: {:?}", start.elapsed());

    server.abort();
}
