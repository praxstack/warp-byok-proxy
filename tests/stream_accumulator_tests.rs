use warp_byok_proxy::frame::OzResponseFrame;
use warp_byok_proxy::stream_accumulator::{BedrockEvent, StreamAccumulator};

#[test]
fn text_block_start_delta_stop_emits_text_delta_and_blockstop() {
    let mut acc = StreamAccumulator::new();
    let f1 = acc.handle(BedrockEvent::ContentBlockStart {
        block_index: 0,
        kind: "text".into(),
    });
    assert!(f1.is_empty());
    let f2 = acc.handle(BedrockEvent::ContentBlockDelta {
        block_index: 0,
        delta_json: r#"{"type":"text_delta","text":"hello"}"#.into(),
    });
    assert_eq!(
        f2,
        vec![OzResponseFrame::TextDelta {
            block_index: 0,
            text: "hello".into()
        }]
    );
    let f3 = acc.handle(BedrockEvent::ContentBlockStop { block_index: 0 });
    assert_eq!(f3, vec![OzResponseFrame::BlockStop { block_index: 0 }]);
}

#[test]
fn tool_use_block_emits_partial_json_then_final_tool_use() {
    let mut acc = StreamAccumulator::new();
    acc.handle(BedrockEvent::ContentBlockStart {
        block_index: 1,
        kind: r#"{"type":"tool_use","id":"tu_1","name":"ls"}"#.into(),
    });
    let d1 = acc.handle(BedrockEvent::ContentBlockDelta {
        block_index: 1,
        delta_json: r#"{"type":"input_json_delta","partial_json":"{\"path\":\""}"#.into(),
    });
    assert!(matches!(d1[0], OzResponseFrame::ToolUseInputDelta { .. }));
    acc.handle(BedrockEvent::ContentBlockDelta {
        block_index: 1,
        delta_json: r#"{"type":"input_json_delta","partial_json":"/tmp\"}"}"#.into(),
    });
    let stop = acc.handle(BedrockEvent::ContentBlockStop { block_index: 1 });
    assert!(stop
        .iter()
        .any(|f| matches!(f, OzResponseFrame::ToolUse { id, .. } if id == "tu_1")));
}

#[test]
fn usage_metadata_emits_usage_update() {
    let mut acc = StreamAccumulator::new();
    let f = acc.handle(BedrockEvent::MessageStreamMetadata {
        input_tokens: 100,
        output_tokens: 250,
        cache_read: 50,
        cache_write: 80,
    });
    assert_eq!(
        f,
        vec![OzResponseFrame::UsageUpdate {
            input_tokens: 100,
            output_tokens: 250,
            cache_read: 50,
            cache_write: 80,
        }]
    );
}

#[test]
fn message_stop_emits_done() {
    let mut acc = StreamAccumulator::new();
    let f = acc.handle(BedrockEvent::MessageStop {
        stop_reason: "end_turn".into(),
    });
    assert_eq!(
        f,
        vec![OzResponseFrame::Done {
            stop_reason: "end_turn".into()
        }]
    );
}

#[test]
fn thinking_block_with_signature() {
    let mut acc = StreamAccumulator::new();
    acc.handle(BedrockEvent::ContentBlockStart {
        block_index: 2,
        kind: r#"{"type":"thinking"}"#.into(),
    });
    let f = acc.handle(BedrockEvent::ContentBlockDelta {
        block_index: 2,
        delta_json: r#"{"type":"thinking_delta","thinking":"reasoning ..."}"#.into(),
    });
    assert!(
        matches!(f[0], OzResponseFrame::ThinkingDelta { ref text, .. } if text == "reasoning ...")
    );
    let f2 = acc.handle(BedrockEvent::ContentBlockDelta {
        block_index: 2,
        delta_json: r#"{"type":"signature_delta","signature":"sig_abc"}"#.into(),
    });
    assert!(
        matches!(f2[0], OzResponseFrame::ThinkingDelta { signature: Some(ref s), .. } if s == "sig_abc")
    );
}
