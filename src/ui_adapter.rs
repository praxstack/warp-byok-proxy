//! Translates internal [`OzResponseFrame`] stream into Warp's `ResponseEvent`
//! protobuf stream.
//!
//! The adapter synthesizes session ids, task ids, and message ids so that the
//! UI consumer cannot tell whether frames came from app.warp.dev or from
//! oz-local. Semantics are deliberately minimal for Phase 0 — the first
//! emitted frame produces a synthetic `StreamInit` + `CreateTask` pair, text
//! and thinking deltas are forwarded as `AppendToMessageContent` carrying the
//! rendered payload in the `Message.message` oneof (`AgentOutput`,
//! `AgentReasoning`, `ToolCall`). `UsageUpdate` frames are accumulated into
//! adapter state and flushed onto `StreamFinished.token_usage` when `Done`
//! arrives. `Message.server_message_data` and `UpdateTaskServerData.server_data`
//! are reserved for opaque client-roundtrip payloads and are intentionally left
//! untouched by this adapter.

use crate::frame::OzResponseFrame;
use uuid::Uuid;
use warp_multi_agent_api as wmaa;

/// Runtime knobs for [`UiAdapter`].
///
/// All fields are optional strings that — when absent — become empty strings
/// in the emitted [`wmaa::response_event::StreamInit`] event. Real values are
/// injected by the request pipeline once it observes the upstream request.
#[derive(Debug, Default, Clone)]
pub struct UiAdapterOpts {
    /// Conversation id to echo back in `StreamInit`.
    pub conversation_id: Option<String>,
    /// Request id to echo back in `StreamInit`.
    pub request_id: Option<String>,
    /// Oz run id to echo back in `StreamInit`.
    pub run_id: Option<String>,
}

/// Stateful translator from [`OzResponseFrame`] to
/// [`wmaa::ResponseEvent`] batches.
///
/// The adapter is single-turn: the first frame observed synthesizes the
/// `StreamInit` and `CreateTask` prelude. All subsequent frames re-use the
/// same task and message ids so the UI can stitch appended content into a
/// single message.
pub struct UiAdapter {
    opts: UiAdapterOpts,
    sent_init: bool,
    task_id: String,
    message_id: String,
    /// Accumulated token usage. Populated by `UsageUpdate` frames
    /// (last-write-wins for Phase 0) and flushed onto `StreamFinished` when
    /// `Done` arrives.
    pending_usage: Option<wmaa::response_event::stream_finished::TokenUsage>,
}

impl UiAdapter {
    /// Constructs a new adapter with the provided options.
    #[must_use]
    pub fn new(opts: UiAdapterOpts) -> Self {
        Self {
            opts,
            sent_init: false,
            task_id: format!("task-{}", Uuid::new_v4()),
            message_id: format!("msg-{}", Uuid::new_v4()),
            pending_usage: None,
        }
    }

