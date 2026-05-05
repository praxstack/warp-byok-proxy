use warp_byok_proxy::frame::OzResponseFrame;
use warp_byok_proxy::ui_adapter::{UiAdapter, UiAdapterOpts};
use warp_multi_agent_api as wmaa;

// ---------------------------------------------------------------------------
// Existing Debug-substring smoke tests (cheap; kept for regression coverage).
// ---------------------------------------------------------------------------

#[test]
fn text_delta_produces_append_to_message_content_action() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "hi".into(),
    });
    assert!(!events.is_empty(), "expected at least one ResponseEvent");
    let dbg = format!("{events:?}");
    assert!(
        dbg.to_lowercase().contains("appendtomessagecontent")
            || dbg.contains("AppendToMessageContent"),
        "expected AppendToMessageContent in {dbg}"
    );
}

#[test]
fn first_turn_emits_stream_init_and_create_task() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "first".into(),
    });
    let dbg = format!("{events:?}");
    assert!(
        dbg.contains("StreamInit") || dbg.to_lowercase().contains("stream_init"),
        "first event should be StreamInit"
    );
    assert!(
        dbg.contains("CreateTask") || dbg.to_lowercase().contains("create_task"),
        "second event should be CreateTask"
    );
}

#[test]
fn done_emits_stream_finished() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::Done {
        stop_reason: "end_turn".into(),
    });
    let dbg = format!("{events:?}");
    assert!(
        dbg.contains("StreamFinished") || dbg.to_lowercase().contains("stream_finished"),
        "expected StreamFinished in {dbg}"
    );
}

// ---------------------------------------------------------------------------
// Structural tests: drill into the real protobuf shape rather than grepping
// Debug output. These are the source of truth for Phase-0 wire fidelity.
// ---------------------------------------------------------------------------

/// Finds the first `AppendToMessageContent` action in a batch of events and
/// returns its inner `Message`.
fn first_append_message(events: &[wmaa::ResponseEvent]) -> &wmaa::Message {
    for e in events {
        let Some(wmaa::response_event::Type::ClientActions(ca)) = &e.r#type else {
            continue;
        };
        for act in &ca.actions {
            if let Some(wmaa::client_action::Action::AppendToMessageContent(a)) = &act.action {
                if let Some(m) = &a.message {
                    return m;
                }
            }
        }
    }
    panic!("no AppendToMessageContent.message found in {events:#?}");
}

#[test]
fn text_delta_populates_message_oneof_agent_output() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "hi".into(),
    });
    let msg = first_append_message(&events);
    // server_message_data is for opaque client-roundtrip only and MUST NOT
    // carry rendered text.
    assert!(
        msg.server_message_data.is_empty(),
        "server_message_data should be empty; got {:?}",
        msg.server_message_data
    );
    match &msg.message {
        Some(wmaa::message::Message::AgentOutput(ao)) => {
            assert_eq!(ao.text, "hi");
        }
        other => panic!("expected AgentOutput variant, got {other:?}"),
    }
}

#[test]
fn thinking_delta_populates_message_oneof_agent_reasoning() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::ThinkingDelta {
        block_index: 0,
        text: "plotting".into(),
        signature: None,
    });
    let msg = first_append_message(&events);
    assert!(
        msg.server_message_data.is_empty(),
        "server_message_data should be empty for thinking; got {:?}",
        msg.server_message_data
    );
    match &msg.message {
        Some(wmaa::message::Message::AgentReasoning(ar)) => {
            assert_eq!(ar.reasoning, "plotting");
            assert!(
                ar.finished_duration.is_none(),
                "finished_duration should be None mid-stream"
            );
        }
        other => panic!("expected AgentReasoning variant, got {other:?}"),
    }
}

#[test]
fn tool_use_populates_message_oneof_tool_call() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::ToolUse {
        block_index: 0,
        id: "call_abc".into(),
        name: "search".into(),
        input: serde_json::json!({"q": "foo"}),
    });
    let msg = first_append_message(&events);
    assert!(
        msg.server_message_data.is_empty(),
        "server_message_data should be empty for tool_use; got {:?}",
        msg.server_message_data
    );
    match &msg.message {
        Some(wmaa::message::Message::ToolCall(tc)) => {
            assert_eq!(tc.tool_call_id, "call_abc");
            // Phase 0: we encode {id, name, input} into the generic
            // `Server { payload }` variant as a JSON blob. Richer
            // variant-specific mapping is deferred to Phase 1.
            match &tc.tool {
                Some(wmaa::message::tool_call::Tool::Server(s)) => {
                    let parsed: serde_json::Value = serde_json::from_str(&s.payload)
                        .expect("Server.payload must be valid JSON");
                    assert_eq!(parsed["id"], "call_abc");
                    assert_eq!(parsed["name"], "search");
                    assert_eq!(parsed["input"]["q"], "foo");
                }
                other => panic!("expected Server tool variant, got {other:?}"),
            }
        }
        other => panic!("expected ToolCall variant, got {other:?}"),
    }
}

