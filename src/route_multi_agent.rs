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
    let req_id = uuid::Uuid::new_v4();
    tracing::info!(%req_id, bytes = body_bytes.len(), "POST /ai/multi-agent received");
    let warp_req = warp_multi_agent_api::Request::decode(body_bytes.as_ref())
        .context("decode protobuf Request")?;
    // Dump the top-level shape — what kind of Input are we seeing?
    tracing::debug!(
        %req_id,
        input_kind = ?warp_req.input.as_ref().and_then(|i| i.r#type.as_ref()).map(std::mem::discriminant),
        "decoded Request"
    );

    // Step 2 — translate to Bedrock input.
    let bedrock_input = translate_warp_request(&warp_req, &cfg)?;
    tracing::info!(
        %req_id,
        model = %bedrock_input.wire_model_id,
        n_messages = bedrock_input.messages.len(),
        has_system = bedrock_input.system.is_some(),
        amrf = %bedrock_input.additional_model_request_fields,
        "translated to Bedrock input"
    );

    // Step 3 — start the Bedrock stream.
    let mut bedrock_stream = bedrock.converse_stream(bedrock_input).await?;

    // Channel carrying hyper body frames back to the client.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(32);

    // Steps 4–6 — pump events through accumulator + adapter, emit SSE frames.
    tokio::spawn(async move {
        let mut acc = StreamAccumulator::new();
        let mut adapter = UiAdapter::new(UiAdapterOpts::default());
        let mut event_count = 0u32;
        let mut frame_count = 0u32;
        let mut sse_count = 0u32;
        while let Some(ev_res) = bedrock_stream.next().await {
            let ev = match ev_res {
                Ok(e) => e,
                Err(e) => {
                    // TODO(phase-1): emit a synthesized StreamFinished ResponseEvent
                    // with an error reason BEFORE breaking, so the Warp UI sees a
                    // structured "stream aborted" frame instead of a silent EOF.
                    // Currently the client sees the headers + whatever frames made
                    // it through, then EOF — indistinguishable from clean end-of-turn.
                    tracing::warn!(%req_id, ?e, "bedrock stream err; client will see silent EOF");
                    break;
                }
            };
            event_count += 1;
            tracing::debug!(%req_id, event_count, ?ev, "bedrock event");
            let frames = acc.handle(ev);
            for f in frames {
                frame_count += 1;
                tracing::debug!(%req_id, frame_count, frame = ?f, "frame emitted");
                for re in adapter.translate(&f) {
                    sse_count += 1;
                    tracing::debug!(
                        %req_id, sse_count,
                        re_type = ?re.r#type.as_ref().map(std::mem::discriminant),
                        "SSE frame built"
                    );
                    let bytes = encode_sse_event(&re);
                    if tx.send(Ok(Frame::data(bytes))).await.is_err() {
                        tracing::warn!(%req_id, "client dropped connection mid-stream");
                        return;
                    }
                }
            }
        }
        tracing::info!(
            %req_id, event_count, frame_count, sse_count,
            "turn complete"
        );
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
