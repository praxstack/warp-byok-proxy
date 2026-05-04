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
    let amrf = out.additional_model_request_fields.to_string();
    assert!(
        amrf.contains("context-1m-2025-08-07"),
        "1M beta missing: {amrf}"
    );
    // Bedrock-GA shape: `thinking` + `output_config` top-level keys
    // (not the old plan-only `reasoningConfig` blob).
    assert!(
        amrf.contains("\"thinking\""),
        "thinking key missing: {amrf}"
    );
    assert!(
        amrf.contains("\"output_config\""),
        "output_config key missing: {amrf}"
    );
    assert!(amrf.contains("adaptive"), "adaptive mode missing: {amrf}");
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

// ---------------------------------------------------------------------------
// Extended input-variant walker coverage (Slice 3 of the 2026-05 audit
// follow-up). The Phase-0 walker only handled UserInputs→UserQuery, which
// meant any client that sent one of the other 9 top-level Input variants
// fell through to the "[PHASE0 WALKER: no UserQuery found]" diagnostic stub
// and got a garbage turn. These tests lock in three additional variants that
// are known to ship real prompt text.
// ---------------------------------------------------------------------------

#[test]
#[allow(deprecated)]
fn translates_deprecated_top_level_user_query() {
    // Older Warp clients (and the ToolCallResult-heavy flows) use the deprecated
    // top-level `Input::UserQuery` (field #2) instead of wrapping in UserInputs.
    // The proto still defines it, prost still decodes it, and real captured
    // traffic in docs/warp-client-behavior-audit-stub.md shows it in use on
    // resume/continuation turns. Treat it as a first-class single-query input.
    use warp_multi_agent_api::request::input::{self as req_input, UserQuery};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserQuery(UserQuery {
                query: "legacy top-level query path".into(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    };

    let cfg = minimal_config();
    let out = translate_warp_request(&req, &cfg).unwrap();
    let serialized = serde_json::to_string(&out.messages).unwrap();
    assert!(
        serialized.contains("legacy top-level query path"),
        "deprecated top-level UserQuery must be walked; got {serialized}"
    );
    assert!(
        !serialized.contains("PHASE0 WALKER"),
        "fallback stub leaked; top-level UserQuery not recognized: {serialized}"
    );
}

#[test]
fn translates_auto_code_diff_query() {
    // `AutoCodeDiffQuery` fires when Warp detects compilation errors in the
    // last run block and wants the agent to explain the diff. The proto
    // carries the trigger text in `query`. We route it as a plain user turn.
    use warp_multi_agent_api::request::input::{self as req_input, AutoCodeDiffQuery};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::AutoCodeDiffQuery(AutoCodeDiffQuery {
                query: "auto diff explanation please".into(),
            })),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = minimal_config();
    let out = translate_warp_request(&req, &cfg).unwrap();
    let serialized = serde_json::to_string(&out.messages).unwrap();
    assert!(
        serialized.contains("auto diff explanation please"),
        "AutoCodeDiffQuery must be walked; got {serialized}"
    );
}

#[test]
fn translates_user_inputs_tool_call_result_as_tool_result_block() {
    // When the user turn carries a ToolCallResult (the response to a prior
    // assistant tool_use), the walker must surface it as a Claude-shaped
    // `tool_result` block on a user message, not drop it. The proto's inner
    // `result` oneof has 32+ variants, so instead of per-variant marshaling
    // we JSON-serialize the ToolCallResult via prost-reflect and hand that
    // structured blob to Claude.
    use warp_multi_agent_api::request::input::tool_call_result::Result as TcrResult;
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::{self as req_input, ToolCallResult, UserInputs};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::{Request, RunShellCommandResult};

    #[allow(deprecated)]
    let shell = RunShellCommandResult {
        command: "echo hi".into(),
        output: "Hello, world!\n".into(),
        exit_code: 0,
        result: None,
    };
    let tcr = ToolCallResult {
        tool_call_id: "call_shell_1".into(),
        result: Some(TcrResult::RunShellCommand(shell)),
    };
    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::ToolCallResult(tcr)),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = minimal_config();
    let out = translate_warp_request(&req, &cfg).unwrap();
    let serialized = serde_json::to_string(&out.messages).unwrap();

    // Contract 1: a tool_result block with the matching tool_call_id is present.
    assert!(
        serialized.contains("\"type\":\"tool_result\"")
            || serialized.contains("\"tool_result\""),
        "expected tool_result block in messages; got {serialized}"
    );
    assert!(
        serialized.contains("call_shell_1"),
        "tool_call_id must be preserved; got {serialized}"
    );
    // Contract 2: the fallback diagnostic stub must NOT leak.
    assert!(
        !serialized.contains("PHASE0 WALKER"),
        "ToolCallResult path fell through to stub: {serialized}"
    );
}

