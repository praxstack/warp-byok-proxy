//! Task 17b smoke test — ignored by default so CI never calls real AWS.
//!
//! Run manually with a real Bedrock API key:
//!
//! ```bash
//! AWS_BEARER_TOKEN_BEDROCK=<key> \
//!   cargo nextest run --test smoke_real_bedrock --run-ignored all
//! ```
//!
//! What this proves end-to-end when `AWS_BEARER_TOKEN_BEDROCK` is set:
//!   * `Config` TOML parses the production-shaped stanza (api-key auth,
//!     1m-context Opus 4.7 model, adaptive thinking at max effort).
//!   * `auth::resolve_auth` round-trips the env bearer into
//!     `ResolvedAuth::BearerToken(_)`.
//!   * `translate_warp_request` produces a `BedrockInput` with a real
//!     `UserQuery` prompt.
//!   * `bedrock_client::build_client` constructs a working SDK client, and
//!     `RealBedrock::converse_stream` dispatches a real ConverseStream call
//!     against Bedrock.
//!   * The server streams back at least one `BedrockEvent::ContentBlockDelta`
//!     (tokens are flowing) and a `BedrockEvent::MessageStop` (clean turn end).
//!
//! The test gates on a 30s wall-clock timeout via `tokio::time::timeout` so
//! a wedged stream fails loudly rather than hanging CI.

use std::time::Duration;

use futures_util::StreamExt;
use warp_byok_proxy::{
    auth,
    bedrock_client::{self, BedrockLike, RealBedrock},
    config::Config,
    stream_accumulator::BedrockEvent,
    translator::translate_warp_request,
};

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Run with: AWS_BEARER_TOKEN_BEDROCK=... cargo nextest run --test smoke_real_bedrock --run-ignored all
async fn opus_4_7_1m_max_thinking_streams_tokens() {
    let api_key = std::env::var("AWS_BEARER_TOKEN_BEDROCK").expect("set AWS_BEARER_TOKEN_BEDROCK");
    let cfg: Config = toml::from_str(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        # NOTE: real Bedrock model ID for Opus 4.7 is `anthropic.claude-opus-4-7`
        # (no `-v1:0` suffix). Our `:1m` marker is stripped by model_id.rs and
        # the CRI prefix turns this into `us.anthropic.claude-opus-4-7` on the
        # wire — which is a valid system-defined inference profile.
        # The 1M context is NOT in the model id itself; it rides as
        # `anthropic_beta: ["context-1m-2025-08-07"]` in additional_model_request_fields.
        model = "anthropic.claude-opus-4-7:1m"
        [bedrock.thinking]
        mode = "adaptive"
        effort = "max"
    "#,
    )
    .expect("parse config");

    let inputs = auth::AuthInputs {
        mode: auth::AuthMode::ApiKey,
        api_key: Some(api_key.clone()),
        region: Some(cfg.bedrock.region.clone()),
        ..Default::default()
    };
    let resolved = auth::resolve_auth(&inputs).expect("resolve auth");
    assert!(
        matches!(resolved, auth::ResolvedAuth::BearerToken(_)),
        "api-key auth must resolve to ResolvedAuth::BearerToken"
    );
    assert_eq!(cfg.bedrock.model, "anthropic.claude-opus-4-7:1m");

    // Bedrock's AWS SDK (1.x) picks up AWS_BEARER_TOKEN_BEDROCK from the
    // process environment and injects it as an Authorization: Bearer header
    // on each Bedrock request — the env var is already set (we just read it
    // above), so we only need to build the client with the resolved region.
    let client = bedrock_client::build_client(
        &resolved,
        &cfg.bedrock.region,
        cfg.bedrock.endpoint.as_deref(),
    )
    .await
    .expect("build bedrock client");
    let real = RealBedrock {
        client,
        tool_config: None,
    };

    // Build a minimal warp request with a real UserQuery.
    let req = build_user_query_request("Say hi in 5 words exactly.");
    let bedrock_input = translate_warp_request(&req, &cfg).expect("translate warp request");

    // Drain the stream with a 30s timeout. Count deltas; ensure we saw at
    // least one ContentBlockDelta and one MessageStop.
    let drain = async {
        let mut stream = real
            .converse_stream(bedrock_input)
            .await
            .expect("converse_stream dispatch");
        let mut saw_delta = false;
        let mut saw_stop = false;
        let mut event_count = 0u32;
        while let Some(ev_res) = stream.next().await {
            let ev = ev_res.expect("per-event stream error");
            event_count += 1;
            match ev {
                BedrockEvent::ContentBlockDelta { .. } => {
                    saw_delta = true;
                }
                BedrockEvent::MessageStop { stop_reason } => {
                    eprintln!("smoke: MessageStop reason={stop_reason}");
                    saw_stop = true;
                }
                BedrockEvent::MessageStreamMetadata {
                    input_tokens,
                    output_tokens,
                    cache_read,
                    cache_write,
                } => {
                    eprintln!(
                        "smoke: usage input={input_tokens} output={output_tokens} cache_read={cache_read} cache_write={cache_write}"
                    );
                }
                BedrockEvent::MessageStart
                | BedrockEvent::ContentBlockStart { .. }
                | BedrockEvent::ContentBlockStop { .. } => {}
            }
        }
        (saw_delta, saw_stop, event_count)
    };

    let (saw_delta, saw_stop, event_count) = tokio::time::timeout(Duration::from_secs(30), drain)
        .await
        .expect("stream did not complete within 30s");
    eprintln!("smoke: drained {event_count} events");
    assert!(
        saw_delta,
        "expected at least one ContentBlockDelta — no tokens streamed"
    );
    assert!(saw_stop, "expected MessageStop — stream ended without one");
}

fn build_user_query_request(prompt: &str) -> warp_multi_agent_api::Request {
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::{Type as InputType, UserInputs, UserQuery};
    use warp_multi_agent_api::request::Input as RequestInput;

    warp_multi_agent_api::Request {
        input: Some(RequestInput {
            r#type: Some(InputType::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::UserQuery(UserQuery {
                        query: prompt.to_string(),
                        ..Default::default()
                    })),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}
