//! Warp `Request` → Bedrock Converse input translator.
//!
//! Composes model-id mutation ([`crate::model_id`]), beta flags
//! ([`crate::betas`]), reasoning config ([`crate::thinking`]), and cache-point
//! injection ([`crate::cache`]) into a single [`BedrockInput`] suitable for
//! feeding the Bedrock runtime client.
//!
//! Phase 0 note: prompt extraction from `req.task_context` is stubbed. The
//! real walker lands after the Day-5 audit; for now this produces a single
//! placeholder user message so the E2E round-trip is wired end-to-end.

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

    // Phase 0 stub: produce a minimal messages list. Real task_context → messages
    // translation is Day-5 work informed by the audit. For now, extract ONLY
    // UserInputs.text_content (the bare minimum to prove a prompt flows through).
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
    if let Some(r) = reasoning {
        amrf.insert("reasoningConfig".into(), r);
    }

    Ok(BedrockInput {
        wire_model_id: prep.wire_model_id,
        messages: cache_result.messages,
        system: cache_result.system,
        additional_model_request_fields: Value::Object(amrf),
        tools: extract_tool_defs(req),
    })
}

fn extract_user_messages(_req: &warp_multi_agent_api::Request) -> Vec<Value> {
    // TODO: walk req.input / req.task_context. Phase 0 stub returns a single
    // user message from whatever text we can find, so the E2E round-trip works.
    // Replace after the audit.
    vec![json!({
        "role": "user",
        "content": [{"type": "text", "text": "[PHASE0 STUB: prompt extraction incomplete]"}]
    })]
}

fn extract_system_prompt(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}

fn extract_tool_defs(_req: &warp_multi_agent_api::Request) -> Option<Value> {
    None
}