#[test]
fn translates_prior_task_messages_into_assistant_history() {
    // Continuation turns carry prior conversation in `task_context.tasks[].messages[]`.
    // The walker must surface prior assistant `agent_output` messages so Claude
    // sees them as history, otherwise every turn looks like a fresh start and
    // the model loses context for follow-up questions.
    use warp_multi_agent_api as wmaa;
    use warp_multi_agent_api::request::input::user_inputs::user_input as ui_oneof;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::{self as req_input, UserInputs, UserQuery};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::request::TaskContext;
    use warp_multi_agent_api::Request;

    // Build one prior assistant message carrying agent_output text.
    let prior_msg = wmaa::Message {
        id: "m1".into(),
        task_id: "t1".into(),
        message: Some(wmaa::message::Message::AgentOutput(
            wmaa::message::AgentOutput {
                text: "I ran ls and found two files.".into(),
            },
        )),
        ..Default::default()
    };
    let task = wmaa::Task {
        id: "t1".into(),
        messages: vec![prior_msg],
        ..Default::default()
    };

    let req = Request {
        task_context: Some(TaskContext {
            tasks: vec![task],
        }),
        input: Some(RequestInput {
            r#type: Some(req_input::Type::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(ui_oneof::Input::UserQuery(UserQuery {
                        query: "great, what's in the second file?".into(),
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

    // Contract: both the prior assistant turn AND the new user question
    // survive into the translated messages array, in order.
    let ap = serialized
        .find("I ran ls and found two files.")
        .expect("prior assistant text must be walked into history");
    let up = serialized
        .find("great, what's in the second file?")
        .expect("current user query must be walked");
    assert!(
        ap < up,
        "assistant history must come before new user query; got indices {ap} vs {up} in {serialized}"
    );
    // The assistant turn must be tagged `role: assistant`.
    let before_up = &serialized[..up];
    assert!(
        before_up.contains("\"role\":\"assistant\""),
        "prior agent_output must be emitted with role=assistant; got {before_up}"
    );
}

#[test]
fn translates_query_with_canned_response_query_field() {
    // `QueryWithCannedResponse` carries a `query` string alongside the canned
    // variant tag. Even when we do not honor the canned response branch,
    // surfacing the user-typed `query` text keeps the BYOP proxy usable for
    // zero-state chips ("Install", "Code", "Deploy", ...).
    use warp_multi_agent_api::request::input::{self as req_input, QueryWithCannedResponse};
    use warp_multi_agent_api::request::Input as RequestInput;
    use warp_multi_agent_api::Request;

    let req = Request {
        input: Some(RequestInput {
            r#type: Some(req_input::Type::QueryWithCannedResponse(
                QueryWithCannedResponse {
                    query: "help me install docker".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = minimal_config();
    let out = translate_warp_request(&req, &cfg).unwrap();
    let serialized = serde_json::to_string(&out.messages).unwrap();
    assert!(
        serialized.contains("help me install docker"),
        "QueryWithCannedResponse.query must be walked; got {serialized}"
    );
}
