use warp_byok_proxy::config::{AuthMode, Config, Effort, ThinkingMode};

fn parse(s: &str) -> anyhow::Result<Config> {
    Ok(toml::from_str(s)?)
}

#[test]
fn parses_minimal_api_key_config() {
    let c = parse(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-opus-4-7-v1:0:1m"
    "#,
    )
    .unwrap();
    assert_eq!(c.bedrock.auth_mode, AuthMode::ApiKey);
    assert_eq!(c.bedrock.region, "us-east-1");
    assert_eq!(c.bedrock.model, "anthropic.claude-opus-4-7-v1:0:1m");
    // Defaults
    assert!(c.bedrock.use_cross_region_inference);
    assert!(c.bedrock.use_prompt_cache);
    assert!(c.bedrock.enable_1m_context);
    assert_eq!(c.bedrock.thinking.mode, ThinkingMode::Adaptive);
    assert_eq!(c.bedrock.thinking.effort, Effort::Max);
}

#[test]
fn rejects_unknown_auth_mode() {
    let err = parse(
        r#"
        [bedrock]
        auth_mode = "super-secure"
        region = "us-east-1"
        model = "m"
    "#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("super-secure") || err.to_string().contains("auth_mode"),
        "expected parse error mentioning the bad value, got: {err}"
    );
}

#[test]
fn rejects_unknown_effort() {
    let err = parse(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "m"
        [bedrock.thinking]
        mode = "adaptive"
        effort = "maximum"
    "#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("maximum") || err.to_string().contains("effort"));
}

#[test]
fn validate_1m_requires_opus_4_6_or_4_7() {
    let c = parse(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-haiku-4-5-v1:0"
        enable_1m_context = true
    "#,
    )
    .unwrap();
    let err = c.validate().unwrap_err();
    assert!(err.to_string().to_lowercase().contains("1m"));
    assert!(err.to_string().to_lowercase().contains("opus"));
}

// Plan had effort="max" which is the default; adjusted to "high" so the
// "ignored" warning fires per validate_with_warnings semantics.
#[test]
fn thinking_off_with_effort_present_warns_not_fails() {
    let c = parse(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-opus-4-7-v1:0:1m"
        [bedrock.thinking]
        mode = "off"
        effort = "high"
    "#,
    )
    .unwrap();
    // validate() must succeed but produce a warning
    let warnings = c.validate_with_warnings().unwrap();
    assert!(warnings.iter().any(|w| w.contains("thinking.mode=\"off\"")));
}
