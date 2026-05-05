//! Warp `Request` → Bedrock Converse input translator.
//!
//! Composes model-id mutation ([`crate::model_id`]), beta flags
//! ([`crate::betas`]), reasoning config ([`crate::thinking`]), and cache-point
//! injection ([`crate::cache`]) into a single [`BedrockInput`] suitable for
//! feeding the Bedrock runtime client.
//!
//! Phase 0 note: prompt extraction is minimal — it walks
//! `req.input → UserInputs → UserQuery` and emits one user message per query.
//! The full `task_context` / tool-result walker (resume, `code_review`,
//! `invoke_skill`, summarize, `messages_from_agents`, etc.) lands in Phase 1.
//! When no `UserQuery` is present the walker falls back to a diagnostic stub
//! message so the gap is visible in Task 17's smoke test.

use crate::{betas, cache, config::Config, model_id, thinking};
use anyhow::Result;
use serde_json::{json, Value};

/// Fully-translated Bedrock Converse input.
#[must_use]
#[derive(Debug, Clone)]
pub struct BedrockInput {
    /// The model ID as it should appear on the Bedrock wire (CRI/global prefix applied).
    pub wire_model_id: String,
    /// Bedrock Converse `messages` array.
    pub messages: Vec<Value>,
    /// Optional Bedrock Converse `system` array.
    pub system: Option<Value>,
    /// Bedrock Converse `additionalModelRequestFields` object (betas + reasoningConfig).
    pub additional_model_request_fields: Value,
    /// Optional Bedrock Converse `toolConfig`/tools array.
    pub tools: Option<Value>,
}

/// Translate a Warp `Request` into a [`BedrockInput`].
///
/// # Errors
/// Propagates errors from [`crate::model_id::prepare_model_id`] and
/// [`crate::thinking::build_reasoning_config`].
pub fn translate_warp_request(
    req: &warp_multi_agent_api::Request,
    cfg: &Config,
) -> Result<BedrockInput> {
    let prep = model_id::prepare_model_id(
        &cfg.bedrock.model,
        &model_id::PrepareOpts {
            use_cross_region_inference: cfg.bedrock.use_cross_region_inference,
            use_global_inference: cfg.bedrock.use_global_inference,
            region_hint: &cfg.bedrock.region,
        },
    )?;
    let betas = betas::build_betas(prep.opus_1m, &[]);
    let reasoning = thinking::build_reasoning_config(&thinking::ReasoningInputs {
        mode: cfg.bedrock.thinking.mode,
        effort: cfg.bedrock.thinking.effort,
        budget_tokens: cfg.bedrock.thinking.budget_tokens,
    })?;

    // Post-Phase-0 walker: pulls prior task history from task_context +
    // current-turn user/tool-result blocks from Request.input. See
    // `build_messages` below.
    let messages = build_messages(req);
    let system = extract_system_prompt(req);

    // Apply cache points.
    let cache_result = cache::apply_cache_points(cache::CacheInputs {
        enabled: cfg.bedrock.use_prompt_cache,
        messages,
        system,
    });

    let mut amrf = serde_json::Map::new();
    if !betas.is_empty() {
        amrf.insert("anthropic_beta".into(), json!(betas));
    }
    if let Some(t) = reasoning.thinking {
        amrf.insert("thinking".into(), t);
    }
    if let Some(o) = reasoning.output_config {
        amrf.insert("output_config".into(), o);
    }

    Ok(BedrockInput {
        wire_model_id: prep.wire_model_id,
        messages: cache_result.messages,
        system: cache_result.system,
        additional_model_request_fields: Value::Object(amrf),
        // Tool schemas live on the server side in the config (Warp doesn't
        // ship them on the wire); `BedrockInput.tools` stays `None` here
        // and `RealBedrock::converse_stream` loads them from `cfg.bedrock.tools`
        // via `sdk_translator::tools_to_sdk` before dispatching.
        tools: extract_tool_defs(req),
    })
}

/// Compose the full Bedrock `messages` array: prior history from
/// `task_context.tasks[*].messages[*]` (walked via [`walk_prior_messages`])
/// followed by the current turn's input (walked via [`extract_user_messages`]).
fn build_messages(req: &warp_multi_agent_api::Request) -> Vec<Value> {
    let mut out = walk_prior_messages(req);
    out.extend(extract_user_messages(req));
    if out.is_empty() {
        tracing::warn!(
            "translator: no prior history AND no text-bearing input variant matched; \
             falling back to diagnostic stub"
        );
        out.push(json!({
            "role": "user",
            "content": [{"type": "text", "text": "[PHASE0 WALKER: no UserQuery found in request.input]"}]
        }));
    }
    out
}

