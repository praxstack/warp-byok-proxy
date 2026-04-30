use warp_byok_proxy::frame::OzResponseFrame;
use warp_byok_proxy::ui_adapter::{UiAdapter, UiAdapterOpts};

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

#[test]
fn usage_update_emits_update_task_server_data() {
    let mut a = UiAdapter::new(UiAdapterOpts::default());
    let events = a.translate(&OzResponseFrame::UsageUpdate {
        input_tokens: 1,
        output_tokens: 2,
        cache_read: 3,
        cache_write: 4,
    });
    let dbg = format!("{events:?}");
    assert!(
        dbg.contains("UpdateTaskServerData")
            || dbg.to_lowercase().contains("update_task_server_data"),
        "expected UpdateTaskServerData in {dbg}"
    );
}