    /// Translates a single [`OzResponseFrame`] into zero or more
    /// [`wmaa::ResponseEvent`] values.
    ///
    /// Warp's UI rendering path (verified 2026-04-30 via audit of the open-
    /// source client code at `app/src/ai/agent/{conversation,task}.rs`)
    /// requires a specific prelude sequence before the first
    /// `AppendToMessageContent` can land:
    ///
    /// 1. `StreamInit` (non-empty `request_id`, `conversation_id`).
    /// 2. `CreateTask { task: Task{ id, messages: vec![] } }` — registers the
    ///    task so later `task_id` references resolve.
    /// 3. `AddMessagesToTask { task_id, messages: vec![Message{ id, task_id,
    ///    message: AgentOutput{ text: "" } }] }` — creates an empty message
    ///    with a stable `message.id` that later `AppendToMessageContent`
    ///    events target. `append_to_message_content` in `task.rs:754-763`
    ///    returns `MessageNotFound` if no prior message with matching `id`
    ///    exists.
    ///
    /// After the prelude, each text chunk rides an `AppendToMessageContent`
    /// with `mask: FieldMask{ paths: ["message.agent_output.text"] }` (rooted
    /// at the Message descriptor; NOT `"agent_output.text"`).
    pub fn translate(&mut self, frame: &OzResponseFrame) -> Vec<wmaa::ResponseEvent> {
        let mut events: Vec<wmaa::ResponseEvent> = Vec::new();
        let needs_prelude = matches!(
            frame,
            OzResponseFrame::TextDelta { .. }
                | OzResponseFrame::ThinkingDelta { .. }
                | OzResponseFrame::ToolUse { .. }
                | OzResponseFrame::Done { .. }
        );
        if !self.sent_init && needs_prelude {
            // 3-part prelude — each must land BEFORE the first append. See
            // doc on `translate` above and the real-world audit of Warp's
            // consumer code for why all three are required.
            events.push(self.stream_init_event());
            events.push(self.create_task_event());
            events.push(self.add_messages_to_task_event());
            self.sent_init = true;
        }
        match frame {
            OzResponseFrame::TextDelta { text, .. } => {
                events.push(self.append_text_event(text));
            }
            OzResponseFrame::ThinkingDelta { text, .. } => {
                events.push(self.append_thinking_event(text));
            }
            OzResponseFrame::ToolUse {
                id, name, input, ..
            } => {
                events.push(self.tool_use_event(id, name, input));
            }
            OzResponseFrame::ToolUseInputDelta { .. } | OzResponseFrame::BlockStop { .. } => {
                // Phase 0: partials collapse into the final ToolUse emission.
            }
            OzResponseFrame::UsageUpdate {
                input_tokens,
                output_tokens,
                cache_read,
                cache_write,
            } => {
                // Accumulate into adapter state; do NOT emit a ClientAction.
                // The usage is flushed onto `StreamFinished.token_usage` when
                // `Done` arrives. Last-write-wins across multiple frames.
                self.pending_usage = Some(wmaa::response_event::stream_finished::TokenUsage {
                    total_input: u32::try_from(*input_tokens).unwrap_or(u32::MAX),
                    output: u32::try_from(*output_tokens).unwrap_or(u32::MAX),
                    input_cache_read: u32::try_from(*cache_read).unwrap_or(u32::MAX),
                    input_cache_write: u32::try_from(*cache_write).unwrap_or(u32::MAX),
                    ..Default::default()
                });
            }
            OzResponseFrame::Done { stop_reason } => {
                events.push(self.stream_finished_event(stop_reason));
            }
        }
        events
    }

