// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! Cache key, eligibility, and the private response-cache wire format used by
//! [`LatencyConfig`](super::LatencyConfig).
//!
//! The format is deliberately small and self-contained: a response cache
//! entry holds a status, a filtered header set, the storage time, the TTL it
//! was written with, and the body. It is only ever read back by this module,
//! so there is no cross-version compatibility promise.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, HOST, RANGE, SET_COOKIE,
    VARY,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::response::Response;

/// Fixed namespace for response-cache keys. Session IDs and application keys
/// never use this prefix.
const KEY_PREFIX: &str = "bee:resp:v1:";
const MAGIC: &[u8; 4] = b"brsp";
const VERSION: u8 = 1;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_NAME: usize = 256;
const MAX_HEADER_VALUE: usize = 8 * 1024;
const MAX_ENTRY: usize = 72 * 1024;

/// A decoded response-cache entry.
pub(crate) struct CacheEntry {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) stored_at: u64,
    pub(crate) ttl_secs: u32,
    pub(crate) body: Vec<u8>,
}

impl CacheEntry {
    pub(crate) fn age_secs(&self) -> u64 {
        now_secs().saturating_sub(self.stored_at)
    }

    pub(crate) fn is_fresh(&self) -> bool {
        self.age_secs() <= u64::from(self.ttl_secs)
    }
}

/// A request-level cache bypass. `GET`/`HEAD` are the only methods in the
/// latency path; a request with credentials, a partial-range header, or an
/// explicit no-cache/no-store directive must not be served from, or written
/// to, the shared response cache.
pub(crate) fn request_cacheable(req: &Request<Body>) -> bool {
    if !matches!(req.method(), &Method::GET | &Method::HEAD) {
        return false;
    }
    if req.headers().contains_key(AUTHORIZATION) || req.headers().contains_key(RANGE) {
        return false;
    }
    !cache_control_has(req.headers(), "no-store") && !cache_control_has(req.headers(), "no-cache")
}

/// Whether a response may be stored. The caller separately checks the body
/// size because that requires the body's size hint.
pub(crate) fn response_cacheable(resp: &Response<Body>) -> bool {
    if resp.status() != StatusCode::OK {
        return false;
    }
    if resp.headers().contains_key(SET_COOKIE) || resp.headers().contains_key(VARY) {
        return false;
    }
    cache_control_has(resp.headers(), "public")
        && !cache_control_has(resp.headers(), "private")
        && !cache_control_has(resp.headers(), "no-store")
        && !cache_control_has(resp.headers(), "no-cache")
}

/// Parse a comma-separated `Cache-Control` directive, case-insensitively.
pub(crate) fn cache_control_has(headers: &HeaderMap, directive: &str) -> bool {
    headers
        .get_all(CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| {
            let key = part.split('=').next().unwrap_or_default().trim();
            key.eq_ignore_ascii_case(directive)
        })
}

