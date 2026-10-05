// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! End-to-end tests for the hello-bee example.
//!
//! Three flows, mirroring what a browser would do in a real deployment:
//!   1. Page-level: the template pipeline renders `test.html` (the only
//!      HTML asset in this workspace) with correct placeholder
//!      substitution and HTML escaping.
//!   2. Full chain: spawn the real `hello-bee` server binary (which runs
//!      `bee_rust::init()` and the axum router), then speak raw HTTP over
//!      TCP and assert status + body.
//!   3. Robustness: malformed requests and overlong URIs must not panic
//!      the server; it must keep serving normal requests afterwards.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bee_rust::bee_template::{TemplateEngine, TemplateError};

// ---------------------------------------------------------------------------
// Flow 1: template pipeline (page-level)
// ---------------------------------------------------------------------------

/// The only real HTML asset in the workspace, shared by the router crate.
fn fixture_template_dir() -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/bee_router/tests/fixtures/templates");
    let dir = dir.canonicalize().unwrap_or_else(|e| panic!("fixture dir missing: {e}"));
    assert!(dir.join("test.html").is_file(), "fixture test.html not found");
    dir
}

#[test]
fn template_renders_fixture_with_placeholders() {
    // Sanity: we really read the fixture file, not a copy.
    let raw = std::fs::read_to_string(fixture_template_dir().join("test.html")).unwrap();
    assert!(raw.contains("{{ title }}"), "fixture must contain the {{ title }} placeholder");

    let engine = TemplateEngine::new(&fixture_template_dir()).unwrap();
    let out = engine
        .render("test.html", &bee_rust::bee_template::context! { "title": &"E2E Home" })
        .unwrap();

    assert!(out.contains("<!DOCTYPE html>"), "doctype missing: {out}");
    assert!(out.contains("<title>E2E Home</title>"), "title tag not substituted: {out}");
    assert!(out.contains("<h1>E2E Home</h1>"), "h1 not substituted: {out}");
    assert!(out.contains("<html>") && out.contains("</html>"), "html shell missing: {out}");
}

#[test]
fn template_replaces_all_placeholders() {
    let engine = TemplateEngine::new(&fixture_template_dir()).unwrap();
    let out = engine
        .render("test.html", &bee_rust::bee_template::context! { "title": &"Replaced" })
        .unwrap();

    assert!(!out.contains("{{"), "raw placeholder left in output: {out}");
    assert_eq!(out.matches("Replaced").count(), 2, "both title slots must be filled: {out}");
}

#[test]
fn template_autoescapes_html() {
    let engine = TemplateEngine::new(&fixture_template_dir()).unwrap();
    let out = engine
        .render(
            "test.html",
            &bee_rust::bee_template::context! { "title": &"<script>alert('x')</script>" },
        )
        .unwrap();

    assert!(out.contains("&lt;script&gt;"), "script tag must be escaped: {out}");
    assert!(!out.contains("<script>"), "raw script tag leaked into output: {out}");
    assert!(out.contains("&#x27;x&#x27;"), "quote must be escaped: {out}");
}

#[test]
fn template_missing_file_reports_error() {
    let engine = TemplateEngine::new(&fixture_template_dir()).unwrap();
    let err =
        engine.render("does_not_exist.html", &bee_rust::bee_template::context! {}).unwrap_err();
    assert!(
        matches!(err, TemplateError::RenderError(_)) && err.to_string().contains("does_not_exist"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// Flow 2 + 3: full HTTP chain against the real server binary
// ---------------------------------------------------------------------------

/// A running `hello-bee` server subprocess; killed when dropped.
struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pick_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_until_ready(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server did not become ready on port {port}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Spawn the real `hello-bee` binary: exercises `bee_rust::init()`, the
/// `PORT` env handling and `axum::serve` exactly as production would.
fn start_server() -> Server {
    let port = pick_free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_hello-bee"))
        .env("PORT", port.to_string())
        .env("RUST_LOG", "error") // keep test output clean
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn hello-bee binary");
    let server = Server { child, port };
    wait_until_ready(port);
    server
}

/// Send raw bytes over TCP and read the full response (requests carry
/// `Connection: close` so EOF marks the end).
fn raw_request(port: u16, request: &[u8]) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to server");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(request).unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let text = String::from_utf8_lossy(&buf).into_owned();
    match text.split_once("\r\n\r\n") {
        Some((head, body)) => (head.to_string(), body.to_string()),
        None => (text, String::new()),
    }
}

fn get_request(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").into_bytes()
}

#[test]
fn full_chain_health_route_via_tcp() {
    let server = start_server(); // readiness proves bee_rust::init() did not block startup

    let (head, body) = raw_request(server.port, &get_request("/api/v1/health"));
    assert!(head.starts_with("HTTP/1.1 200"), "unexpected status: {head}");
    assert_eq!(body, "OK", "unexpected body: {body}");
    assert!(head.to_lowercase().contains("content-type: text/plain"), "unexpected headers: {head}");
}

#[test]
fn full_chain_unknown_route_returns_404() {
    let server = start_server();

    let (head, _) = raw_request(server.port, &get_request("/no/such/page"));
    assert!(head.starts_with("HTTP/1.1 404"), "expected 404, got: {head}");
}

#[test]
fn malformed_request_does_not_kill_server() {
    let server = start_server();

    // Invalid request line + control bytes in a header: hyper rejects it.
    let (_, _) = raw_request(server.port, b"GARBAGE \x00\x01\x02\r\nBad Header: \x03\x04\r\n\r\n");

    // The server must still answer a normal request.
    let (head, body) = raw_request(server.port, &get_request("/api/v1/health"));
    assert!(head.starts_with("HTTP/1.1 200"), "server died after malformed request: {head}");
    assert_eq!(body, "OK", "unexpected body: {body}");
}

#[test]
fn overlong_path_does_not_kill_server() {
    let server = start_server();

    let long_path = format!("/{}", "a".repeat(100_000));
    let (head, _) = raw_request(server.port, &get_request(&long_path));
    assert!(head.starts_with("HTTP/1.1 4"), "expected 4xx for overlong URI, got: {head}");

    // The server must still answer a normal request.
    let (head, body) = raw_request(server.port, &get_request("/api/v1/health"));
    assert!(head.starts_with("HTTP/1.1 200"), "server died after overlong path: {head}");
    assert_eq!(body, "OK", "unexpected body: {body}");
}
