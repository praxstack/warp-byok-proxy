//! Unit tests for `sdk_translator` — the pure-function bridge between our
//! `serde_json::Value` shaped `BedrockInput` and the strongly-typed
//! `aws-sdk-bedrockruntime` request shape. These run without any AWS
//! credentials or network access.

use aws_sdk_bedrockruntime::types::{
    CachePointType, ContentBlock, ConversationRole, SystemContentBlock,
};
use aws_smithy_types::{Document, Number};
use serde_json::json;
use warp_byok_proxy::sdk_translator::{json_to_document, messages_to_sdk, system_to_sdk};

#[test]
fn json_to_document_handles_primitives_and_null() {
    assert!(matches!(json_to_document(&json!(null)), Document::Null));
    assert!(matches!(
        json_to_document(&json!(true)),
        Document::Bool(true)
    ));
    assert!(matches!(
        json_to_document(&json!("hello")),
        Document::String(s) if s == "hello"
    ));
    assert!(matches!(
        json_to_document(&json!(42u64)),
        Document::Number(Number::PosInt(42))
    ));
    assert!(matches!(
        json_to_document(&json!(-7)),
        Document::Number(Number::NegInt(-7))
    ));
    assert!(matches!(
        json_to_document(&json!(3.25_f64)),
        Document::Number(Number::Float(_))
    ));
}

#[test]
fn json_to_document_recurses_into_arrays_and_objects() {
    let v = json!({
        "reasoningConfig": {
            "type": "enabled",
            "budgetTokens": 32_000u64,
        },
        "anthropic_beta": ["context-1m-2025-08-07", "interleaved-thinking-2025-05-14"],
    });
    let doc = json_to_document(&v);
    let Document::Object(map) = doc else {
        panic!("expected object");
    };
    let rc = map.get("reasoningConfig").expect("reasoningConfig present");
    let Document::Object(rc_map) = rc else {
        panic!("expected reasoningConfig object");
    };
    assert!(matches!(rc_map.get("type"), Some(Document::String(s)) if s == "enabled"));
    assert!(matches!(
        rc_map.get("budgetTokens"),
        Some(Document::Number(Number::PosInt(32_000)))
    ));
    let betas = map.get("anthropic_beta").expect("anthropic_beta present");
    let Document::Array(a) = betas else {
        panic!("expected array");
    };
    assert_eq!(a.len(), 2);
    assert!(matches!(&a[0], Document::String(s) if s == "context-1m-2025-08-07"));
}

#[test]
fn messages_to_sdk_translates_text_and_cache_point_blocks() {
    let msgs = vec![json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "hello world"},
            {"cachePoint": {"type": "default"}},
        ]
    })];
    let out = messages_to_sdk(&msgs).expect("ok");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].role, ConversationRole::User);
    assert_eq!(out[0].content.len(), 2);
    match &out[0].content[0] {
        ContentBlock::Text(s) => assert_eq!(s, "hello world"),
        other => panic!("expected Text, got {other:?}"),
    }
    match &out[0].content[1] {
        ContentBlock::CachePoint(cp) => {
            assert_eq!(cp.r#type, CachePointType::Default);
        }
        other => panic!("expected CachePoint, got {other:?}"),
    }
}

#[test]
fn messages_to_sdk_parses_assistant_role() {
    let msgs = vec![json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "hi back"}]
    })];
    let out = messages_to_sdk(&msgs).unwrap();
    assert_eq!(out[0].role, ConversationRole::Assistant);
}

#[test]
fn messages_to_sdk_errors_on_unknown_role() {
    let msgs = vec![json!({
        "role": "system", // not a valid ConversationRole
        "content": [{"type": "text", "text": "x"}]
    })];
    let err = messages_to_sdk(&msgs).unwrap_err();
    assert!(
        err.to_string().contains("unknown role 'system'"),
        "unexpected error: {err}"
    );
}

#[test]
fn messages_to_sdk_skips_unknown_blocks_silently() {
    // Well-formed but unsupported block types (image, document, ...) should
    // be warn!-skipped, not errored, so a mixed content array still produces
    // a valid Message as long as at least one supported block is present.
    let msgs = vec![json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "before"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}},
            {"type": "text", "text": "after"},
        ]
    })];
    let out = messages_to_sdk(&msgs).unwrap();
    assert_eq!(
        out[0].content.len(),
        2,
        "image block dropped, two text blocks remain"
    );
}

// ---------------------------------------------------------------------------
// Tool-use / tool-result translation (completes option 2 of the
// 2026-05 audit follow-up). Claude's serde shape lands on these cases:
//
//   assistant → user request:
//     {"type":"tool_use","id":"call_xyz","name":"run_shell","input":{"cmd":"ls"}}
//
//   user → assistant reply:
//     {"type":"tool_result","tool_use_id":"call_xyz",
//      "content":[{"type":"text","text":"total 0\n"}],
//      "is_error":false}
//
// Both MUST translate into their strongly-typed SDK counterparts
// (ContentBlock::ToolUse / ContentBlock::ToolResult) or Bedrock rejects the
// full request with a validation error. Previously these were silently
// dropped via the "unknown block" path, which is what forced the
// "text-only" scope limitation in the README.
// ---------------------------------------------------------------------------

