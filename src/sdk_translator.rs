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
    ToolResultBlock, ToolResultContentBlock, ToolResultStatus, ToolUseBlock,
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
/// Returns `Ok(None)` when the block is well-formed but not a variant we
/// surface (image, document, `search_result`, audio, video, citations,
/// guardrail, ...). Upstream callers log and skip the `None` case so a
/// mixed-content array still produces a valid Message as long as at least
/// one block translates.
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
    if let Some(tu) = build_tool_use_block(block)? {
        return Ok(Some(ContentBlock::ToolUse(tu)));
    }
    if let Some(tr) = build_tool_result_block(block)? {
        return Ok(Some(ContentBlock::ToolResult(tr)));
    }
    Ok(None)
}

/// Translate a Claude-shaped
/// `{"type":"tool_use","id":...,"name":...,"input":{...}}`
/// block into an SDK [`ToolUseBlock`]. Returns `Ok(None)` if the block is
/// not a `tool_use`; returns `Err` if the block is `tool_use` but malformed
/// (missing `id`/`name`).
fn build_tool_use_block(block: &Value) -> Result<Option<ToolUseBlock>> {
    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
        return Ok(None);
    }
    let id = block
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tool_use: missing 'id'"))?;
    let name = block
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tool_use: missing 'name'"))?;
    // Missing `input` → empty object (tool schema with no parameters).
    let input_doc = block
        .get("input")
        .map_or_else(|| Document::Object(std::collections::HashMap::new()), json_to_document);
    let tu = ToolUseBlock::builder()
        .tool_use_id(id)
        .name(name)
        .input(input_doc)
        .build()
        .map_err(|e| anyhow!("tool_use build: {e}"))?;
    Ok(Some(tu))
}

/// Translate a Claude-shaped
/// `{"type":"tool_result","tool_use_id":...,"content":[...],"is_error":bool}`
/// block into an SDK [`ToolResultBlock`]. Each content entry becomes a
/// [`ToolResultContentBlock`] — `text` → `Text`, `json` → `Json`, unknown
/// entries are dropped with a warning. `is_error: true` → `Status::Error`,
/// otherwise `Status::Success`.
fn build_tool_result_block(block: &Value) -> Result<Option<ToolResultBlock>> {
    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
        return Ok(None);
    }
    let id = block
        .get("tool_use_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tool_result: missing 'tool_use_id'"))?;
    let empty: Vec<Value> = Vec::new();
    let content_arr = block
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut content: Vec<ToolResultContentBlock> = Vec::with_capacity(content_arr.len());
    for (i, entry) in content_arr.iter().enumerate() {
        match entry.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = entry.get("text").and_then(Value::as_str) {
                    content.push(ToolResultContentBlock::Text(t.to_string()));
                }
            }
            Some("json") => {
                if let Some(j) = entry.get("json") {
                    content.push(ToolResultContentBlock::Json(json_to_document(j)));
                }
            }
            _ => {
                tracing::warn!(
                    index = i,
                    entry = %entry,
                    "sdk_translator: dropping unsupported tool_result content entry"
                );
            }
        }
    }
    let status = match block.get("is_error").and_then(Value::as_bool) {
        Some(true) => Some(ToolResultStatus::Error),
        _ => Some(ToolResultStatus::Success),
    };
    let mut tr_builder = ToolResultBlock::builder().tool_use_id(id);
    for c in content {
        tr_builder = tr_builder.content(c);
    }
    if let Some(s) = status {
        tr_builder = tr_builder.status(s);
    }
    let tr = tr_builder
        .build()
        .map_err(|e| anyhow!("tool_result build: {e}"))?;
    Ok(Some(tr))
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
