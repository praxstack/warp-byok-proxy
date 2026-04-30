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

#[test]
fn translates_user_query_text_into_bedrock_messages() {
    // Build a minimal Request carrying a UserQuery with real prompt text.
    // Verified proto layout (from target/debug/build/.../warp.multi_agent.v1.rs):
    //   Request.input : Option<request::Input>              (message, not oneof)
    //   request::Input.r#type : Option<request::input::Type> (the oneof)
    //   request::input::Type::UserInputs(request::input::UserInputs)
    //   request::input::UserInputs.inputs : Vec<request::input::user_inputs::UserInput>
    //   request::input::user_inputs::UserInput.input :
    //       Option<request::input::user_inputs::user_input::Input>
    //   request::input::user_inputs::user_input::Input::UserQuery(request::input::UserQuery)
    //   request::input::UserQuery.query : String
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::{self as req_input, UserInputs, UserQuery};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::UserQuery(UserQuery {
                        query: "hello from prax".into(),
                        ..Default::default()
                    })),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    };

    let cfg = minimal_config();
    let out = translate_warp_request(&req, &cfg).unwrap();

    let serialized = serde_json::to_string(&out.messages).unwrap();
    assert!(
        serialized.contains("hello from prax"),
        "expected user query in translated messages, got: {serialized}"
    );
    assert!(
        !serialized.contains("PHASE0 STUB"),
        "placeholder leaked into translated output: {serialized}"
    );
}
