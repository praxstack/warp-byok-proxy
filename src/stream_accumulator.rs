use crate::frame::OzResponseFrame;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum BedrockEvent {
    /// `kind` is a JSON string describing the block. For text: `"text"`.
    /// For `tool_use`: `{"type":"tool_use","id":"..","name":".."}`.
    /// For thinking: `{"type":"thinking"}`.
    ContentBlockStart {
        block_index: u32,
        kind: String,
    },
    ContentBlockDelta {
        block_index: u32,
        delta_json: String,
    },
    ContentBlockStop {
        block_index: u32,
    },
    MessageStart,
    MessageStop {
        stop_reason: String,
    },
    MessageStreamMetadata {
        input_tokens: u64,
        output_tokens: u64,
        cache_read: u64,
        cache_write: u64,
    },
}

#[derive(Debug)]
struct BlockState {
    kind: BlockKind,
    tool_use: Option<ToolUseAcc>,
}

#[derive(Debug)]
enum BlockKind {
    Text,
    Thinking,
    ToolUse,
}

#[derive(Debug)]
struct ToolUseAcc {
    id: String,
    name: String,
    partial_json: String,
}

pub struct StreamAccumulator {
    blocks: HashMap<u32, BlockState>,
}

impl Default for StreamAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamAccumulator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    pub fn handle(&mut self, ev: BedrockEvent) -> Vec<OzResponseFrame> {
        match ev {
            BedrockEvent::MessageStart => Vec::new(),
            BedrockEvent::ContentBlockStart { block_index, kind } => {
                let state = parse_block_start(&kind);
                self.blocks.insert(block_index, state);
                Vec::new()
            }
            BedrockEvent::ContentBlockDelta {
                block_index,
                delta_json,
            } => {
                let Some(state) = self.blocks.get_mut(&block_index) else {
                    tracing::warn!(%block_index, "delta for unknown block");
                    return Vec::new();
                };
                handle_delta(state, block_index, &delta_json)
            }
            BedrockEvent::ContentBlockStop { block_index } => {
                let mut out = Vec::new();
                if let Some(state) = self.blocks.remove(&block_index) {
                    if let BlockKind::ToolUse = state.kind {
                        if let Some(tu) = state.tool_use {
                            let input: Value = serde_json::from_str(&tu.partial_json)
                                .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
                            out.push(OzResponseFrame::ToolUse {
                                block_index,
                                id: tu.id,
                                name: tu.name,
                                input,
                            });
                        }
                    }
                }
                out.push(OzResponseFrame::BlockStop { block_index });
                out
            }
            BedrockEvent::MessageStop { stop_reason } => {
                vec![OzResponseFrame::Done { stop_reason }]
            }
            BedrockEvent::MessageStreamMetadata {
                input_tokens,
                output_tokens,
                cache_read,
                cache_write,
            } => vec![OzResponseFrame::UsageUpdate {
                input_tokens,
                output_tokens,
                cache_read,
                cache_write,
            }],
        }
    }
}

fn parse_block_start(kind: &str) -> BlockState {
    if kind == "text" {
        return BlockState {
            kind: BlockKind::Text,
            tool_use: None,
        };
    }
    let parsed: Value = serde_json::from_str(kind).unwrap_or(Value::Null);
    let ty = parsed.get("type").and_then(Value::as_str).unwrap_or("text");
    match ty {
        "thinking" => BlockState {
            kind: BlockKind::Thinking,
            tool_use: None,
        },
        "tool_use" => {
            let id = parsed
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = parsed
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            BlockState {
                kind: BlockKind::ToolUse,
                tool_use: Some(ToolUseAcc {
                    id,
                    name,
                    partial_json: String::new(),
                }),
            }
        }
        _ => BlockState {
            kind: BlockKind::Text,
            tool_use: None,
        },
    }
}

fn handle_delta(
    state: &mut BlockState,
    block_index: u32,
    delta_json: &str,
) -> Vec<OzResponseFrame> {
    let parsed: Value = match serde_json::from_str(delta_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let ty = parsed.get("type").and_then(Value::as_str).unwrap_or("");
    match (&state.kind, ty) {
        (BlockKind::Text, "text_delta") => {
            let text = parsed
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            vec![OzResponseFrame::TextDelta { block_index, text }]
        }
        (BlockKind::Thinking, "thinking_delta") => {
            let text = parsed
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            vec![OzResponseFrame::ThinkingDelta {
                block_index,
                text,
                signature: None,
            }]
        }
        (BlockKind::Thinking, "signature_delta") => {
            let sig = parsed
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_string);
            vec![OzResponseFrame::ThinkingDelta {
                block_index,
                text: String::new(),
                signature: sig,
            }]
        }
        (BlockKind::ToolUse, "input_json_delta") => {
            let piece = parsed
                .get("partial_json")
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(tu) = &mut state.tool_use {
                tu.partial_json.push_str(piece);
            }
            let id = state
                .tool_use
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_default();
            vec![OzResponseFrame::ToolUseInputDelta {
                block_index,
                id,
                partial_json: piece.to_string(),
            }]
        }
        _ => Vec::new(),
    }
}
