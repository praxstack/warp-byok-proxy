use warp_byok_proxy::config::Config;
use warp_byok_proxy::translator::translate_warp_request;

fn minimal_config() -> Config {
    toml::from_str(
        r#"
        [bedrock]
        auth_mode = "api-key"
        region = "us-east-1"
        model = "anthropic.claude-opus-4-7-v1:0:1m"
    "#,
    )
    .unwrap()
}

#[test]
fn translates_empty_request_to_bedrock_input_with_model_and_betas() {
    let cfg = minimal_config();
    let req = warp_multi_agent_api::Request::default();
    let out = translate_warp_request(&req, &cfg).unwrap();
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
    assert!(out
        .additional_model_request_fields
        .to_string()
        .contains("context-1m-2025-08-07"));
    assert!(out
        .additional_model_request_fields
        .to_string()
        .contains("reasoningConfig"));
}
