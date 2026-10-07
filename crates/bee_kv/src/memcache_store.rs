// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! A [`KvStore`] backed by a memcached server.
//!
//! `memcache` 0.17 is synchronous — there is no async client to use — so every
//! operation runs on a blocking thread ([`tokio::task::spawn_blocking`])
//! against a shared `Arc<Client>`. Blocking because the client is safe to
//! share: it ships its own connection pool (`Client::with_pool_size`, default
//! size 4), so the pool bounds the sockets and operations beyond the pool size
//! wait for a checkout (r2d2's 30 s default timeout) rather than opening more.
//!
//! Three protocol gaps shape the mappings below and are documented where they
//! bite: memcached has no `EXISTS` command (only a full `get` answers), no
//! "increment a missing key from 0" (`increment` creates the counter at 0 but
//! returns 0 *without* applying the delta, unlike Redis's `INCR`), and counters
//! are unsigned — a decrement floors at 0, where Redis would go negative.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use memcache::{Client, CommandError, MemcacheError};

use crate::{KvError, KvStore};

/// A [`KvStore`] backed by a memcached server.
#[cfg(feature = "memcached")]
pub struct MemcacheStore {
    client: Arc<Client>,
}

#[cfg(feature = "memcached")]
impl MemcacheStore {
    /// Connect to `addr` — `"host:port"` (scheme added when missing) — with the
    /// client's default pool size (4).
    ///
    /// Connects eagerly (the pool opens its sockets here), so a bad address or
    /// an unreachable server fails in the constructor. Not an `async fn`: the
    /// client is synchronous, and only the operations run on blocking threads.
    pub fn new(addr: &str) -> Result<Self, KvError> {
        Self::with_pool_size(addr, 4)
    }

    /// Connect with an explicit pool size — see [`MemcacheStore`] for what the
    /// limit means for concurrent operations.
    pub fn with_pool_size(addr: &str, size: u32) -> Result<Self, KvError> {
        let client = Client::with_pool_size(normalize(addr), size)
            .map_err(|e| KvError::ConnectionError(format!("failed to connect: {e}")))?;
        Ok(Self { client: Arc::new(client) })
    }

    /// Run one synchronous client operation on a blocking thread. The pool
    /// lives behind the `Arc`, so the closure gets the shared client; the
    /// borrow ends with the task, hence `'static`.
    async fn blocking<T, F>(&self, op: F) -> Result<T, KvError>
    where
        T: Send + 'static,
        F: FnOnce(&Client) -> Result<T, MemcacheError> + Send + 'static,
    {
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || op(&client))
            .await
            .map_err(|e| KvError::OperationFailed(format!("blocking task failed: {e}")))?
            .map_err(|e| KvError::OperationFailed(e.to_string()))
    }
}

#[cfg(feature = "memcached")]
#[async_trait]
impl KvStore for MemcacheStore {
    async fn get(&self, key: &str) -> Result<Option<String>, KvError> {
        let key = key.to_string();
        self.blocking(move |c| c.get::<String>(&key)).await
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), KvError> {
        let (key, value) = (key.to_string(), value.to_string());
        // Expiration 0: no TTL, the key lives until it is deleted or evicted.
        self.blocking(move |c| c.set(key.as_str(), value.as_str(), 0)).await
    }

    async fn del(&self, key: &str) -> Result<(), KvError> {
        let key = key.to_string();
        // The boolean says whether the key existed; deleting a missing key is
        // not an error here (Redis `DEL` reports a count the same way).
        self.blocking(move |c| c.delete(&key).map(|_| ())).await
    }

    async fn exists(&self, key: &str) -> Result<bool, KvError> {
        // No `EXISTS` in the protocol: fetching is the only way to ask.
        Ok(self.get(key).await?.is_some())
    }

    async fn incr(&self, key: &str, amount: i64) -> Result<i64, KvError> {
        let key = key.to_string();
        self.blocking(move |c| {
            create_counter(c, &key)?;
            match counter_op(amount) {
                CounterOp::Increment(by) => Ok(c.increment(&key, by)? as i64),
                CounterOp::Decrement(by) => Ok(c.decrement(&key, by)? as i64),
            }
        })
        .await
    }

