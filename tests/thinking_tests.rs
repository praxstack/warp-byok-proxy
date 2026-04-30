//! Thinking/reasoning shape tests.
//!
//! Real Bedrock (verified 2026-04-30 against Opus 4.7) requires the thinking
//! control to ride as TWO top-level keys: `thinking` + `output_config`.
//! Prior `reasoningConfig` blob shape was a plan pseudocode artifact; these
//! tests now exercise the Bedrock-GA shape.

use serde_json::json;
use warp_byok_proxy::config::{Effort, ThinkingMode};
use warp_byok_proxy::thinking::{build_reasoning_config, ReasoningInputs};

#[test]
fn adaptive_max_emits_thinking_plus_output_config() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Adaptive,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap();
    assert_eq!(v.thinking.unwrap(), json!({ "type": "adaptive" }));
    assert_eq!(v.output_config.unwrap(), json!({ "effort": "max" }));
}

#[test]
fn adaptive_medium_sets_effort_medium() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Adaptive,
        effort: Effort::Medium,
        budget_tokens: None,
    })
    .unwrap();
    assert_eq!(v.thinking.unwrap(), json!({ "type": "adaptive" }));
    assert_eq!(v.output_config.unwrap()["effort"], "medium");
}

#[test]
fn enabled_mode_emits_thinking_with_budget_tokens_no_output_config() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Enabled,
        effort: Effort::Max,
        budget_tokens: Some(32000),
    })
    .unwrap();
    assert_eq!(
        v.thinking.unwrap(),
        json!({ "type": "enabled", "budget_tokens": 32000 })
    );
    assert!(
        v.output_config.is_none(),
        "enabled mode must NOT emit output_config — that's adaptive-only"
    );
}

#[test]
fn enabled_mode_defaults_budget_to_16000_when_none() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Enabled,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap();
    assert_eq!(
        v.thinking.unwrap(),
        json!({ "type": "enabled", "budget_tokens": 16000 })
    );
}

#[test]
fn off_mode_emits_neither_thinking_nor_output_config() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Off,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap();
    assert!(v.thinking.is_none(), "off mode must not emit thinking");
    assert!(
        v.output_config.is_none(),
        "off mode must not emit output_config"
    );
}
