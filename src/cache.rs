//! Cache-point injection for Bedrock prompt caching.
//!
//! Appends a `cachePoint` marker to the system array and to the last two
//! user messages when caching is enabled. No-op when disabled.

use serde_json::{json, Value};

/// Inputs for [`apply_cache_points`].
pub struct CacheInputs {
    /// When `false`, inputs pass through unchanged.
    pub enabled: bool,
    /// Messages array (Bedrock Converse shape).
    pub messages: Vec<Value>,
    /// Optional system array.
    pub system: Option<Value>,
}

/// Result of [`apply_cache_points`].
pub struct CacheResult {
    /// Possibly-annotated messages.
    pub messages: Vec<Value>,
    /// Possibly-annotated system.
    pub system: Option<Value>,
}

/// Inject `cachePoint` markers onto the system array and the last two user messages.
///
/// Returns inputs unchanged when `enabled` is `false`.
#[must_use]
pub fn apply_cache_points(inp: CacheInputs) -> CacheResult {
    if !inp.enabled {
        return CacheResult {
            messages: inp.messages,
            system: inp.system,
        };
    }
    let system = inp.system.map(|mut s| {
        if let Some(arr) = s.as_array_mut() {
            arr.push(json!({"cachePoint": {"type": "default"}}));
        }
        s
    });
    let user_idxs: Vec<usize> = inp
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m["role"] == "user")
        .map(|(i, _)| i)
        .collect();
    let last_two: std::collections::HashSet<usize> =
        user_idxs.iter().rev().take(2).copied().collect();
    let messages = inp
        .messages
        .into_iter()
        .enumerate()
        .map(|(i, mut m)| {
            if last_two.contains(&i) {
                if let Some(arr) = m["content"].as_array_mut() {
                    arr.push(json!({"cachePoint": {"type": "default"}}));
                }
            }
            m
        })
        .collect();
    CacheResult { messages, system }
}