    async fn expire(&self, key: &str, seconds: i64) -> Result<(), KvError> {
        match expiration(seconds) {
            // No wire form for "expire now" — deleting is the only immediate
            // expiry, and it mirrors Redis, where `EXPIRE` with a non-positive
            // TTL deletes the key. A missing key is fine, like a 0 reply there.
            None => self.del(key).await,
            Some(seconds) => {
                let key = key.to_string();
                // `touch` returning false (no such key) is not an error either.
                self.blocking(move |c| c.touch(&key, seconds).map(|_| ())).await
            }
        }
    }

    async fn mget(&self, keys: &[&str]) -> Result<Vec<Option<String>>, KvError> {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        // No multi-get in the client: one fetch per key, order preserved, no
        // atomicity (documented on `KvStore::mget`).
        self.blocking(move |c| {
            let mut values = Vec::with_capacity(keys.len());
            for key in &keys {
                values.push(c.get::<String>(key)?);
            }
            Ok(values)
        })
        .await
    }

    async fn mset(&self, pairs: &[(&str, &str)]) -> Result<(), KvError> {
        let pairs: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        // One `set` per pair, as above.
        self.blocking(move |c| {
            for (key, value) in &pairs {
                c.set(key.as_str(), value.as_str(), 0)?;
            }
            Ok(())
        })
        .await
    }
}

/// Which wire command a signed amount turns into.
#[derive(Debug, PartialEq)]
enum CounterOp {
    Increment(u64),
    Decrement(u64),
}

/// memcached counts in `u64`: a non-negative amount is an `increment`, a
/// negative one the matching `decrement` (`unsigned_abs`, so `i64::MIN` cannot
/// overflow on negation).
fn counter_op(amount: i64) -> CounterOp {
    if amount >= 0 {
        CounterOp::Increment(amount as u64)
    } else {
        CounterOp::Decrement(amount.unsigned_abs())
    }
}

/// Make sure `key` holds a counter, creating it at 0 when it is absent.
///
/// Neither `increment` nor `decrement` does this on its own: asked for a
/// missing key, memcached *creates it at 0 and returns 0 without applying the
/// delta* (the binary protocol's initial-value semantic), while this trait —
/// like Redis's `INCR` — counts from 0 and then applies the delta. Costs one
/// extra round trip per increment; reading the reply `0` as "just created"
/// instead would break the moment the protocol or the initial value changes.
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

/// The wire expiration for a TTL, or `None` for "delete the key now".
///
/// memcached's `0` means *never expires*, so a non-positive TTL cannot be sent
/// at all; beyond 30 days the protocol switches meaning and the value is an
/// absolute unix timestamp instead of an offset. Both forms are `u32`, hence
/// the saturation.
fn expiration(seconds: i64) -> Option<u32> {
    const THIRTY_DAYS: i64 = 60 * 60 * 24 * 30;
    if seconds <= 0 {
        return None;
    }
    if seconds > THIRTY_DAYS {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let absolute = now.saturating_add(seconds as u64);
        return Some(absolute.min(u32::MAX as u64) as u32);
    }
    Some(seconds as u32)
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
    fn non_positive_ttls_delete() {
        assert_eq!(expiration(-5), None);
        assert_eq!(expiration(0), None);
    }

    #[test]
    fn amounts_dispatch_to_increment_or_decrement() {
        assert_eq!(counter_op(1), CounterOp::Increment(1));
        assert_eq!(counter_op(0), CounterOp::Increment(0));
        assert_eq!(counter_op(-3), CounterOp::Decrement(3));
        // `-amount` would overflow here; `unsigned_abs` must be used.
        assert_eq!(counter_op(i64::MIN), CounterOp::Decrement(1 << 63));
    }

    #[test]
    fn short_ttls_are_offsets() {
        assert_eq!(expiration(30), Some(30));
        // Exactly 30 days is still an offset; the protocol switches *above* it.
        assert_eq!(expiration(60 * 60 * 24 * 30), Some(2_592_000));
    }

    #[test]
    fn long_ttls_become_absolute_timestamps() {
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let seconds: i64 = 60 * 60 * 24 * 31;
        let Some(absolute) = expiration(seconds) else { panic!("expected an expiration") };
        let absolute = absolute as u64;
        let seconds = seconds as u64;
        assert!(absolute >= before + seconds, "{absolute} < {before} + {seconds}");
        assert!(absolute <= before + seconds + 5, "{absolute}");
    }

    #[test]
    fn absurd_ttls_saturate_instead_of_wrapping() {
        assert_eq!(expiration(i64::MAX), Some(u32::MAX));
        assert_eq!(expiration(u32::MAX as i64 + 1), Some(u32::MAX));
    }
}
