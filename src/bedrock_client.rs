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
use crate::sdk_translator;
use crate::stream_accumulator::BedrockEvent;
use crate::translator::BedrockInput;
use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::types::{
    ContentBlockDelta, ContentBlockStart, ConverseStreamOutput, ReasoningContentBlockDelta,
};
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
/// Dispatches the fluent `converse_stream()` builder on the SDK client using
/// translations from [`sdk_translator`] (Steps 1, 2, 4 of Phase 0):
///   * `messages: Vec<serde_json::Value>` → `Vec<types::Message>`
///   * `system: Option<serde_json::Value>` → `Vec<types::SystemContentBlock>`
///   * `additional_model_request_fields: serde_json::Value` →
///     `aws_smithy_types::Document`
///   * `tools` is currently always `None` on `BedrockInput` (Phase 1 work).
///
/// The `EventReceiver` returned by the SDK is drained in a spawned task and
/// each `ConverseStreamOutput` variant is mapped to the matching
/// [`BedrockEvent`] (6-way 1:1 translation). Unknown variants or per-event
/// SDK errors surface as `Err(_)` on the `ReceiverStream` and terminate the
/// drain loop.
#[must_use]
pub struct RealBedrock {
    /// Underlying Bedrock SDK client, built by [`build_client`].
    pub client: BedrockClient,
}

#[async_trait::async_trait]
impl BedrockLike for RealBedrock {
    async fn converse_stream(
        &self,
        input: BedrockInput,
    ) -> Result<ReceiverStream<Result<BedrockEvent>>> {
        // --- Step 1: serde messages → typed SDK messages ---
        let sdk_messages = sdk_translator::messages_to_sdk(&input.messages)
            .context("translate messages to SDK shape")?;
        // --- Step 2: serde system → typed SDK system blocks ---
        let sdk_system = sdk_translator::system_to_sdk(input.system.as_ref())
            .context("translate system to SDK shape")?;
        // --- Step 4: serde additionalModelRequestFields → smithy Document ---
        let amrf_doc = sdk_translator::json_to_document(&input.additional_model_request_fields);

        // Assemble the fluent call. Tool translation (Step 3) is deferred to
        // Phase 1; translator::extract_tool_defs returns None today.
        let mut fluent = self
            .client
            .converse_stream()
            .model_id(input.wire_model_id.clone())
            .set_messages(Some(sdk_messages))
            .additional_model_request_fields(amrf_doc);
        if !sdk_system.is_empty() {
            fluent = fluent.set_system(Some(sdk_system));
        }

        let output = fluent
            .send()
            .await
            .context("Bedrock ConverseStream dispatch failed")?;
        let mut stream = output.stream;

        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            loop {
                match stream.recv().await {
                    Ok(Some(sdk_event)) => {
                        let Some(bedrock_ev) = translate_output_event(sdk_event) else {
                            continue;
                        };
                        if tx.send(Ok(bedrock_ev)).await.is_err() {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx
                            .send(Err(anyhow::anyhow!("bedrock stream recv: {e}")))
                            .await;
                        return;
                    }
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }
}

/// Translate a single `ConverseStreamOutput` variant to our [`BedrockEvent`]
/// enum. Returns `None` when the SDK yields an event variant we don't yet
/// surface (`Unknown` — forward-compat) or when a delta has no payload.
///
/// The SDK types are all `#[non_exhaustive]`, which forces us to keep a
/// wildcard match arm — the explicit `Unknown` variants are not publicly
/// constructible. We silence `wildcard_enum_match_arm` at the function level
/// to document that this is deliberate forward-compat, not sloppiness.
#[allow(clippy::cast_sign_loss, clippy::wildcard_enum_match_arm)]
fn translate_output_event(sdk_event: ConverseStreamOutput) -> Option<BedrockEvent> {
    match sdk_event {
        ConverseStreamOutput::MessageStart(_) => Some(BedrockEvent::MessageStart),
        ConverseStreamOutput::ContentBlockStart(ev) => {
            let idx = u32::try_from(ev.content_block_index).unwrap_or(0);
            // Text blocks emit no start payload; only ToolUse surfaces a
            // typed payload that the accumulator needs to pick up id+name from.
            // Image/ToolResult/Unknown/None all fall through to "text" so the
            // accumulator's default block kind kicks in.
            let kind = if let Some(ContentBlockStart::ToolUse(tu)) = ev.start {
                serde_json::json!({
                    "type": "tool_use",
                    "id": tu.tool_use_id,
                    "name": tu.name,
                })
                .to_string()
            } else {
                "text".to_string()
            };
            Some(BedrockEvent::ContentBlockStart {
                block_index: idx,
                kind,
            })
        }
        ConverseStreamOutput::ContentBlockDelta(ev) => {
            let idx = u32::try_from(ev.content_block_index).unwrap_or(0);
            // Only Text / ToolUse / ReasoningContent(Text|Signature) map into
            // our BedrockEvent delta shape. Everything else (Citation, Image,
            // ToolResult, Blob reasoning, and the non_exhaustive Unknown
            // tails) is dropped via early `return None`.
            let delta_json = match ev.delta {
                Some(ContentBlockDelta::Text(t)) => {
                    serde_json::json!({ "type": "text_delta", "text": t }).to_string()
                }
                Some(ContentBlockDelta::ToolUse(tu)) => serde_json::json!({
                    "type": "input_json_delta",
                    "partial_json": tu.input,
                })
                .to_string(),
                Some(ContentBlockDelta::ReasoningContent(ReasoningContentBlockDelta::Text(t))) => {
                    serde_json::json!({ "type": "thinking_delta", "thinking": t }).to_string()
                }
                Some(ContentBlockDelta::ReasoningContent(
                    ReasoningContentBlockDelta::Signature(s),
                )) => serde_json::json!({ "type": "signature_delta", "signature": s }).to_string(),
                _ => return None,
            };
            Some(BedrockEvent::ContentBlockDelta {
                block_index: idx,
                delta_json,
            })
        }
        ConverseStreamOutput::ContentBlockStop(ev) => {
            let idx = u32::try_from(ev.content_block_index).unwrap_or(0);
            Some(BedrockEvent::ContentBlockStop { block_index: idx })
        }
        ConverseStreamOutput::MessageStop(ev) => Some(BedrockEvent::MessageStop {
            stop_reason: ev.stop_reason.as_str().to_string(),
        }),
        ConverseStreamOutput::Metadata(m) => {
            let usage = m.usage.as_ref();
            let to_u64 = |v: i32| u64::try_from(v).unwrap_or(0);
            Some(BedrockEvent::MessageStreamMetadata {
                input_tokens: usage.map_or(0, |u| to_u64(u.input_tokens)),
                output_tokens: usage.map_or(0, |u| to_u64(u.output_tokens)),
                cache_read: usage
                    .and_then(|u| u.cache_read_input_tokens)
                    .map_or(0, to_u64),
                cache_write: usage
                    .and_then(|u| u.cache_write_input_tokens)
                    .map_or(0, to_u64),
            })
        }
        // Forward-compat: ignore unknown union variants rather than erroring.
        _ => None,
    }
}
