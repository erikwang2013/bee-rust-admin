// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
#![cfg(feature = "postgres-tls")]
//! §41 A2 / §30 amendment: TLS against the real PostgreSQL container, now that
//! [`Pool::connect_tls_with`] takes a caller-supplied `rustls::ClientConfig`.
//!
//! - Positive: a test-only no-verify verifier + `sslmode=require` → the
//!   handshake completes and `pg_stat_ssl` reports the session as encrypted.
//! - Negative B: the bundled-roots [`Pool::connect_tls`] must reject the
//!   container's self-signed certificate — verification is enforced by the
//!   wrapper; injection is how a caller opts out.
//!
//! Env-gated by `BEE_ORM_PG_TLS_DSN` (`sslmode=require` is appended when
//! absent): point it at a TLS-enabled server. In CI the service container is
//! switched to TLS between the main integration step — which is where the
//! non-TLS negatives ([`integration_pg.rs`]) must keep running — and this
//! target's step.

mod common;

use std::sync::Arc;

use bee_orm::OrmError;
use bee_orm::pool::postgres::Pool;
use bee_orm::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use bee_orm::rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature};
use bee_orm::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use bee_orm::rustls::{ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme};

/// Accepts any certificate: the injection point exists so a caller can opt out
/// of chain verification (private CA, test rig). Lives in `tests/` only.
#[derive(Debug)]
struct NoVerify;

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

/// The TLS DSN, with `sslmode=require` appended when the caller did not state a
/// mode — `prefer` would silently fall back to plaintext and hollow out both
/// tests.
fn tls_dsn() -> Option<String> {
    let dsn = common::dsn("BEE_ORM_PG_TLS_DSN")?;
    Some(if dsn.contains("sslmode") {
        dsn
    } else {
        let separator = if dsn.contains('?') { '&' } else { '?' };
        format!("{dsn}{separator}sslmode=require")
    })
}

fn no_verify_config() -> ClientConfig {
    // The ring provider is named explicitly: with both rustls providers in the
    // workspace graph the process-level default is ambiguous (same reason as
    // the pool's bundled-roots config).
    ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth()
}

#[tokio::test]
async fn injected_config_completes_a_real_tls_handshake() -> Result<(), OrmError> {
    let Some(dsn) = tls_dsn() else { return Ok(()) };
    let pool = Pool::connect_tls_with(&dsn, 1, no_verify_config())?;

    let rows = pool.query("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()", &[]).await?;
    assert_eq!(rows[0]["ssl"].as_bool(), Some(true), "the session must be encrypted");

    // The pooled connection stays usable past the handshake.
    let n = pool.query("SELECT 1 AS n", &[]).await?;
    assert_eq!(n[0]["n"], 1);
    Ok(())
}

#[tokio::test]
async fn bundled_roots_reject_the_self_signed_certificate() -> Result<(), OrmError> {
    let Some(dsn) = tls_dsn() else { return Ok(()) };
    let pool = Pool::connect_tls(&dsn, 1)?;

    let err = match pool.get().await {
        Ok(_) => panic!("bundled roots must not trust the self-signed test certificate"),
        Err(err) => err,
    };
    // The driver's message is a generic "error performing TLS handshake"
    // (measured) — it can't name the certificate. The evidence is the pair:
    // the positive test runs this same DSN to completion, so the only variable
    // left in this one is the verifier.
    assert!(matches!(err, OrmError::ConnectionError(_)), "unexpected error: {err}");
    Ok(())
}
