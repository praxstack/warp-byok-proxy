//! Bedrock `additional_model_request_fields` builder for Anthropic's
//! adaptive / enabled / off thinking modes.
//!
//! Real Bedrock (verified against Opus 4.7 on 2026-04-30) requires the
//! thinking control to ride as TWO separate top-level keys:
//!   * `thinking`: `{ "type": "adaptive" }` or `{ "type": "enabled", "budget_tokens": N }`
//!   * `output_config`: `{ "effort": "low|medium|high|max" }` (paired with adaptive)
//!
//! Prior to this shape, the plan used a single `reasoningConfig` blob — Bedrock
//! rejects that with "Extra inputs are not permitted; use thinking.type.adaptive
//! and output_config.effort". The plan's pseudocode predated the GA shape.

use crate::config::{Effort, ThinkingMode};
use anyhow::Result;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ReasoningInputs {
    pub mode: ThinkingMode,
    pub effort: Effort,
    pub budget_tokens: Option<u32>,
}

/// Bedrock-shaped thinking fields, to be merged into the top level of
/// `additional_model_request_fields`.
#[derive(Debug, Clone, Default)]
pub struct ThinkingFields {
    /// `{"type":"adaptive"}` or `{"type":"enabled","budget_tokens":N}`. `None` means "off".
    pub thinking: Option<Value>,
    /// `{"effort":"max"}` etc. Paired with `thinking.type=adaptive`; `None` for enabled/off.
    pub output_config: Option<Value>,
}

/// Build the `thinking` + `output_config` pair from thinking inputs.
///
/// # Errors
/// Currently infallible but returns `Result` to reserve space for future
/// validation (e.g., budget bounds, effort/mode compatibility checks).
pub fn build_reasoning_config(inp: &ReasoningInputs) -> Result<ThinkingFields> {
    Ok(match inp.mode {
        ThinkingMode::Off => ThinkingFields::default(),
        ThinkingMode::Adaptive => ThinkingFields {
            thinking: Some(json!({ "type": "adaptive" })),
            output_config: Some(json!({ "effort": effort_str(inp.effort) })),
        },
        ThinkingMode::Enabled => ThinkingFields {
            thinking: Some(json!({
                "type": "enabled",
                "budget_tokens": inp.budget_tokens.unwrap_or(16000),
            })),
            output_config: None,
        },
    })
}

fn effort_str(e: Effort) -> &'static str {
    match e {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
        Effort::Max => "max",
    }
}
