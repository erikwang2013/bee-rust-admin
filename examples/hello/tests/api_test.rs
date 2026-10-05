// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//
// End-to-end tests for the hello example server: spawn the real binary on
// a random 127.0.0.1 port (free port probed first, then handed to the app
// via the PORT env var it already reads), and talk plain HTTP/1.1 over a
// raw TcpStream. No test-only code was added to main.rs.

use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Kills and reaps the server process on test exit (drop does not kill).
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reserve a free port, release it, and boot the app on it. The probe-then-
/// rebind race is acceptable for tests: the app exits if the bind fails and
/// the readiness loop detects it.
async fn start_server() -> (Server, String) {
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let child = Command::new(env!("CARGO_BIN_EXE_hello-bee"))
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn hello-bee binary");

    let mut server = Server(child);
    let addr = format!("127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        if server.0.try_wait().unwrap().is_some() {
            panic!("hello-bee exited before accepting connections");
        }
        if tokio::time::Instant::now() > deadline {
            panic!("hello-bee did not start listening within 10s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    (server, addr)
}

/// Send one raw HTTP/1.1 request with `Connection: close` and return the
/// full raw response text.
async fn raw_request(addr: &str, method: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn status_line(raw: &str) -> &str {
    raw.lines().next().unwrap_or("")
}

#[tokio::test]
async fn health_route_serves_200_ok() {
    let (_server, addr) = start_server().await;
    let raw = raw_request(&addr, "GET", "/api/v1/health").await;
    assert!(status_line(&raw).starts_with("HTTP/1.1 200"), "status: {}", status_line(&raw));
    assert!(raw.contains("OK"), "body missing OK: {raw}");
}

#[tokio::test]
async fn unknown_path_returns_404() {
    let (_server, addr) = start_server().await;
    let raw = raw_request(&addr, "GET", "/api/v1/nope").await;
    assert!(status_line(&raw).starts_with("HTTP/1.1 404"), "status: {}", status_line(&raw));

    let raw = raw_request(&addr, "GET", "/definitely/not/a/route").await;
    assert!(status_line(&raw).starts_with("HTTP/1.1 404"), "status: {}", status_line(&raw));
}

#[tokio::test]
async fn wrong_method_on_health_returns_405() {
    let (_server, addr) = start_server().await;
    let raw = raw_request(&addr, "POST", "/api/v1/health").await;
    assert!(status_line(&raw).starts_with("HTTP/1.1 405"), "status: {}", status_line(&raw));
}
