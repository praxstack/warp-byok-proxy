//! Pure-function bridges between our `serde_json::Value` shaped
//! [`crate::translator::BedrockInput`] and the `aws-sdk-bedrockruntime`
//! strongly-typed request shape (`Message`, `ContentBlock`,
//! `SystemContentBlock`, `aws_smithy_types::Document`).
//!
//! These helpers are intentionally pure — no AWS I/O — so they can be
//! exercised by unit tests without touching the network. The stream-output
//! side (`ConverseStreamOutput` → [`crate::stream_accumulator::BedrockEvent`])
//! lives inline in `bedrock_client::RealBedrock::converse_stream` so it can
//! own the mutable streaming loop state.

use anyhow::{anyhow, Result};
use aws_sdk_bedrockruntime::types::{
    CachePointBlock, CachePointType, ContentBlock, ConversationRole, Message, SystemContentBlock,
};
use aws_smithy_types::{Document, Number};
use serde_json::Value;

/// Recursively convert a `serde_json::Value` into an `aws_smithy_types::Document`.
///
/// Used for Bedrock `additionalModelRequestFields`. Numbers are mapped as
/// follows:
/// * fits in `i64` and is negative → [`Number::NegInt`]
/// * fits in `u64` → [`Number::PosInt`]
/// * otherwise (floats, or out-of-range) → [`Number::Float`]
#[must_use]
pub fn json_to_document(v: &Value) -> Document {
    match v {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(*b),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Document::Number(Number::PosInt(u))
            } else if let Some(i) = n.as_i64() {
                Document::Number(Number::NegInt(i))
            } else {
                Document::Number(Number::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => Document::String(s.clone()),
        Value::Array(arr) => Document::Array(arr.iter().map(json_to_document).collect()),
        Value::Object(obj) => Document::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_document(v)))
                .collect(),
        ),
    }
}

/// Translate the serde-shaped `BedrockInput.messages` into the SDK's typed
/// `Vec<Message>`.
///
/// Supported block variants in Phase 0:
///   * `{"type":"text","text":"..."}` → [`ContentBlock::Text`]
///   * `{"cachePoint":{"type":"default"}}` → [`ContentBlock::CachePoint`]
///
/// Unknown block shapes (`tool_use`, `tool_result`, image, document, ...) are
/// logged with `tracing::warn!` and skipped — Phase 1 extends coverage.
///
/// # Errors
/// Returns an error if a message has an unknown `role` or if the SDK
/// builder rejects the assembled `Message` (missing required fields).
pub fn messages_to_sdk(messages: &[Value]) -> Result<Vec<Message>> {
    let mut out = Vec::with_capacity(messages.len());
    for (idx, msg) in messages.iter().enumerate() {
        let role_str = msg
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("message[{idx}]: missing 'role'"))?;
        let role = parse_role(role_str)
            .ok_or_else(|| anyhow!("message[{idx}]: unknown role '{role_str}'"))?;
        let empty: Vec<Value> = Vec::new();
        let content_arr = msg
            .get("content")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let mut builder = Message::builder().role(role);
        for (b_idx, block) in content_arr.iter().enumerate() {
            match block_to_content(block) {
                Ok(Some(cb)) => builder = builder.content(cb),
                Ok(None) => {
                    tracing::warn!(
                        message_index = idx,
                        block_index = b_idx,
                        block = %block,
                        "sdk_translator: skipping unsupported content block (Phase 0)"
                    );
                }
                Err(e) => {
                    return Err(anyhow!("message[{idx}].content[{b_idx}]: {e}"));
                }
            }
        }
        out.push(builder.build()?);
    }
    Ok(out)
}

/// Translate the serde-shaped `BedrockInput.system` into the SDK's typed
/// `Vec<SystemContentBlock>`. Returns an empty vec when `system` is `None`
/// or not an array.
///
/// Supported entries: `{"type":"text","text":"..."}` and
/// `{"cachePoint":{"type":"default"}}`.
///
/// # Errors
/// Returns an error if a cache-point block fails to build (missing type).
pub fn system_to_sdk(system: Option<&Value>) -> Result<Vec<SystemContentBlock>> {
    let Some(arr) = system.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, block) in arr.iter().enumerate() {
        if let Some(text) = extract_text_block(block) {
            out.push(SystemContentBlock::Text(text));
            continue;
        }
        if is_cache_point_block(block) {
            let cp = CachePointBlock::builder()
                .r#type(CachePointType::Default)
                .build()
                .map_err(|e| anyhow!("system[{idx}]: cache point build: {e}"))?;
            out.push(SystemContentBlock::CachePoint(cp));
            continue;
        }
        tracing::warn!(
            index = idx,
            block = %block,
            "sdk_translator: skipping unsupported system content block (Phase 0)"
        );
    }
    Ok(out)
}

fn parse_role(s: &str) -> Option<ConversationRole> {
    match s {
        "user" => Some(ConversationRole::User),
        "assistant" => Some(ConversationRole::Assistant),
        _ => None,
    }
}

/// Map a single serde content block to an SDK [`ContentBlock`].
///
/// Returns `Ok(None)` when the block is well-formed but not supported in
/// Phase 0 (`tool_use`, `tool_result`, image, document, ...).
fn block_to_content(block: &Value) -> Result<Option<ContentBlock>> {
    if let Some(text) = extract_text_block(block) {
        return Ok(Some(ContentBlock::Text(text)));
    }
    if is_cache_point_block(block) {
        let cp = CachePointBlock::builder()
            .r#type(CachePointType::Default)
            .build()
            .map_err(|e| anyhow!("cache point build: {e}"))?;
        return Ok(Some(ContentBlock::CachePoint(cp)));
    }
    Ok(None)
}

fn extract_text_block(block: &Value) -> Option<String> {
    let ty = block.get("type").and_then(Value::as_str)?;
    if ty != "text" {
        return None;
    }
    block
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn is_cache_point_block(block: &Value) -> bool {
    block.get("cachePoint").and_then(Value::as_object).is_some()
}
