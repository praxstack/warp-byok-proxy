use anyhow::Result;

#[derive(Debug, Clone)]
pub struct PrepareOpts<'a> {
    pub use_cross_region_inference: bool,
    pub use_global_inference: bool,
    pub region_hint: &'a str,
}

#[derive(Debug, Clone)]
pub struct PreparedModel {
    /// Model ID without `:1m` suffix, prefix-stripped.
    pub canonical: String,
    /// Model ID as it should appear on the Bedrock wire (with CRI/global prefix if applicable).
    pub wire_model_id: String,
    /// True if user requested 1M context via `:1m` suffix.
    pub opus_1m: bool,
}

const KNOWN_PREFIXES: &[&str] = &["us.", "eu.", "apac.", "jp.", "au.", "global."];

/// Prepare a model ID for the Bedrock wire:
/// - Strips a trailing `:1m` suffix and records it in `opus_1m`.
/// - Adds a CRI (`us.` / `eu.` / `apac.`) or `global.` prefix when requested,
///   unless a known prefix is already present.
///
/// # Errors
///
/// Currently infallible in practice but returns `Result` to reserve space for
/// future validation (e.g. rejecting empty or malformed model IDs).
pub fn prepare_model_id(raw: &str, opts: &PrepareOpts<'_>) -> Result<PreparedModel> {
    // Strip :1m suffix
    let (stripped, opus_1m) = match raw.rsplit_once(':') {
        Some((head, "1m")) => (head.to_string(), true),
        _ => (raw.to_string(), false),
    };
    // Detect existing prefix
    let already_prefixed = KNOWN_PREFIXES.iter().any(|p| stripped.starts_with(p));
    let wire = if already_prefixed {
        stripped.clone()
    } else if opts.use_global_inference {
        format!("global.{stripped}")
    } else if opts.use_cross_region_inference {
        let prefix = region_prefix(opts.region_hint);
        format!("{prefix}.{stripped}")
    } else {
        stripped.clone()
    };
    Ok(PreparedModel {
        canonical: stripped,
        wire_model_id: wire,
        opus_1m,
    })
}

fn region_prefix(region: &str) -> &'static str {
    if region.starts_with("us-") || region.starts_with("ca-") {
        "us"
    } else if region.starts_with("eu-") {
        "eu"
    } else if region.starts_with("ap-") {
        "apac"
    } else {
        "us" // safe fallback
    }
}