/// The response's `max-age` in seconds, if present and valid.
pub(crate) fn response_max_age(headers: &HeaderMap) -> Option<u64> {
    headers
        .get_all(CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .find_map(|part| {
            let (key, value) = part.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("max-age")
                .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                .flatten()
        })
}

/// Stable, opaque cache key: method, authority, path/query, and the two
/// representation headers that commonly change a response. Hashing keeps
/// query strings (which can contain tokens) out of shared cache keys.
pub(crate) fn cache_key(req: &Request<Body>) -> String {
    let method = if req.method().as_str() == "HEAD" { "GET" } else { req.method().as_str() };
    let authority = req
        .uri()
        .authority()
        .map(|authority| authority.as_str())
        .or_else(|| req.headers().get(HOST).and_then(|value| value.to_str().ok()))
        .unwrap_or_default();
    let path = req.uri().path_and_query().map(|path| path.as_str()).unwrap_or_default();
    let accept =
        req.headers().get(ACCEPT).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let accept_encoding = req
        .headers()
        .get(ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for field in [method, authority, path, accept, accept_encoding] {
        // Length-prefix each field so concatenation cannot alias.
        hash = fnv1a(hash, &(field.len() as u64).to_le_bytes());
        hash = fnv1a(hash, field.as_bytes());
    }
    format!("{KEY_PREFIX}{hash:016x}")
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Encode a response for the cache. Returns `None` when the header set cannot
/// be represented within the entry limits; the caller then skips storage.
pub(crate) fn encode(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    ttl_secs: u32,
) -> Option<Vec<u8>> {
    if status != StatusCode::OK || body.len() > MAX_ENTRY {
        return None;
    }
    let mut filtered = Vec::new();
    for (name, value) in headers {
        if skip_header(name) {
            continue;
        }
        if filtered.len() >= MAX_HEADERS
            || name.as_str().len() > MAX_HEADER_NAME
            || value.as_bytes().len() > MAX_HEADER_VALUE
        {
            return None;
        }
        filtered.push((name.as_str().as_bytes().to_vec(), value.as_bytes().to_vec()));
    }

    let mut out = Vec::with_capacity(32 + body.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(status.as_u16()).to_le_bytes());
    out.extend_from_slice(&now_secs().to_le_bytes());
    out.extend_from_slice(&ttl_secs.to_le_bytes());
    out.extend_from_slice(&(filtered.len() as u16).to_le_bytes());
    for (name, value) in &filtered {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    (out.len() <= MAX_ENTRY).then_some(out)
}

/// Decode a cache value. Any malformed entry is a cache miss; it must never
/// turn into a 500 for the request.
pub(crate) fn decode(bytes: &[u8]) -> Option<CacheEntry> {
    if bytes.len() < 4 + 1 + 2 + 8 + 4 + 2 + 4 || bytes.len() > MAX_ENTRY {
        return None;
    }
    let mut cursor = 0;
    let take = |cursor: &mut usize, len: usize| -> Option<&[u8]> {
        let end = cursor.checked_add(len)?;
        let slice = bytes.get(*cursor..end)?;
        *cursor = end;
        Some(slice)
    };

    if take(&mut cursor, 4)? != MAGIC || take(&mut cursor, 1)?[0] != VERSION {
        return None;
    }
    let status =
        StatusCode::from_u16(u16::from_le_bytes(take(&mut cursor, 2)?.try_into().ok()?)).ok()?;
    if status != StatusCode::OK {
        return None;
    }
    let stored_at = u64::from_le_bytes(take(&mut cursor, 8)?.try_into().ok()?);
    let ttl_secs = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().ok()?);
    let count = u16::from_le_bytes(take(&mut cursor, 2)?.try_into().ok()?) as usize;
    if count > MAX_HEADERS {
        return None;
    }

    let mut headers = HeaderMap::with_capacity(count);
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(&mut cursor, 2)?.try_into().ok()?) as usize;
        if name_len == 0 || name_len > MAX_HEADER_NAME {
            return None;
        }
        let name = HeaderName::from_bytes(take(&mut cursor, name_len)?).ok()?;
        if skip_header(&name) {
            return None;
        }
        let value_len = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().ok()?) as usize;
        if value_len > MAX_HEADER_VALUE {
            return None;
        }
        let value = HeaderValue::from_bytes(take(&mut cursor, value_len)?).ok()?;
        headers.append(name, value);
    }

    let body_len = u32::from_le_bytes(take(&mut cursor, 4)?.try_into().ok()?) as usize;
    if cursor.checked_add(body_len)? != bytes.len() || body_len > MAX_ENTRY {
        return None;
    }
    let body = take(&mut cursor, body_len)?.to_vec();
    Some(CacheEntry { status, headers, stored_at, ttl_secs, body })
}

/// Rebuild a response from a cache entry. `HEAD` keeps the representation's
/// headers (including length) but sends no body.
pub(crate) fn replay(entry: CacheEntry, head: bool) -> Response<Body> {
    let length = entry.body.len();
    let age = entry.age_secs().min(u64::from(u32::MAX)) as u32;
    let status = entry.status;
    let mut headers = entry.headers;
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length as u64));
    headers.insert(HeaderName::from_static("age"), HeaderValue::from(age));
    headers.insert(HeaderName::from_static("x-bee-cache"), HeaderValue::from_static("stale"));
    let body = if head { Vec::new() } else { entry.body };
    let mut response =
        Response::builder().status(status).body(Body::from(body)).unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("cached response could not be rebuilt"))
                .expect("static response is valid")
        });
    *response.headers_mut() = headers;
    response
}

