use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OzResponseFrame {
    TextDelta {
        block_index: u32,
        text: String,
    },
    ThinkingDelta {
        block_index: u32,
        text: String,
        signature: Option<String>,
    },
    ToolUse {
        block_index: u32,
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolUseInputDelta {
        block_index: u32,
        id: String,
        partial_json: String,
    },
    BlockStop {
        block_index: u32,
    },
    UsageUpdate {
        input_tokens: u64,
        output_tokens: u64,
        cache_read: u64,
        cache_write: u64,
    },
    Done {
        stop_reason: String,
    },
}
