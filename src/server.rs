//! HTTPS server (Task 14). Binds a TCP listener, terminates TLS via rustls, and
//! serves a minimal hyper 1.x service. `/health` returns 200; `/ai/multi-agent`
//! is a Task-15 placeholder returning 501. Route wiring into the Bedrock
//! pipeline lands in Task 15; Task 14 only lands the server skeleton so the
//! boot test can exercise the TLS handshake + request path end-to-end.

use anyhow::{Context, Result};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

#[must_use]
pub struct ShutdownTx(pub oneshot::Sender<()>);
impl ShutdownTx {
    /// Send the shutdown signal. The `()` arg matches the plan's pseudo-signature
    /// so tests read as `shutdown.send(())`.
    ///
    /// # Errors
    ///
    /// Returns `Err(())` if the receiver has already been dropped (server task
    /// exited before shutdown).
    #[allow(clippy::result_unit_err)]
    pub fn send(self, (): ()) -> Result<(), ()> {
        self.0.send(())
    }
}

/// Spawn an HTTPS server on `bind` using the PEM cert + key at the given paths.
/// Returns the bound `SocketAddr` (useful when `bind` uses port 0) and a
/// `ShutdownTx` handle.
///
/// # Errors
///
/// Returns an error if the cert/key files can't be read or parsed, rustls
/// rejects the material, or binding the TCP listener fails.
pub async fn spawn_test_server(
    bind: &str,
    cert_pem: &Path,
    key_pem: &Path,
) -> Result<(SocketAddr, ShutdownTx)> {
    let tls_cfg = tls_config_from_pem(cert_pem, key_pem)?;
    let listener = TcpListener::bind(bind).await.context("bind")?;
    let addr = listener.local_addr()?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_cfg));
    let (tx, rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::select! {
            _ = rx => { tracing::info!("server shutdown signal received"); }
            () = accept_loop(listener, acceptor) => {}
        }
    });
    Ok((addr, ShutdownTx(tx)))
}

async fn accept_loop(listener: TcpListener, acceptor: TlsAcceptor) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?e, "accept failed");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(stream).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(%peer, ?e, "tls handshake failed");
                    return;
                }
            };
            let io = TokioIo::new(tls);
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, hyper::service::service_fn(handle))
                .await
            {
                tracing::warn!(?e, "connection error");
            }
        });
    }
}

async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method().clone(), req.uri().path()) {
        (hyper::Method::GET, "/health") => Ok(Response::new(Full::new(Bytes::from("ok")))),
        (hyper::Method::POST, "/ai/multi-agent") => {
            // Placeholder; real routing lives in Task 15.
            Ok(Response::builder()
                .status(StatusCode::NOT_IMPLEMENTED)
                .body(Full::new(Bytes::from("Phase 0 WIP")))
                .unwrap())
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::new()))
            .unwrap()),
    }
}

/// Load cert chain + private key from PEM files and build a rustls `ServerConfig`.
///
/// # Errors
///
/// Returns an error if files can't be opened, PEM parsing fails, no PKCS#8 key
/// is present, or rustls rejects the cert/key pair.
fn tls_config_from_pem(cert_pem: &Path, key_pem: &Path) -> Result<ServerConfig> {
    // rustls 0.23 requires an installed crypto provider before `ServerConfig::builder`.
    // We use `aws_lc_rs` because the workspace already links aws-lc-rs transitively
    // via aws-sdk; adding `ring` would duplicate crypto backends. `.ok()` swallows
    // "already installed" — safe to call repeatedly.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut cert_reader = std::io::BufReader::new(std::fs::File::open(cert_pem)?);
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader).collect::<std::result::Result<_, _>>()?;
    let mut key_reader = std::io::BufReader::new(std::fs::File::open(key_pem)?);
    let key = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .next()
        .context("no pkcs8 key")?
        .map(PrivateKeyDer::Pkcs8)?;
    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(cfg)
}