fn skip_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "date"
            | "age"
            | "set-cookie"
            | "vary"
    )
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, VARY};

    fn request(method: Method, uri: &str) -> Request<Body> {
        Request::builder().method(method).uri(uri).body(Body::empty()).unwrap()
    }

    fn public_response(body: &'static str) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(CACHE_CONTROL, "public, max-age=30")
            .header(CONTENT_TYPE, "text/plain")
            .body(Body::from(body))
            .unwrap()
    }

    #[test]
    fn key_is_stable_and_separates_method_uri_and_representation() {
        let get = request(Method::GET, "https://example.com/a?x=1");
        let head = request(Method::HEAD, "https://example.com/a?x=1");
        assert_eq!(cache_key(&get), cache_key(&head));
        assert_ne!(cache_key(&get), cache_key(&request(Method::GET, "https://example.com/a?x=2")));
        assert_ne!(cache_key(&get), cache_key(&request(Method::GET, "https://other.com/a?x=1")));

        let mut accept = request(Method::GET, "https://example.com/a?x=1");
        accept.headers_mut().insert(ACCEPT, HeaderValue::from_static("application/json"));
        assert_ne!(cache_key(&get), cache_key(&accept));
    }

    #[test]
    fn request_bypasses_credentials_ranges_and_no_store() {
        let mut req = request(Method::GET, "/a");
        assert!(request_cacheable(&req));
        req.headers_mut().insert(AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        assert!(!request_cacheable(&req));

        let mut req = request(Method::GET, "/a");
        req.headers_mut().insert(RANGE, HeaderValue::from_static("bytes=0-1"));
        assert!(!request_cacheable(&req));

        let mut req = request(Method::GET, "/a");
        req.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        assert!(!request_cacheable(&req));

        assert!(!request_cacheable(&request(Method::POST, "/a")));
    }

    #[test]
    fn response_requires_public_and_rejects_private_markers() {
        let resp = public_response("ok");
        assert!(response_cacheable(&resp));

        let private = Response::builder()
            .status(StatusCode::OK)
            .header(CACHE_CONTROL, "public, private")
            .body(Body::empty())
            .unwrap();
        assert!(!response_cacheable(&private));

        let cookie = Response::builder()
            .status(StatusCode::OK)
            .header(CACHE_CONTROL, "public")
            .header(SET_COOKIE, "a=b")
            .body(Body::empty())
            .unwrap();
        assert!(!response_cacheable(&cookie));

        let vary = Response::builder()
            .status(StatusCode::OK)
            .header(CACHE_CONTROL, "public")
            .header(VARY, "accept")
            .body(Body::empty())
            .unwrap();
        assert!(!response_cacheable(&vary));

        let not_ok = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(CACHE_CONTROL, "public")
            .body(Body::empty())
            .unwrap();
        assert!(!response_cacheable(&not_ok));
    }

    #[test]
    fn max_age_is_parsed_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("Public, Max-Age=\"12\""));
        assert!(cache_control_has(&headers, "public"));
        assert_eq!(response_max_age(&headers), Some(12));
    }

    #[test]
    fn entry_roundtrip_filters_headers_and_preserves_ttl() {
        let mut resp = public_response("hello");
        resp.headers_mut().insert(SET_COOKIE, HeaderValue::from_static("s=1"));
        resp.headers_mut().insert(VARY, HeaderValue::from_static("accept"));
        let encoded = encode(resp.status(), resp.headers(), b"hello", 42).expect("encodes");
        let entry = decode(&encoded).expect("decodes");
        assert_eq!(entry.body, b"hello");
        assert_eq!(entry.ttl_secs, 42);
        assert!(entry.is_fresh() || entry.age_secs() <= 42);
        assert!(!entry.headers.contains_key(SET_COOKIE));
        assert!(!entry.headers.contains_key(VARY));
        assert_eq!(entry.headers.get(CONTENT_TYPE).unwrap(), "text/plain");
    }

    #[test]
    fn malformed_or_oversized_entries_are_misses() {
        assert!(decode(b"").is_none());
        assert!(decode(b"brsp\x01").is_none());
        let mut value = encode(StatusCode::OK, &HeaderMap::new(), b"ok", 1).unwrap();
        value[0] = b'x';
        assert!(decode(&value).is_none());

        let oversized = vec![0u8; MAX_ENTRY + 1];
        assert!(decode(&oversized).is_none());
    }

    #[test]
    fn replay_adds_staleness_headers_and_head_drops_the_body() {
        let entry =
            decode(&encode(StatusCode::OK, &HeaderMap::new(), b"hello", 30).unwrap()).unwrap();
        let response = replay(entry, true);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-bee-cache").unwrap(), "stale");
        assert_eq!(response.headers().get(CONTENT_LENGTH).unwrap(), "5");
    }
}
