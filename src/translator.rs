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

    // Phase 0 minimal-real walker: pulls UserQuery.query text out of
    // Request.input → UserInputs. See `extract_user_messages` below. The full
    // task_context walker (resume, code_review, invoke_skill, summarize,
    // tool results, messages_from_agents, ...) lands in Phase 1.
    let messages = extract_user_messages(req);
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
        tools: extract_tool_defs(req),
    })
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
                        // ToolCallResult / MessagesReceivedFromAgents / etc.
                        // are Phase-A work — they need structured marshaling
                        // rather than plain text injection.
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

    // Fallback: if the walker found no queries (unsupported input variant or
    // empty UserInputs), emit a diagnostic stub so the turn surfaces the gap
    // clearly rather than silently sending nothing.
    if messages.is_empty() {
        tracing::warn!(
            "translator: no text-bearing input variant matched; falling back to diagnostic stub"
        );
        messages.push(json!({
            "role": "user",
            "content": [{"type": "text", "text": "[PHASE0 WALKER: no UserQuery found in request.input]"}]
        }));
    }

    messages
}

fn extract_system_prompt(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}

fn extract_tool_defs(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}