    fn stream_init_event(&self) -> wmaa::ResponseEvent {
        wmaa::ResponseEvent {
            r#type: Some(wmaa::response_event::Type::Init(
                wmaa::response_event::StreamInit {
                    conversation_id: self.opts.conversation_id.clone().unwrap_or_default(),
                    request_id: self.opts.request_id.clone().unwrap_or_default(),
                    run_id: self.opts.run_id.clone().unwrap_or_default(),
                },
            )),
        }
    }

    fn create_task_event(&self) -> wmaa::ResponseEvent {
        let task = wmaa::Task {
            id: self.task_id.clone(),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::CreateTask(
                wmaa::client_action::CreateTask { task: Some(task) },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    /// Emits `AddMessagesToTask` carrying an empty `AgentOutput` message with
    /// `id == self.message_id`. This is required BEFORE the first
    /// `AppendToMessageContent` so that Warp's `append_to_message_content`
    /// handler (in `task.rs:754-763`) can find a matching message by id.
    /// Skipping this step is what causes Warp to silently drop appends and
    /// render nothing — the bug caught live on 2026-04-30.
    fn add_messages_to_task_event(&self) -> wmaa::ResponseEvent {
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            message: Some(wmaa::message::Message::AgentOutput(
                wmaa::message::AgentOutput {
                    text: String::new(),
                },
            )),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AddMessagesToTask(
                wmaa::client_action::AddMessagesToTask {
                    task_id: self.task_id.clone(),
                    messages: vec![message],
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn append_text_event(&self, text: &str) -> wmaa::ResponseEvent {
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            message: Some(wmaa::message::Message::AgentOutput(
                wmaa::message::AgentOutput {
                    text: text.to_string(),
                },
            )),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
                    // FieldMask paths are rooted at the outer `Message` proto
                    // descriptor. Each segment must match a real field name
                    // returned by `descriptor.get_field_by_name(...)`. In proto3,
                    // `oneof message { AgentOutput agent_output = 3; ... }`
                    // exposes each variant AS A TOP-LEVEL FIELD — the oneof
                    // wrapper name `message` itself is NOT a field, so putting
                    // it in the path causes `field_mask/src/lib.rs:108` to
                    // silently no-op (verified via src/bin/test_fieldmask.rs).
                    // The correct path is rooted at the variant name.
                    mask: Some(::prost_types::FieldMask {
                        paths: vec!["agent_output.text".to_string()],
                    }),
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn append_thinking_event(&self, text: &str) -> wmaa::ResponseEvent {
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            message: Some(wmaa::message::Message::AgentReasoning(
                wmaa::message::AgentReasoning {
                    reasoning: text.to_string(),
                    finished_duration: None,
                },
            )),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
                    // See append_text_event for why we drop the `message.`
                    // prefix — the oneof wrapper is not a real field.
                    mask: Some(::prost_types::FieldMask {
                        paths: vec!["agent_reasoning.reasoning".to_string()],
                    }),
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn tool_use_event(
        &self,
        id: &str,
        name: &str,
        input: &serde_json::Value,
    ) -> wmaa::ResponseEvent {
        // Phase 0: The generated `Message.ToolCall` wraps a `tool` oneof of
        // ~33 tool-specific variants (RunShellCommand, ReadFiles, ...). We do
        // not attempt variant-specific decoding here — upstream BYOK tool
        // names are arbitrary and Warp's client decoder reads the generic
        // `Server { payload }` variant for opaque round-trippable payloads.
        // Richer variant mapping is deferred to Phase 1.
        let payload = serde_json::json!({
            "id": id,
            "name": name,
            "input": input,
        })
        .to_string();
        let tool_call = wmaa::message::ToolCall {
            tool_call_id: id.to_string(),
            tool: Some(wmaa::message::tool_call::Tool::Server(
                wmaa::message::tool_call::Server { payload },
            )),
        };
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            message: Some(wmaa::message::Message::ToolCall(tool_call)),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
                    // Descriptor-rooted; no `message.` prefix (see
                    // append_text_event comment).
                    mask: Some(::prost_types::FieldMask {
                        paths: vec!["tool_call.server.payload".to_string()],
                    }),
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn stream_finished_event(&self, reason: &str) -> wmaa::ResponseEvent {
        use wmaa::response_event::stream_finished;
        let reason_variant = match reason {
            "max_tokens" => {
                stream_finished::Reason::MaxTokenLimit(stream_finished::ReachedMaxTokenLimit {})
            }
            "quota_limit" => stream_finished::Reason::QuotaLimit(stream_finished::QuotaLimit {}),
            "context_window_exceeded" => stream_finished::Reason::ContextWindowExceeded(
                stream_finished::ContextWindowExceeded {},
            ),
            "end_turn" | "stop_sequence" | "tool_use" | "done" => {
                stream_finished::Reason::Done(stream_finished::Done {})
            }
            _ => stream_finished::Reason::Other(stream_finished::Other {}),
        };
        let token_usage = self
            .pending_usage
            .clone()
            .map(|tu| vec![tu])
            .unwrap_or_default();
        wmaa::ResponseEvent {
            r#type: Some(wmaa::response_event::Type::Finished(
                wmaa::response_event::StreamFinished {
                    reason: Some(reason_variant),
                    token_usage,
                    ..Default::default()
                },
            )),
        }
    }

    fn client_actions_event(actions: Vec<wmaa::ClientAction>) -> wmaa::ResponseEvent {
        wmaa::ResponseEvent {
            r#type: Some(wmaa::response_event::Type::ClientActions(
                wmaa::response_event::ClientActions { actions },
            )),
        }
    }
}