/// Walk every prior `task_context.tasks[*].messages[*]` into a Bedrock-shaped
/// Claude message, preserving order. Each `Message.oneof message` variant we
/// know the role of (`user_query`, `agent_output`, `agent_reasoning`,
/// `tool_call`, `tool_call_result`) is surfaced; empty or unknown variants
/// are skipped.
///
/// Tool-use / tool-result blocks on prior turns are encoded as Claude
/// `tool_use` / `tool_result` blocks so a stateful tool loop can resume mid-
/// trajectory on a continuation turn.
fn walk_prior_messages(req: &warp_multi_agent_api::Request) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(tc) = req.task_context.as_ref() else {
        return out;
    };
    for task in &tc.tasks {
        for msg in &task.messages {
            if let Some(v) = prior_message_to_json(msg) {
                out.push(v);
            }
        }
    }
    out
}

/// Translate one prior `api::Message` into a Bedrock-shaped Claude message.
/// Returns `None` for variants that don't belong in the conversation history
/// (UI-only metadata: `ServerEvent`, `SystemQuery`, `UpdateTodos`, ...) or
/// for empty text payloads.
///
/// Implemented as an `if let` chain rather than a `match` over the 18-variant
/// oneof because the crate is `#![deny(clippy::wildcard_enum_match_arm)]` and
/// listing every UI-only arm individually would be pure noise.
#[allow(deprecated)]
fn prior_message_to_json(msg: &warp_multi_agent_api::Message) -> Option<Value> {
    use warp_multi_agent_api::message::Message as MsgOneof;
    let inner = msg.message.as_ref()?;
    if let MsgOneof::UserQuery(uq) = inner {
        if !uq.query.trim().is_empty() {
            return Some(json!({
                "role": "user",
                "content": [{"type": "text", "text": uq.query.clone()}]
            }));
        }
    } else if let MsgOneof::AgentOutput(a) = inner {
        if !a.text.trim().is_empty() {
            return Some(json!({
                "role": "assistant",
                "content": [{"type": "text", "text": a.text.clone()}]
            }));
        }
    } else if let MsgOneof::AgentReasoning(r) = inner {
        // Reasoning rides alongside assistant output; flatten into an
        // assistant text block so Claude sees it in the history.
        if !r.reasoning.trim().is_empty() {
            return Some(json!({
                "role": "assistant",
                "content": [{"type": "text", "text": r.reasoning.clone()}]
            }));
        }
    } else if let MsgOneof::ToolCall(tc_msg) = inner {
        // Prior assistant tool_use — surface as a Claude tool_use block so
        // continuation turns carry the correct trajectory.
        let payload = dyn_msg_to_json(tc_msg);
        let name = payload
            .as_object()
            .and_then(|o| o.get("tool"))
            .and_then(serde_json::Value::as_object)
            .and_then(|o| o.keys().next().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        return Some(json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": tc_msg.tool_call_id.clone(),
                "name": name,
                "input": payload,
            }]
        }));
    } else if let MsgOneof::ToolCallResult(tcr) = inner {
        // Prior user tool_result — JSON-serializing the full proto gives
        // Claude the structured payload for all 32+ result variants without
        // per-variant marshaling.
        let body = dyn_msg_to_json(tcr);
        return Some(json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tcr.tool_call_id.clone(),
                "content": [{"type": "json", "json": body}],
            }]
        }));
    }
    // Everything else (ServerEvent, SystemQuery, UpdateTodos, InvokeSkill,
    // Summarization, CodeReview, WebSearch, ...) is UI-only metadata that
    // does NOT belong in the Bedrock conversation history.
    None
}

/// Serialize any `prost::Message` with a prost-reflect descriptor into a
/// `serde_json::Value`. Uses proto3 canonical JSON encoding (camelCase field
/// names, well-known types serialized as their JSON form).
///
/// Falls back to `Value::Null` on the very-rare case the transcode fails,
/// rather than erroring — a missing tool-call payload should not block the
/// rest of the turn from going through.
fn dyn_msg_to_json<M: prost_reflect::ReflectMessage>(m: &M) -> Value {
    use prost_reflect::DynamicMessage;
    let desc = m.descriptor();
    let mut dyn_msg = DynamicMessage::new(desc);
    if dyn_msg.transcode_from(m).is_err() {
        return Value::Null;
    }
    serde_json::to_value(&dyn_msg).unwrap_or(Value::Null)
}

