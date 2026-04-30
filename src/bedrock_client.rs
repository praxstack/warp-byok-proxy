//! Thin Bedrock runtime client factory.
//!
//! Maps a [`crate::auth::ResolvedAuth`] onto an `aws_config` loader, optionally
//! pinning an endpoint URL, and hands back an `aws_sdk_bedrockruntime::Client`.
//! Bearer-token auth is not supplied through the SDK credentials chain — it
//! rides as an HTTP header set by the caller (see [`bearer_header`]).
//!
//! The module also defines [`BedrockLike`], a small async trait used by the
//! `/ai/multi-agent` route so the pipeline can be driven by either the real
//! Bedrock `converse_stream` (Task 17) or a [`MockBedrock`] in-memory scripted
//! stream (Task 15 E2E test).

use crate::auth::ResolvedAuth;
use crate::stream_accumulator::BedrockEvent;
use crate::translator::BedrockInput;
use anyhow::Result;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::Client as BedrockClient;
use tokio_stream::wrappers::ReceiverStream;

/// Build a Bedrock runtime client honoring the resolved auth mode, region,
/// and optional endpoint override.
///
/// # Errors
/// Currently infallible because the underlying SDK calls do not return errors
/// here, but returns `Result` to reserve space for future endpoint/region
/// validation without a breaking change.
pub async fn build_client(
    auth: &ResolvedAuth,
    region: &str,
    endpoint: Option<&str>,
) -> Result<BedrockClient> {
    let mut loader =
        aws_config::defaults(BehaviorVersion::latest()).region(Region::new(region.to_string()));
    match auth {
        ResolvedAuth::Profile(p) => {
            loader = loader.profile_name(p);
        }
        ResolvedAuth::ExplicitKeys {
            access_key,
            secret_key,
            session_token,
        } => {
            loader = loader.credentials_provider(aws_credential_types::Credentials::new(
                access_key.clone(),
                secret_key.clone(),
                session_token.clone(),
                None,
                "warp-byok-proxy",
            ));
        }
        ResolvedAuth::BearerToken(_) | ResolvedAuth::DefaultChain | ResolvedAuth::Skipped => {
            // bearer → handled as an HTTP header by the caller (see `bearer_header`);
            // default chain → SDK picks up env/IMDS/etc.;
            // skipped → no auth injected.
        }
    }
    if let Some(ep) = endpoint {
        loader = loader.endpoint_url(ep);
    }
    let sdk_config = loader.load().await;
    Ok(BedrockClient::new(&sdk_config))
}

/// Return a `Bearer <token>` string when the resolved auth is a bearer token,
/// otherwise `None`.
#[must_use]
pub fn bearer_header(auth: &ResolvedAuth) -> Option<String> {
    match auth {
        ResolvedAuth::BearerToken(t) => Some(format!("Bearer {t}")),
        ResolvedAuth::Profile(_)
        | ResolvedAuth::ExplicitKeys { .. }
        | ResolvedAuth::DefaultChain
        | ResolvedAuth::Skipped => None,
    }
}

/// Abstraction over a Bedrock Converse streaming call.
///
/// Implementors take a translated [`BedrockInput`] and return a stream of
/// [`BedrockEvent`]s. This lets the `/ai/multi-agent` route accept either a
/// real [`RealBedrock`] (Task 17) or a scripted [`MockBedrock`] (Task 15 test).
#[async_trait::async_trait]
pub trait BedrockLike: Send + Sync {
    /// Start a streaming converse call. Returns a `ReceiverStream` yielding
    /// `Result<BedrockEvent>` per event. Dropping the sender (end of stream)
    /// indicates the turn has finished.
    ///
    /// # Errors
    /// Returns an error if the request cannot be dispatched. Individual
    /// per-event errors surface as `Err(_)` items on the stream.
    async fn converse_stream(
        &self,
        input: BedrockInput,
    ) -> Result<ReceiverStream<Result<BedrockEvent>>>;
}

/// Scripted in-memory [`BedrockLike`] used by the Task 15 E2E test.
///
/// On [`BedrockLike::converse_stream`], spawns a task that replays each
/// scripted event through a bounded mpsc channel and drops the sender. The
/// input [`BedrockInput`] is ignored — tests that want to assert on it should
/// capture inputs via a custom `BedrockLike` impl.
#[must_use]
pub struct MockBedrock {
    /// Events to emit, in order.
    pub scripted: Vec<BedrockEvent>,
}

#[async_trait::async_trait]
impl BedrockLike for MockBedrock {
    async fn converse_stream(
        &self,
        _input: BedrockInput,
    ) -> Result<ReceiverStream<Result<BedrockEvent>>> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let events = self.scripted.clone();
        tokio::spawn(async move {
            for ev in events {
                if tx.send(Ok(ev)).await.is_err() {
                    break;
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }
}

/// Real Bedrock implementation of [`BedrockLike`].
///
/// Stubbed for Phase 0 — Task 17 will populate the actual `converse_stream`
/// call using [`build_client`]. Calling it today panics.
#[must_use]
pub struct RealBedrock {
    /// Underlying Bedrock SDK client, built by [`build_client`].
    pub client: BedrockClient,
}

#[async_trait::async_trait]
impl BedrockLike for RealBedrock {
    async fn converse_stream(
        &self,
        _input: BedrockInput,
    ) -> Result<ReceiverStream<Result<BedrockEvent>>> {
        todo!("real Bedrock streaming - Task 17")
    }
}
