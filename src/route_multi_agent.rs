//! Route handler for `POST /ai/multi-agent`.
//!
//! Pipeline:
//!   1. Decode request body as `warp_multi_agent_api::Request` protobuf.
//!   2. `translate_warp_request(&req, &cfg)` → `BedrockInput`.
//!   3. `bedrock.converse_stream(input)` → `Stream<BedrockEvent>`.
//!   4. Per event, run it through [`StreamAccumulator::handle`] →
//!      `Vec<OzResponseFrame>`.
//!   5. Per frame, run it through [`UiAdapter::translate`] →
//!      `Vec<ResponseEvent>`.
//!   6. For each `ResponseEvent`, encode as protobuf + base64 and wrap as an
//!      SSE `data: <base64>\n\n` line.
//!   7. Stream the SSE lines back as the HTTP response body.

use anyhow::{Context, Result};
use base64::Engine as _;
use futures_util::StreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use prost::Message as _;
use std::sync::Arc;

use crate::{
    bedrock_client::BedrockLike,
    config::Config,
    stream_accumulator::StreamAccumulator,
    translator::translate_warp_request,
    ui_adapter::{UiAdapter, UiAdapterOpts},
};

/// Type alias for the boxed body used on multi-agent responses (SSE stream).
pub type BoxedBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

/// Handle an incoming `POST /ai/multi-agent` request.
///
/// Returns an SSE-streaming `Response<BoxedBody>` whose body is a sequence of
/// `data: <base64(ResponseEvent)>\n\n` lines, one per protobuf event emitted
/// by the UI adapter.
///
/// # Errors
/// Returns an error if the request body cannot be read, the protobuf decode
/// fails, or the translator rejects the request. Bedrock per-event stream
/// errors are logged and terminate the SSE body cleanly; they are not
/// propagated as `Result::Err`.
pub async fn handle(
    req: Request<Incoming>,
    cfg: Arc<Config>,
    bedrock: Arc<dyn BedrockLike>,
) -> Result<Response<BoxedBody>> {
    // Step 1 — decode protobuf body.
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .context("read body")?
        .to_bytes();
    let warp_req = warp_multi_agent_api::Request::decode(body_bytes.as_ref())
        .context("decode protobuf Request")?;

    // Step 2 — translate to Bedrock input.
    let bedrock_input = translate_warp_request(&warp_req, &cfg)?;

    // Step 3 — start the Bedrock stream.
    let mut bedrock_stream = bedrock.converse_stream(bedrock_input).await?;

    // Channel carrying hyper body frames back to the client.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(32);

    // Steps 4–6 — pump events through accumulator + adapter, emit SSE frames.
    tokio::spawn(async move {
        let mut acc = StreamAccumulator::new();
        let mut adapter = UiAdapter::new(UiAdapterOpts::default());
        while let Some(ev_res) = bedrock_stream.next().await {
            let ev = match ev_res {
                Ok(e) => e,
                Err(e) => {
                    // TODO(phase-1): emit a synthesized StreamFinished ResponseEvent
                    // with an error reason BEFORE breaking, so the Warp UI sees a
                    // structured "stream aborted" frame instead of a silent EOF.
                    // Currently the client sees the headers + whatever frames made
                    // it through, then EOF — indistinguishable from clean end-of-turn.
                    tracing::warn!(?e, "bedrock stream err; client will see silent EOF");
                    break;
                }
            };
            let frames = acc.handle(ev);
            for f in frames {
                for re in adapter.translate(&f) {
                    let bytes = encode_sse_event(&re);
                    if tx.send(Ok(Frame::data(bytes))).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    // Step 7 — wrap the receiver as an SSE body.
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = StreamBody::new(stream);
    let boxed: BoxedBody = BoxedBody::new(body);
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(boxed)
        .context("build response")?;
    Ok(resp)
}

/// Encode a `ResponseEvent` as a single SSE `data:` line (base64 + `\n\n`).
fn encode_sse_event(evt: &warp_multi_agent_api::ResponseEvent) -> Bytes {
    let mut buf = Vec::new();
    evt.encode(&mut buf).expect("encode protobuf");
    let b64 = base64::engine::general_purpose::URL_SAFE.encode(&buf);
    Bytes::from(format!("data: {b64}\n\n"))
}
