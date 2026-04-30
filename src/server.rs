//! HTTPS server. Binds a TCP listener, terminates TLS via rustls, and serves a
//! minimal hyper 1.x service.
//!
//! Two public entry points:
//!
//!   * [`spawn_test_server`] — Task 14's skeleton. `/health` returns 200;
//!     `/ai/multi-agent` responds 501. Still used by the health boot test.
//!   * [`spawn`] — Task 15 wiring. Accepts an `Arc<Config>` + `Arc<dyn
//!     BedrockLike>` and delegates `POST /ai/multi-agent` to
//!     [`crate::route_multi_agent::handle`]. Everything else shares the Task
//!     14 semantics (200 on `/health`, 404 otherwise).

use crate::bedrock_client::BedrockLike;
use crate::config::Config;
use crate::route_multi_agent::{self, BoxedBody};
use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// Shutdown handle for a server spawned by [`spawn_test_server`].
///
/// Dropping this handle does NOT shut down the server — the internal
/// `oneshot::Sender` is dropped silently and the accept loop continues
/// running. Call [`ShutdownTx::send`] to actually stop the server.
#[must_use = "dropping ShutdownTx does not stop the server; call .send(()) to shut down"]
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

/// Spawn an HTTPS server wired to the Task 15 `/ai/multi-agent` pipeline.
///
/// Identical TLS + accept-loop plumbing as [`spawn_test_server`], but the
/// per-request handler dispatches `POST /ai/multi-agent` to
/// [`route_multi_agent::handle`] with the supplied `cfg` + `bedrock`
/// implementation. All other paths preserve the Task 14 behavior.
///
/// # Errors
/// Same as [`spawn_test_server`].
pub async fn spawn(
    bind: &str,
    cert_pem: &Path,
    key_pem: &Path,
    cfg: Arc<Config>,
    bedrock: Arc<dyn BedrockLike>,
) -> Result<(SocketAddr, ShutdownTx)> {
    let tls_cfg = tls_config_from_pem(cert_pem, key_pem)?;
    let listener = TcpListener::bind(bind).await.context("bind")?;
    let addr = listener.local_addr()?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_cfg));
    let (tx, rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::select! {
            _ = rx => { tracing::info!("server shutdown signal received"); }
            () = accept_loop_with_ctx(listener, acceptor, cfg, bedrock) => {}
        }
    });
    Ok((addr, ShutdownTx(tx)))
}

async fn accept_loop_with_ctx(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    cfg: Arc<Config>,
    bedrock: Arc<dyn BedrockLike>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?e, "accept failed");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let cfg = cfg.clone();
        let bedrock = bedrock.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(stream).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(%peer, ?e, "tls handshake failed");
                    return;
                }
            };
            let io = TokioIo::new(tls);
            let svc = hyper::service::service_fn(move |req| {
                let cfg = cfg.clone();
                let bedrock = bedrock.clone();
                async move { handle_with_context(req, cfg, bedrock).await }
            });
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await
            {
                tracing::warn!(?e, "connection error");
            }
        });
    }
}

/// Wraps a `Full<Bytes>` body as the `BoxedBody` the multi-agent route uses,
/// mapping the `Infallible` error into `io::Error` so the types line up.
fn full_to_boxed(body: Full<Bytes>) -> BoxedBody {
    body.map_err(|never: Infallible| match never {}).boxed()
}

/// Dispatcher for the Task-15 server: routes `POST /ai/multi-agent` into the
/// pipeline; everything else preserves the Task-14 health/404 semantics.
///
/// Never returns `Err` up to hyper — any internal failure is surfaced as a 500
/// with the anyhow chain in the body. Returning `Err` would close the
/// connection without a response, which is worse UX for callers.
async fn handle_with_context(
    req: Request<Incoming>,
    cfg: Arc<Config>,
    bedrock: Arc<dyn BedrockLike>,
) -> Result<Response<BoxedBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    match (method, path.as_str()) {
        (hyper::Method::POST, "/ai/multi-agent") => {
            match route_multi_agent::handle(req, cfg, bedrock).await {
                Ok(resp) => Ok(resp),
                Err(e) => {
                    tracing::error!(?e, "route_multi_agent error");
                    let body = Full::new(Bytes::from(format!("{e:#}")));
                    let resp = Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(full_to_boxed(body))
                        .expect("static 500 response");
                    Ok(resp)
                }
            }
        }
        (hyper::Method::GET, "/health") => {
            let body = Full::new(Bytes::from("ok"));
            Ok(Response::new(full_to_boxed(body)))
        }
        _ => {
            let body = Full::new(Bytes::new());
            let resp = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full_to_boxed(body))
                .expect("static 404 response");
            Ok(resp)
        }
    }
}

// Task 14 legacy handler — retained solely for `spawn_test_server` and its
// health-boot integration test. Task 15 went with a parallel
// `handle_with_context` + `spawn` rather than refactoring this one, so
// `spawn_test_server` continues to exercise this code path verbatim.
// The `POST /ai/multi-agent` arm here still returns 501; the real pipeline
// lives in `handle_with_context` above.
async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method().clone(), req.uri().path()) {
        (hyper::Method::GET, "/health") => Ok(Response::new(Full::new(Bytes::from("ok")))),
        (hyper::Method::POST, "/ai/multi-agent") => Ok(Response::builder()
            .status(StatusCode::NOT_IMPLEMENTED)
            .body(Full::new(Bytes::from(
                "Phase 0 WIP (use `spawn`, not `spawn_test_server`)",
            )))
            .unwrap()),
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