/// UsageUpdate is no longer eagerly emitted as its own ClientAction; it is
/// accumulated into adapter state and flushed onto `StreamFinished.token_usage`
/// when `Done` arrives. Asserts both halves of that contract in one test.
#[test]
fn usage_update_is_deferred_until_done() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());

    // UsageUpdate BEFORE Done: no ClientAction/UpdateTaskServerData emission.
    let usage_events = a.translate(&OzResponseFrame::UsageUpdate {
        input_tokens: 11,
        output_tokens: 22,
        cache_read: 3,
        cache_write: 4,
    });
    // StreamInit + CreateTask prelude may still fire on first frame, but there
    // must be NO action emitted that carries UpdateTaskServerData.
    for e in &usage_events {
        if let Some(wmaa::response_event::Type::ClientActions(ca)) = &e.r#type {
            for act in &ca.actions {
                assert!(
                    !matches!(
                        act.action,
                        Some(wmaa::client_action::Action::UpdateTaskServerData(_))
                    ),
                    "UsageUpdate must NOT emit UpdateTaskServerData; got {act:?}"
                );
            }
        }
    }

    // Done AFTER UsageUpdate: StreamFinished.token_usage reflects the usage.
    let done_events = a.translate(&OzResponseFrame::Done {
        stop_reason: "end_turn".into(),
    });
    let finished = done_events
        .iter()
        .find_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::Finished(f)) => Some(f),
            _ => None,
        })
        .expect("Done must emit a StreamFinished ResponseEvent");
    assert_eq!(
        finished.token_usage.len(),
        1,
        "exactly one TokenUsage entry expected, got {:#?}",
        finished.token_usage
    );
    let tu = &finished.token_usage[0];
    assert_eq!(tu.total_input, 11, "total_input");
    assert_eq!(tu.output, 22, "output");
    assert_eq!(tu.input_cache_read, 3, "input_cache_read");
    assert_eq!(tu.input_cache_write, 4, "input_cache_write");
}

#[test]
fn done_without_usage_update_has_empty_token_usage() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::Done {
        stop_reason: "end_turn".into(),
    });
    let finished = events
        .iter()
        .find_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::Finished(f)) => Some(f),
            _ => None,
        })
        .expect("Done must emit a StreamFinished ResponseEvent");
    assert!(
        finished.token_usage.is_empty(),
        "no preceding UsageUpdate; token_usage should be empty, got {:#?}",
        finished.token_usage
    );
}

#[test]
fn usage_update_last_write_wins_across_multiple_emissions() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let _ = a.translate(&OzResponseFrame::UsageUpdate {
        input_tokens: 1,
        output_tokens: 1,
        cache_read: 1,
        cache_write: 1,
    });
    let _ = a.translate(&OzResponseFrame::UsageUpdate {
        input_tokens: 99,
        output_tokens: 100,
        cache_read: 0,
        cache_write: 0,
    });
    let done = a.translate(&OzResponseFrame::Done {
        stop_reason: "end_turn".into(),
    });
    let finished = done
        .iter()
        .find_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::Finished(f)) => Some(f),
            _ => None,
        })
        .expect("Done must emit a StreamFinished ResponseEvent");
    assert_eq!(finished.token_usage.len(), 1);
    let tu = &finished.token_usage[0];
    assert_eq!(tu.total_input, 99);
    assert_eq!(tu.output, 100);
    assert_eq!(tu.input_cache_read, 0);
    assert_eq!(tu.input_cache_write, 0);
}

// ---------------------------------------------------------------------------
// FieldMask path assertions — rooted at `Message` descriptor, NO `message.`
// prefix. The oneof wrapper `message` is not a field in the descriptor, so
// a `message.X` path silently no-ops in `field_mask` apply_path, leaving the
// target unchanged and the UI blank. (Verified via src/bin/test_fieldmask.rs.)
// ---------------------------------------------------------------------------

