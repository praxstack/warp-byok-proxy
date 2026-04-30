//! Anthropic `anthropic-beta` header builder.

/// Beta flag enabling the 1M context window for Claude Opus models.
pub const CONTEXT_1M_BETA: &str = "context-1m-2025-08-07";

/// Build the list of `anthropic-beta` flags, injecting the 1M-context beta
/// when requested while preserving caller-supplied flags and de-duplicating.
#[must_use]
pub fn build_betas(opus_1m: bool, existing: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = existing.iter().map(ToString::to_string).collect();
    if opus_1m && !out.iter().any(|b| b == CONTEXT_1M_BETA) {
        out.push(CONTEXT_1M_BETA.to_string());
    }
    out
}
