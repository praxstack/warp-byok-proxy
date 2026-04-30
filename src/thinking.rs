use crate::config::{Effort, ThinkingMode};
use anyhow::Result;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ReasoningInputs {
    pub mode: ThinkingMode,
    pub effort: Effort,
    pub budget_tokens: Option<u32>,
}

/// Build the `reasoningConfig` JSON fragment from thinking inputs.
///
/// # Errors
/// Currently infallible but returns `Result` to reserve space for future
/// validation (e.g., budget bounds, effort/mode compatibility checks).
pub fn build_reasoning_config(inp: &ReasoningInputs) -> Result<Option<Value>> {
    Ok(match inp.mode {
        ThinkingMode::Off => None,
        ThinkingMode::Adaptive => Some(json!({
            "type": "adaptive",
            "maxReasoningEffort": effort_str(inp.effort),
        })),
        ThinkingMode::Enabled => Some(json!({
            "type": "enabled",
            "budgetTokens": inp.budget_tokens.unwrap_or(16000),
        })),
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
