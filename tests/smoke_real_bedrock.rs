//! Task 17 smoke test — ignored by default so CI never calls real AWS.
//!
//! Run manually with a real Bedrock API key:
//!
//! ```bash
//! AWS_BEARER_TOKEN_BEDROCK=<key> \
//!   cargo nextest run --test smoke_real_bedrock --run-ignored all
//! ```
//!
//! Expected: PASS. Produces token latencies in tracing logs once the
//! `RealBedrock::converse_stream` impl replaces today's `todo!` stub.
//!
//! Phase 0 scope: ship the `#[ignore]` skeleton so the invocation path is
//! wired. Full `RealBedrock` conversion from `serde_json::Value` messages /
//! system / tools into the SDK's strongly-typed `Message` / `ContentBlock` /
//! `SystemContentBlock` / `ToolConfiguration` builders + translation from
//! `ConverseStreamOutput` SDK events back into our `BedrockEvent` enum lands
//! as a follow-up. See the TODO on `bedrock_client::RealBedrock::converse_stream`.
//!
//! Keeping the skeleton green today proves:
//!   * the `#[ignore]` gate compiles and is skipped by default nextest runs
//!   * our `auth::resolve_auth` accepts an `AWS_BEARER_TOKEN_BEDROCK`-style
//!     bearer input (i.e. the wiring between env → `AuthInputs` →
//!     `ResolvedAuth::BearerToken` is exercised end-to-end)
//!   * the `Config` toml parses the exact production-shaped stanza (api-key
//!     auth, 1m-context model, adaptive thinking at max effort)

use warp_byok_proxy::{auth, config::Config};

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Run with: AWS_BEARER_TOKEN_BEDROCK=... cargo nextest run --test smoke_real_bedrock --run-ignored all
async fn opus_4_7_1m_max_thinking_streams_tokens() {
    let api_key = std::env::var("AWS_BEARER_TOKEN_BEDROCK").expect("set AWS_BEARER_TOKEN_BEDROCK");
    let cfg: Config = toml::from_str(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-opus-4-7-v1:0:1m"
        [bedrock.thinking]
        mode = "adaptive"
        effort = "max"
    "#,
    )
    .expect("parse config");

    let inputs = auth::AuthInputs {
        mode: auth::AuthMode::ApiKey,
        api_key: Some(api_key),
        region: Some(cfg.bedrock.region.clone()),
        ..Default::default()
    };
    let resolved = auth::resolve_auth(&inputs).expect("resolve auth");

    // Phase 0 skeleton assertion: prove auth/config wiring end-to-end.
    // The bearer token must round-trip into `ResolvedAuth::BearerToken(_)`
    // and the model id must be the exact Opus 4.7 1m variant the plan targets.
    // Once `RealBedrock::converse_stream` lands, replace the `matches!` below
    // with a live stream-drain and a `ContentBlockDelta` assertion.
    assert!(
        matches!(resolved, auth::ResolvedAuth::BearerToken(_)),
        "api-key auth must resolve to ResolvedAuth::BearerToken"
    );
    assert_eq!(cfg.bedrock.model, "anthropic.claude-opus-4-7-v1:0:1m");

    // TODO(task-17-followup): once `RealBedrock::converse_stream` lands the
    // real SDK plumbing, expand this test to:
    //   1. Build a `warp_multi_agent_api::Request` whose
    //      `input.type = UserInputs{UserQuery{query: "say hi"}}`.
    //   2. Call `translate_warp_request(&req, &cfg)` → `BedrockInput`.
    //   3. Build a `BedrockClient` via `bedrock_client::build_client(&resolved,
    //      &cfg.bedrock.region, cfg.bedrock.endpoint.as_deref())`, wrap in
    //      `RealBedrock { client }`.
    //   4. Await `real.converse_stream(input)`, drain 5 events, and assert at
    //      least one `BedrockEvent::ContentBlockDelta` with a `text_delta`
    //      payload arrived — that's the "tokens are streaming" signal.
    //
    // Today `RealBedrock::converse_stream` is `todo!()`, so this skeleton
    // exercises only the auth+config path. Keeping the file wired lets a
    // future session land only the body, not the plumbing.
    eprintln!("smoke skeleton — implement with current aws-sdk API once RealBedrock is real");
}
