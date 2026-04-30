//! Translates internal [`OzResponseFrame`] stream into Warp's `ResponseEvent`
//! protobuf stream.
//!
//! The adapter synthesizes session ids, task ids, and message ids so that the
//! UI consumer cannot tell whether frames came from app.warp.dev or from
//! oz-local. Semantics are deliberately minimal for Phase 0 — the first
//! emitted frame produces a synthetic `StreamInit` + `CreateTask` pair, text
//! and thinking deltas are forwarded as `AppendToMessageContent`, usage
//! updates are serialised into `UpdateTaskServerData`, and `Done` is mapped
//! to `StreamFinished` with a best-effort reason mapping.

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
        }
    }

    /// Translates a single [`OzResponseFrame`] into zero or more
    /// [`wmaa::ResponseEvent`] values.
    pub fn translate(&mut self, frame: &OzResponseFrame) -> Vec<wmaa::ResponseEvent> {
        let mut events: Vec<wmaa::ResponseEvent> = Vec::new();
        if !self.sent_init {
            events.push(self.stream_init_event());
            events.push(self.create_task_event());
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
                events.push(self.usage_event(
                    *input_tokens,
                    *output_tokens,
                    *cache_read,
                    *cache_write,
                ));
            }
            OzResponseFrame::Done { stop_reason } => {
                events.push(Self::stream_finished_event(stop_reason));
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
        wmaa::ResponseEvent {
            r#type: Some(wmaa::response_event::Type::ClientActions(
                wmaa::response_event::ClientActions {
                    actions: vec![action],
                },
            )),
        }
    }

    fn append_text_event(&self, text: &str) -> wmaa::ResponseEvent {
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            server_message_data: text.to_string(),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
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
            server_message_data: text.to_string(),
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
                    mask: Some(::prost_types::FieldMask {
                        paths: vec!["agent_output.thinking".to_string()],
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
        // Phase 0 scaffolding: encode tool_use as an AppendToMessageContent
        // carrying a JSON blob of {id, name, input}. Richer mapping to the
        // real ToolUse message variant is deferred to Phase 1.
        let blob = serde_json::json!({
            "id": id,
            "name": name,
            "input": input,
        })
        .to_string();
        let message = wmaa::Message {
            id: self.message_id.clone(),
            task_id: self.task_id.clone(),
            server_message_data: blob,
            ..Default::default()
        };
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::AppendToMessageContent(
                wmaa::client_action::AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(message),
                    mask: Some(::prost_types::FieldMask {
                        paths: vec!["agent_output.tool_use".to_string()],
                    }),
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn usage_event(&self, i: u64, o: u64, cr: u64, cw: u64) -> wmaa::ResponseEvent {
        let payload = serde_json::json!({
            "input_tokens": i,
            "output_tokens": o,
            "cache_read": cr,
            "cache_write": cw,
        })
        .to_string();
        let action = wmaa::ClientAction {
            action: Some(wmaa::client_action::Action::UpdateTaskServerData(
                wmaa::client_action::UpdateTaskServerData {
                    task_id: self.task_id.clone(),
                    server_data: payload,
                },
            )),
        };
        Self::client_actions_event(vec![action])
    }

    fn stream_finished_event(reason: &str) -> wmaa::ResponseEvent {
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
        wmaa::ResponseEvent {
            r#type: Some(wmaa::response_event::Type::Finished(
                wmaa::response_event::StreamFinished {
                    reason: Some(reason_variant),
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
