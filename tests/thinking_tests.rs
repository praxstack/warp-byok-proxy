use serde_json::json;
use warp_byok_proxy::config::{Effort, ThinkingMode};
use warp_byok_proxy::thinking::{build_reasoning_config, ReasoningInputs};

#[test]
fn adaptive_max_is_standard_shape() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Adaptive,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        v,
        json!({
            "type": "adaptive",
            "maxReasoningEffort": "max"
        })
    );
}

#[test]
fn adaptive_medium() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Adaptive,
        effort: Effort::Medium,
        budget_tokens: None,
    })
    .unwrap()
    .unwrap();
    assert_eq!(v["maxReasoningEffort"], "medium");
}

#[test]
fn enabled_mode_with_budget_tokens() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Enabled,
        effort: Effort::Max,
        budget_tokens: Some(32000),
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        v,
        json!({
            "type": "enabled",
            "budgetTokens": 32000
        })
    );
}

#[test]
fn enabled_mode_defaults_budget_when_none() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Enabled,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        v,
        json!({
            "type": "enabled",
            "budgetTokens": 16000
        })
    );
}

#[test]
fn off_mode_returns_none() {
    let v = build_reasoning_config(&ReasoningInputs {
        mode: ThinkingMode::Off,
        effort: Effort::Max,
        budget_tokens: None,
    })
    .unwrap();
    assert!(v.is_none());
}
