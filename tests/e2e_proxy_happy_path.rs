//! Task 15 E2E: happy-path Warp → /ai/multi-agent → MockBedrock → SSE.
//!
//! Wires the full pipeline under MockBedrock and asserts that:
//!   * The HTTPS server responds 200 on POST /ai/multi-agent.
//!   * `content-type` is `text/event-stream`.
//!   * Body is SSE-framed `data: <base64>\n\n` lines.
//!   * The first decoded `ResponseEvent` is the adapter's synthesized prelude
//!     (StreamInit or ClientActions).

use base64::Engine as _;
use prost::Message as _;
use std::sync::Arc;
use std::time::Duration;
use warp_byok_proxy::{
    bedrock_client::{BedrockLike, MockBedrock},
    config::Config,
    stream_accumulator::BedrockEvent,
};

fn minimal_config() -> Config {
    toml::from_str(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-opus-4-7-v1:0:1m"
    "#,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_text_turn_streams_back_as_sse() {
    tracing_subscriber::fmt::try_init().ok();

    let cert_tmp = tempfile::tempdir().unwrap();
    let paths =
        warp_byok_proxy::cert::generate_self_signed(cert_tmp.path(), &["127.0.0.1"]).unwrap();

    // Scripted Bedrock stream: text block with two deltas, clean end_turn.
    let scripted = vec![
        BedrockEvent::MessageStart,
        BedrockEvent::ContentBlockStart {
            block_index: 0,
            kind: "text".into(),
        },
        BedrockEvent::ContentBlockDelta {
            block_index: 0,
            delta_json: r#"{"type":"text_delta","text":"hello "}"#.into(),
        },
        BedrockEvent::ContentBlockDelta {
            block_index: 0,
            delta_json: r#"{"type":"text_delta","text":"world"}"#.into(),
        },
        BedrockEvent::ContentBlockStop { block_index: 0 },
        BedrockEvent::MessageStop {
            stop_reason: "end_turn".into(),
        },
    ];
    let mock_bedrock: Arc<dyn BedrockLike> = Arc::new(MockBedrock { scripted });

    let cfg = Arc::new(minimal_config());

    let (addr, shutdown) = warp_byok_proxy::server::spawn(
        "127.0.0.1:0",
        &paths.cert_pem,
        &paths.key_pem,
        cfg,
        mock_bedrock,
    )
    .await
    .unwrap();

    // Build a minimal protobuf Request with a UserQuery (same layout as
    // translator_tests::translates_user_query_text_into_bedrock_messages).
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::{self as req_input, UserInputs, UserQuery};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::UserQuery(UserQuery {
                        query: "hi from test".into(),
                        ..Default::default()
                    })),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut body_bytes = Vec::new();
    req.encode(&mut body_bytes).unwrap();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = client
        .post(format!("https://{addr}/ai/multi-agent"))
        .body(body_bytes)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "text/event-stream");

    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("data: "),
        "expected SSE body, got: {body:?}"
    );
    assert!(body.contains("\n\n"), "expected SSE framing, got: {body:?}");

    // Decode the first SSE frame and sanity-check it's a ResponseEvent from
    // the adapter's prelude (StreamInit or ClientActions carrying CreateTask).
    let first_line = body.lines().next().unwrap();
    let b64 = first_line.strip_prefix("data: ").unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(b64)
        .unwrap();
    let re = warp_multi_agent_api::ResponseEvent::decode(&bytes[..]).unwrap();
    let dbg = format!("{re:?}");
    assert!(
        dbg.contains("StreamInit") || dbg.contains("ClientActions") || dbg.contains("Init"),
        "expected first ResponseEvent to be StreamInit or ClientActions, got: {dbg}"
    );

    shutdown.send(()).ok();
}
