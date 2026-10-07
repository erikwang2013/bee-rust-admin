// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`Cache`] backed by a memcached server.
//!
//! `memcache` 0.17 is synchronous — there is no async client to use — so every
//! operation runs on a blocking thread ([`tokio::task::spawn_blocking`])
//! against a shared `Arc<Client>`. Blocking because the client is safe to
//! share: it ships its own connection pool (`Client::with_pool_size`, default
//! size 4), so the pool bounds the sockets and operations beyond the pool size
//! wait for a checkout (r2d2's 30 s default timeout) rather than opening more.
//!
//! Counter semantics are memcached's, not Redis's, and [`Cache::incr`]
//! compensates where the trait promises otherwise: a missing counter is
//! created at 0 *without* the delta being applied (see `create_counter`), and
//! counters are unsigned, so nothing here can go negative.
//!
//! This deliberately duplicates `bee_kv::MemcacheStore`'s connection handling
//! rather than sharing it: the two traits disagree on the value type
//! (`Vec<u8>` here, `String` there), and one small copy beats an internal crate
//! or a lossy adapter.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use memcache::{Client, CommandError, MemcacheError};

use crate::{Cache, CacheError};

/// A [`Cache`] backed by a memcached server.
pub struct MemcacheCache {
    client: Arc<Client>,
}

impl MemcacheCache {
    /// Connect to `addr` — `"host:port"` (scheme added when missing) — with the
    /// client's default pool size (4).
    ///
    /// Connects eagerly (the pool opens its sockets here), so a bad address or
    /// an unreachable server fails in the constructor. Not an `async fn`: the
    /// client is synchronous, and only the operations run on blocking threads.
    pub fn new(addr: &str) -> Result<Self, CacheError> {
        Self::with_pool_size(addr, 4)
    }

    /// Connect with an explicit pool size — see [`MemcacheCache`] for what the
    /// limit means for concurrent operations.
    pub fn with_pool_size(addr: &str, size: u32) -> Result<Self, CacheError> {
        let client = Client::with_pool_size(normalize(addr), size)
            .map_err(|e| CacheError::ConnectionError(format!("failed to connect: {e}")))?;
        Ok(Self { client: Arc::new(client) })
    }

    /// Run one synchronous client operation on a blocking thread. The pool
    /// lives behind the `Arc`, so the closure gets the shared client; the
    /// borrow ends with the task, hence `'static`.
    async fn blocking<T, F>(&self, op: F) -> Result<T, CacheError>
    where
        T: Send + 'static,
        F: FnOnce(&Client) -> Result<T, MemcacheError> + Send + 'static,
    {
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || op(&client))
            .await
            .map_err(|e| CacheError::ConnectionError(format!("blocking task failed: {e}")))?
            .map_err(map_err)
    }
}

#[async_trait]
impl Cache for MemcacheCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let key = key.to_string();
        // `Vec<u8>` converts byte-for-byte (the crate special-cases it), so any
        // blob survives the trip.
        self.blocking(move |c| c.get::<Vec<u8>>(&key)).await
    }

    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<u64>) -> Result<(), CacheError> {
        let key = key.to_string();
        match ttl {
            // No TTL: memcached's 0 means *never expires*.
            None => self.blocking(move |c| c.set(key.as_str(), value.as_slice(), 0)).await,
            // A zero TTL means already expired = absent, which is not something
            // the protocol can express (`0` is the opposite); deleting is the
            // only way, and a missing key is not an error here.
            Some(0) => self.blocking(move |c| c.delete(&key).map(|_| ())).await,
            Some(seconds) => {
                let expiration = wire_expiration(seconds);
                self.blocking(move |c| c.set(key.as_str(), value.as_slice(), expiration)).await
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        let key = key.to_string();
        match self.blocking(move |c| c.delete(&key)).await? {
            true => Ok(()),
            false => Err(CacheError::NotFound),
        }
    }

    async fn incr(&self, key: &str) -> Result<i64, CacheError> {
        let key = key.to_string();
        self.blocking(move |c| {
            create_counter(c, &key)?;
            Ok(c.increment(&key, 1)? as i64)
        })
        .await
    }
}

/// Make sure `key` holds a counter, creating it at 0 when it is absent.
///
/// `increment` alone does not honour the trait's "a missing key is set to 0
/// before incrementing": asked for a missing key, memcached *creates it at 0
/// and returns 0 without applying the delta* (the binary protocol's
/// initial-value semantic), so `incr` on a fresh key would report 0 instead of
/// the 1 the trait promises. Costs one extra round trip per increment; reading
/// the reply `0` as "just created" instead would break the moment the protocol
/// or the initial value changes.
fn create_counter(client: &Client, key: &str) -> Result<(), MemcacheError> {
    match client.add(key, "0", 0) {
        Ok(()) => Ok(()),
        // Already there — all we wanted. (The text protocol reports this as a
        // `NOT_STORED` reply the client maps to `Ok(())`; the binary one as
        // `KeyExists`.)
        Err(MemcacheError::CommandError(CommandError::KeyExists)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// `host:port` is what users type; the URL parser needs a scheme.
fn normalize(addr: &str) -> String {
    if addr.contains("://") { addr.to_string() } else { format!("memcache://{addr}") }
}

/// The wire expiration for a positive TTL.
///
/// Beyond 30 days the protocol switches meaning and the value is an absolute
/// unix timestamp instead of an offset. Both forms are `u32`, hence the
/// saturation; `0` (never expires) is the caller's business, not this
/// function's.
fn wire_expiration(seconds: u64) -> u32 {
    const THIRTY_DAYS: u64 = 60 * 60 * 24 * 30;
    if seconds > THIRTY_DAYS {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        return now.saturating_add(seconds).min(u32::MAX as u64) as u32;
    }
    seconds as u32
}

/// The binary protocol's "increment/decrement on a non-numeric value" status;
/// the crate leaves protocol statuses it has no name for as `Unknown`.
const NON_NUMERIC_COUNTER: u16 = 0x0006;

/// A server-side refusal of the *stored data* — `increment` on a non-numeric
/// value — is not a connection problem; everything else is. (The binary
/// protocol reports that refusal as a status code, the text one as a
/// `CLIENT_ERROR`, hence the two shapes.)
fn map_err(e: MemcacheError) -> CacheError {
    let message = e.to_string();
    match e {
        MemcacheError::CommandError(CommandError::Unknown(code)) if code == NON_NUMERIC_COUNTER => {
            CacheError::SerializeError(message)
        }
        MemcacheError::ClientError(_) => CacheError::SerializeError(message),
        _ => CacheError::ConnectionError(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_port_gets_a_scheme() {
        assert_eq!(normalize("127.0.0.1:11211"), "memcache://127.0.0.1:11211");
        assert_eq!(normalize("memcache://host:11211"), "memcache://host:11211");
    }

    #[test]
    fn short_ttls_are_offsets() {
        assert_eq!(wire_expiration(1), 1);
        // Exactly 30 days is still an offset; the protocol switches *above* it.
        assert_eq!(wire_expiration(60 * 60 * 24 * 30), 2_592_000);
    }

    #[test]
    fn long_ttls_become_absolute_timestamps() {
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let seconds = 60 * 60 * 24 * 31;
        let absolute = wire_expiration(seconds) as u64;
        assert!(absolute >= before + seconds, "{absolute} < {before} + {seconds}");
        assert!(absolute <= before + seconds + 5, "{absolute}");
    }

    #[test]
    fn absurd_ttls_saturate_instead_of_wrapping() {
        assert_eq!(wire_expiration(u64::MAX), u32::MAX);
    }
}