fn extract_mask_paths(ev: &wmaa::ResponseEvent) -> Vec<String> {
    let Some(wmaa::response_event::Type::ClientActions(ca)) = &ev.r#type else {
        return Vec::new();
    };
    ca.actions
        .iter()
        .flat_map(|a| match &a.action {
            Some(wmaa::client_action::Action::AppendToMessageContent(append)) => append
                .mask
                .as_ref()
                .map(|m| m.paths.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn text_delta_append_mask_has_no_message_prefix() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "hi".into(),
    });
    let paths: Vec<String> = events.iter().flat_map(extract_mask_paths).collect();
    assert_eq!(
        paths,
        vec!["agent_output.text".to_string()],
        "FieldMask path must be rooted at `agent_output`, NOT `message.agent_output` \
         — the oneof wrapper is not a field (verified via test_fieldmask bin). \
         got: {paths:?}"
    );
}

#[test]
fn thinking_delta_append_mask_has_no_message_prefix() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::ThinkingDelta {
        block_index: 0,
        text: "reasoning".into(),
        signature: None,
    });
    let paths: Vec<String> = events.iter().flat_map(extract_mask_paths).collect();
    assert_eq!(paths, vec!["agent_reasoning.reasoning".to_string()]);
}

#[test]
fn tool_use_append_mask_has_no_message_prefix() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::ToolUse {
        block_index: 0,
        id: "tu_1".into(),
        name: "ls".into(),
        input: serde_json::json!({"path": "/tmp"}),
    });
    let paths: Vec<String> = events.iter().flat_map(extract_mask_paths).collect();
    assert_eq!(paths, vec!["tool_call.server.payload".to_string()]);
}

// ---------------------------------------------------------------------------
// Block-kind segmentation — Slice 1 of Phase 3.
//
// `Message.message` is a proto3 oneof, so a single Message proto can carry
// EXACTLY ONE of {AgentOutput, AgentReasoning, ToolCall}. Reusing the same
// `message_id` across a kind change means Warp's
// `append_to_message_content` handler applies the FieldMask against a message
// whose oneof is already locked to a different variant — the target field
// doesn't exist on the current descriptor, so the apply silently no-ops and
// the UI renders nothing for the second kind.
//
// Contract: whenever the block kind changes, the adapter MUST:
//   1. Rotate `message_id` to a new uuid.
//   2. Emit `AddMessagesToTask` with the new id and an empty instance of the
//      target oneof variant, BEFORE the kind's first `AppendToMessageContent`.
// ---------------------------------------------------------------------------

fn collect_append_message_ids(events: &[wmaa::ResponseEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::ClientActions(ca)) => ca
                .actions
                .iter()
                .filter_map(|a| match &a.action {
                    Some(wmaa::client_action::Action::AppendToMessageContent(app)) => {
                        app.message.as_ref().map(|m| m.id.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn count_add_messages_actions(events: &[wmaa::ResponseEvent]) -> usize {
    events
        .iter()
        .filter_map(|e| match &e.r#type {
            Some(wmaa::response_event::Type::ClientActions(ca)) => Some(ca),
            _ => None,
        })
        .flat_map(|ca| ca.actions.iter())
        .filter(|a| {
            matches!(
                a.action,
                Some(wmaa::client_action::Action::AddMessagesToTask(_))
            )
        })
        .count()
}

#[test]
fn kind_change_rotates_message_id_text_then_thinking() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let mut all = Vec::new();
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "hello".into(),
    }));
    all.extend(a.translate(&OzResponseFrame::ThinkingDelta {
        block_index: 1,
        text: "musing".into(),
        signature: None,
    }));
    let ids = collect_append_message_ids(&all);
    assert_eq!(ids.len(), 2, "expected one append per kind, got {ids:?}");
    assert_ne!(
        ids[0], ids[1],
        "message_id MUST rotate on kind change (text→thinking); got same id: {ids:?}"
    );
}

#[test]
fn kind_change_rotates_message_id_thinking_then_text() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let mut all = Vec::new();
    all.extend(a.translate(&OzResponseFrame::ThinkingDelta {
        block_index: 0,
        text: "pondering".into(),
        signature: None,
    }));
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 1,
        text: "answer".into(),
    }));
    let ids = collect_append_message_ids(&all);
    assert_eq!(ids.len(), 2);
    assert_ne!(
        ids[0], ids[1],
        "message_id MUST rotate on kind change (thinking→text); got: {ids:?}"
    );
}

#[test]
fn kind_change_rotates_message_id_text_then_tool_use_then_text() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let mut all = Vec::new();
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "before".into(),
    }));
    all.extend(a.translate(&OzResponseFrame::ToolUse {
        block_index: 1,
        id: "tu-1".into(),
        name: "ls".into(),
        input: serde_json::json!({"path": "/"}),
    }));
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 2,
        text: "after".into(),
    }));
    let ids = collect_append_message_ids(&all);
    assert_eq!(ids.len(), 3);
    // All three must be distinct — including the text↔text pair separated by
    // a tool_use, because the middle kind flip locked the prior text message.
    assert_ne!(ids[0], ids[1], "text→tool_use must rotate");
    assert_ne!(ids[1], ids[2], "tool_use→text must rotate");
    assert_ne!(
        ids[0], ids[2],
        "text resuming after a tool_use must NOT reuse the pre-tool message id; got {ids:?}"
    );
}

