//! E2E: Bedrock mid-stream error → synthesized `StreamFinished` ResponseEvent.
//!
//! The historical behavior (documented as a TODO in route_multi_agent.rs)
//! was to log the error and break out of the loop, producing a silent EOF
//! on the client side that is indistinguishable from a clean end-of-turn.
//! Warp's UI then renders a partial response with no error indicator,
//! leaving the user unsure whether the turn succeeded.
//!
//! The contract this test locks in: on mid-stream error, the adapter must
//! flush a synthetic `StreamFinished` frame with a non-`Done` reason BEFORE
//! closing the SSE body, so the UI can surface a "stream aborted" state.

use async_trait::async_trait;
use base64::Engine as _;
use prost::Message as _;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::ReceiverStream;
use warp_byok_proxy::{
    bedrock_client::BedrockLike,
    config::Config,
    stream_accumulator::BedrockEvent,
    translator::BedrockInput,
};
use warp_multi_agent_api as wmaa;

/// BedrockLike that emits a few events, then a stream error, then shuts down.
/// Exercises the error path in `route_multi_agent::handle`.
struct FailingBedrock;

#[async_trait]
impl BedrockLike for FailingBedrock {
    async fn converse_stream(
        &self,
        _input: BedrockInput,
    ) -> anyhow::Result<ReceiverStream<anyhow::Result<BedrockEvent>>> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            // One happy event, then simulate an upstream RPC failure.
            let _ = tx.send(Ok(BedrockEvent::MessageStart)).await;
            let _ = tx
                .send(Ok(BedrockEvent::ContentBlockStart {
                    block_index: 0,
                    kind: "text".into(),
                }))
                .await;
            let _ = tx
                .send(Ok(BedrockEvent::ContentBlockDelta {
                    block_index: 0,
                    delta_json: r#"{"type":"text_delta","text":"partial"}"#.into(),
                }))
                .await;
            let _ = tx
                .send(Err(anyhow::anyhow!(
                    "simulated upstream Bedrock stream error"
                )))
                .await;
        });
        Ok(ReceiverStream::new(rx))
    }
}

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
async fn midstream_error_emits_synthesized_stream_finished() {
    tracing_subscriber::fmt::try_init().ok();

    let cert_tmp = tempfile::tempdir().unwrap();
    let paths =
        warp_byok_proxy::cert::generate_self_signed(cert_tmp.path(), &["127.0.0.1"]).unwrap();

    let bedrock: Arc<dyn BedrockLike> = Arc::new(FailingBedrock);
    let cfg = Arc::new(minimal_config());

    let (addr, shutdown) = warp_byok_proxy::server::spawn(
        "127.0.0.1:0",
        &paths.cert_pem,
        &paths.key_pem,
        cfg,
        bedrock,
    )
    .await
    .unwrap();

    use wmaa::request::input::user_inputs::user_input as ui_oneof;
    use wmaa::request::input::user_inputs::UserInput;
    use wmaa::request::input::{self as req_input, UserInputs, UserQuery};
    use wmaa::request::Input as RequestInput;
    use wmaa::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::UserQuery(UserQuery {
                        query: "trigger a mid-stream error".into(),
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
    let body = resp.text().await.unwrap();

    // Parse every SSE frame, decode as ResponseEvent, collect the type tags.
    let events: Vec<wmaa::ResponseEvent> = body
        .split("\n\n")
        .filter_map(|chunk| chunk.strip_prefix("data: "))
        .map(|b64| {
            let bytes = base64::engine::general_purpose::URL_SAFE
                .decode(b64.trim())
                .expect("valid base64");
            wmaa::ResponseEvent::decode(&bytes[..]).expect("valid ResponseEvent")
        })
        .collect();

    // Contract 1: a `StreamFinished` ResponseEvent MUST be present.
    let finished = events
        .iter()
        .find_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::Finished(f)) => Some(f),
            _ => None,
        })
        .expect(
            "mid-stream error must produce a synthesized StreamFinished frame; \
             silent EOF is the bug this test guards against",
        );

    // Contract 2: the stop reason must NOT be `Done` — the turn did not
    // complete cleanly. `Other` is the canonical bucket for stream-aborted.
    use wmaa::response_event::stream_finished::Reason;
    match &finished.reason {
        Some(Reason::Done(_)) => panic!(
            "synthesized StreamFinished after an error must NOT use Reason::Done; \
             the UI cannot distinguish a clean end-of-turn from an aborted stream"
        ),
        Some(_) => {} // Other / MaxTokenLimit / QuotaLimit / etc. all acceptable.
        None => panic!("StreamFinished.reason must be populated"),
    }

    shutdown.send(()).ok();
}