// The deprecated top-level `Input::UserQuery` variant (proto field #2) is
// still emitted by older Warp clients on resume/continuation turns. The
// `#[allow(deprecated)]` is load-bearing: dropping it would break real-world
// compatibility and is explicitly out of scope for this slice.
#[allow(deprecated)]
fn extract_user_messages(req: &warp_multi_agent_api::Request) -> Vec<Value> {
    // Walker covers the Input variants known to ship user-typed prompt text.
    // The `Request.input.type` oneof has ~11 variants; the ones below are the
    // text-bearing branches. Empty/metadata-only variants (ResumeConversation,
    // InitProjectRules, FetchReviewComments, StartFromAmbientRunPrompt) do
    // not carry a query — they rely on task_context for the real work and
    // are correctly handled by the empty-messages path today.
    //
    // Covered variants (field numbers in the proto parentheses):
    //   • UserInputs         (#6)  → each UserInput oneof:
    //                                  - UserQuery.query
    //                                  - CLIAgentUserQuery.user_query.query
    //   • UserQuery          (#2, deprecated) — older clients / resume turns
    //   • AutoCodeDiffQuery  (#5) — compile-error explanations
    //   • QueryWithCannedResponse (#4) — zero-state chips w/ user-typed text
    //   • CreateNewProject   (#10) — description rides in `query`
    //
    // Intentionally NOT walked here (deferred to the Phase-A task_context
    // walker): ToolCallResult branches, SummarizeConversation, CodeReview,
    // InvokeSkill — these feed structured context, not raw user prompts,
    // and need variant-specific marshaling into Bedrock ContentBlocks.
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::Type as InputType;

    let mut messages = Vec::new();

    let push_user_text = |messages: &mut Vec<Value>, text: &str| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": [{"type": "text", "text": text.to_string()}]
            }));
        }
    };

    if let Some(input) = req.input.as_ref() {
        match input.r#type.as_ref() {
            Some(InputType::UserInputs(user_inputs)) => {
                for ui in &user_inputs.inputs {
                    match ui.input.as_ref() {
                        Some(ui_oneof::Input::UserQuery(uq)) => {
                            push_user_text(&mut messages, &uq.query);
                        }
                        Some(ui_oneof::Input::CliAgentUserQuery(cli)) => {
                            if let Some(uq) = cli.user_query.as_ref() {
                                push_user_text(&mut messages, &uq.query);
                            }
                        }
                        Some(ui_oneof::Input::ToolCallResult(tcr)) => {
                            // Current-turn tool_result: surface as a Claude
                            // `tool_result` block. The inner proto payload is
                            // JSON-serialized so all 32+ result variants
                            // round-trip without per-variant marshaling.
                            let body = dyn_msg_to_json(tcr);
                            messages.push(json!({
                                "role": "user",
                                "content": [{
                                    "type": "tool_result",
                                    "tool_use_id": tcr.tool_call_id.clone(),
                                    "content": [{"type": "json", "json": body}],
                                }]
                            }));
                        }
                        // MessagesReceivedFromAgents / EventsFromAgents /
                        // PassiveSuggestionResult stay Phase-A scope — they
                        // carry metadata, not a user turn.
                        _ => {}
                    }
                }
            }
            Some(InputType::UserQuery(uq)) => {
                // Deprecated top-level UserQuery (proto field #2). Still in
                // use by older clients on resume/continuation turns.
                push_user_text(&mut messages, &uq.query);
            }
            Some(InputType::AutoCodeDiffQuery(q)) => {
                push_user_text(&mut messages, &q.query);
            }
            Some(InputType::QueryWithCannedResponse(q)) => {
                push_user_text(&mut messages, &q.query);
            }
            Some(InputType::CreateNewProject(q)) => {
                push_user_text(&mut messages, &q.query);
            }
            _ => {}
        }
    }

    // No fallback here — `build_messages` handles the empty-prior-and-empty-
    // input case with a single diagnostic stub so we don't double-stub.
    messages
}

fn extract_system_prompt(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}

fn extract_tool_defs(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}