#[test]
fn consecutive_same_kind_deltas_share_message_id() {
    // Baseline: successive TextDeltas on the same turn SHOULD land on the
    // same message_id — the oneof is already locked to AgentOutput, so the
    // FieldMask patch is safe to keep appending.
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let mut all = Vec::new();
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "hel".into(),
    }));
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "lo".into(),
    }));
    let ids = collect_append_message_ids(&all);
    assert_eq!(ids.len(), 2, "one append per delta");
    assert_eq!(
        ids[0], ids[1],
        "same-kind consecutive deltas must share message_id; got {ids:?}"
    );
}

#[test]
fn each_kind_change_emits_add_messages_to_task_prelude() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let mut all = Vec::new();
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 0,
        text: "a".into(),
    }));
    all.extend(a.translate(&OzResponseFrame::ThinkingDelta {
        block_index: 1,
        text: "b".into(),
        signature: None,
    }));
    all.extend(a.translate(&OzResponseFrame::TextDelta {
        block_index: 2,
        text: "c".into(),
    }));
    let n = count_add_messages_actions(&all);
    assert_eq!(
        n, 3,
        "expected one AddMessagesToTask per distinct kind-run (text/thinking/text); got {n}"
    );
}

#[test]
fn fieldmask_append_actually_mutates_text_via_descriptor() {
    // End-to-end: construct base + patch Message exactly as our adapter emits,
    // run the same prost-reflect-based FieldMaskOperation pattern Warp uses,
    // and confirm the resulting text is non-empty. If the path ever reverts
    // to `message.X`, this test will fail.
    use prost_reflect::{DynamicMessage, ReflectMessage, Value};
    use prost_types::FieldMask;

    let desc = wmaa::MESSAGE_DESCRIPTOR.clone();

    let base = wmaa::Message {
        id: "msg-1".into(),
        task_id: "task-1".into(),
        message: Some(wmaa::message::Message::AgentOutput(
            wmaa::message::AgentOutput {
                text: String::new(),
            },
        )),
        ..Default::default()
    };
    let patch = wmaa::Message {
        id: "msg-1".into(),
        task_id: "task-1".into(),
        message: Some(wmaa::message::Message::AgentOutput(
            wmaa::message::AgentOutput {
                text: "hello".into(),
            },
        )),
        ..Default::default()
    };
    let mask = FieldMask {
        paths: vec!["agent_output.text".into()],
    };

    let mut dyn_target = DynamicMessage::new(desc.clone());
    dyn_target.transcode_from(&base).unwrap();
    let mut dyn_patch = DynamicMessage::new(desc.clone());
    dyn_patch.transcode_from(&patch).unwrap();

    for path in &mask.paths {
        let mut segs: Vec<&str> = path.split('.').collect();
        apply(&mut dyn_target, &dyn_patch, &mut segs);
    }

    let merged: wmaa::Message = dyn_target.transcode_to().unwrap();
    let text = match &merged.message {
        Some(wmaa::message::Message::AgentOutput(a)) => a.text.clone(),
        _ => panic!("agent_output variant missing after merge"),
    };
    assert_eq!(
        text, "hello",
        "FieldMask merge must yield non-empty text; if this is empty, the mask \
         path is wrong (likely reverted to `message.agent_output.text`)."
    );

    fn apply(target: &mut DynamicMessage, patch: &DynamicMessage, segs: &mut Vec<&str>) {
        let Some(first) = segs.first().copied() else {
            return;
        };
        let Some(f) = target.descriptor().get_field_by_name(first) else {
            panic!(
                "segment {first:?} not found on descriptor {} — this means the \
                 FieldMask is a no-op (the bug this test guards against)",
                target.descriptor().full_name()
            );
        };
        if segs.len() == 1 {
            let pv = patch.get_field(&f).into_owned();
            target.try_set_field(&f, pv).unwrap();
            return;
        }
        let rest: Vec<&str> = segs[1..].to_vec();
        let tv = target.get_field_mut(&f);
        let pv = patch.get_field(&f);
        match (&mut *tv, pv.as_ref()) {
            (Value::Message(t), Value::Message(p)) => {
                let mut rest_mut = rest;
                apply(t, p, &mut rest_mut);
            }
            _ => panic!("non-message at segment {first:?}"),
        }
    }
}