#[test]
fn messages_to_sdk_translates_tool_use_block() {
    let msgs = vec![json!({
        "role": "assistant",
        "content": [
            {
                "type": "tool_use",
                "id": "call_abc",
                "name": "run_shell",
                "input": {"cmd": "ls -la /tmp", "timeout": 30}
            }
        ]
    })];
    let out = messages_to_sdk(&msgs).expect("ok");
    assert_eq!(out[0].content.len(), 1);
    match &out[0].content[0] {
        ContentBlock::ToolUse(tu) => {
            assert_eq!(tu.tool_use_id(), "call_abc");
            assert_eq!(tu.name(), "run_shell");
            // input is a smithy Document — verify structure.
            let Document::Object(obj) = tu.input() else {
                panic!("input should be Document::Object, got {:?}", tu.input());
            };
            assert!(matches!(obj.get("cmd"), Some(Document::String(s)) if s == "ls -la /tmp"));
            assert!(matches!(
                obj.get("timeout"),
                Some(Document::Number(Number::PosInt(30)))
            ));
        }
        other => panic!("expected ContentBlock::ToolUse, got {other:?}"),
    }
}

#[test]
fn messages_to_sdk_translates_tool_result_block_with_text() {
    let msgs = vec![json!({
        "role": "user",
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": "call_abc",
                "content": [
                    {"type": "text", "text": "total 0\nfoo.txt\n"}
                ],
                "is_error": false
            }
        ]
    })];
    let out = messages_to_sdk(&msgs).expect("ok");
    assert_eq!(out[0].content.len(), 1);
    match &out[0].content[0] {
        ContentBlock::ToolResult(tr) => {
            assert_eq!(tr.tool_use_id(), "call_abc");
            assert_eq!(tr.content().len(), 1);
            use aws_sdk_bedrockruntime::types::{ToolResultContentBlock, ToolResultStatus};
            match &tr.content()[0] {
                ToolResultContentBlock::Text(t) => assert_eq!(t, "total 0\nfoo.txt\n"),
                other => panic!("expected ToolResultContentBlock::Text, got {other:?}"),
            }
            // is_error: false should map to ToolResultStatus::Success.
            assert_eq!(tr.status(), Some(&ToolResultStatus::Success));
        }
        other => panic!("expected ContentBlock::ToolResult, got {other:?}"),
    }
}

#[test]
fn messages_to_sdk_tool_result_is_error_maps_to_status_error() {
    let msgs = vec![json!({
        "role": "user",
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": "call_err",
                "content": [
                    {"type": "text", "text": "command not found: xyzzy"}
                ],
                "is_error": true
            }
        ]
    })];
    let out = messages_to_sdk(&msgs).unwrap();
    use aws_sdk_bedrockruntime::types::ToolResultStatus;
    match &out[0].content[0] {
        ContentBlock::ToolResult(tr) => {
            assert_eq!(tr.status(), Some(&ToolResultStatus::Error));
        }
        _ => panic!("expected ToolResult"),
    }
}

#[test]
fn messages_to_sdk_tool_result_carries_json_content() {
    // A JSON payload on a tool_result must land on ToolResultContentBlock::Json
    // so downstream Claude gets structured data rather than a stringified blob.
    let msgs = vec![json!({
        "role": "user",
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": "call_json",
                "content": [
                    {"type": "json", "json": {"files": ["a.txt", "b.txt"], "count": 2}}
                ],
            }
        ]
    })];
    let out = messages_to_sdk(&msgs).unwrap();
    use aws_sdk_bedrockruntime::types::ToolResultContentBlock;
    match &out[0].content[0] {
        ContentBlock::ToolResult(tr) => match &tr.content()[0] {
            ToolResultContentBlock::Json(doc) => {
                let Document::Object(obj) = doc else {
                    panic!("json content should be Object, got {doc:?}");
                };
                assert!(matches!(obj.get("count"), Some(Document::Number(Number::PosInt(2)))));
            }
            other => panic!("expected Json, got {other:?}"),
        },
        _ => panic!("expected ToolResult"),
    }
}

#[test]
fn system_to_sdk_translates_text_and_cache_point() {
    let system = json!([
        {"type": "text", "text": "you are helpful"},
        {"cachePoint": {"type": "default"}},
    ]);
    let out = system_to_sdk(Some(&system)).unwrap();
    assert_eq!(out.len(), 2);
    match &out[0] {
        SystemContentBlock::Text(s) => assert_eq!(s, "you are helpful"),
        other => panic!("expected Text, got {other:?}"),
    }
    match &out[1] {
        SystemContentBlock::CachePoint(cp) => {
            assert_eq!(cp.r#type, CachePointType::Default);
        }
        other => panic!("expected CachePoint, got {other:?}"),
    }
}

#[test]
fn system_to_sdk_returns_empty_for_none_or_non_array() {
    assert!(system_to_sdk(None).unwrap().is_empty());
    assert!(system_to_sdk(Some(&json!("not an array")))
        .unwrap()
        .is_empty());
}
