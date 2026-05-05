use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bedrock: Bedrock,
    #[serde(default)]
    pub proxy: Proxy,
}

/// Proxy-level knobs distinct from the upstream Bedrock wiring.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    /// When `true`, the proxy answers a small set of non-`/ai/multi-agent`
    /// Warp endpoints (`/graphql`, `/auth/*`) with inert 200 stubs so the
    /// Warp client's startup probes don't hard-fail and hide the real chat
    /// plumbing. Off by default to preserve the zero-egress semantics the
    /// proxy ships with — enable only if you have confirmed (via
    /// `scripts/verify_zero_egress.sh`) that your `/etc/hosts` redirect is
    /// in place for every Warp-contacted host and you can live with a 200
    /// "ok" shape that does NOT include login/session state.
    #[serde(default)]
    pub stub_warp_api: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(clippy::struct_excessive_bools)] // config toggles are independent knobs, not a flag set
pub struct Bedrock {
    pub auth_mode: AuthMode,
    pub region: String,
    pub model: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>, // VPC endpoint
    #[serde(default = "yes")]
    pub use_cross_region_inference: bool,
    #[serde(default = "no")]
    pub use_global_inference: bool,
    #[serde(default = "yes")]
    pub use_prompt_cache: bool,
    #[serde(default = "yes")]
    pub enable_1m_context: bool,
    #[serde(default)]
    pub thinking: Thinking,
    /// Optional list of tool definitions to expose to Claude via Bedrock's
    /// `ToolConfiguration`. Defaults to empty (no tools). Each entry carries
    /// a JSON Schema describing the tool's input shape — see [`ToolDef`].
    #[serde(default)]
    pub tools: Vec<ToolDef>,
}

/// One tool definition for Bedrock's `ToolConfiguration.tools[]`.
///
/// `input_schema_json` holds the JSON Schema as a raw TOML string so users
/// can paste a schema without fighting TOML→JSON translation. The string is
/// parsed + validated by [`ToolDef::parse_input_schema`] at startup so
/// typos surface before the first request lands.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    /// Tool name that Claude will emit in `tool_use.name` when it decides to
    /// call this tool. Warp maps arbitrary names into the generic
    /// `Server { payload }` variant — see `ui_adapter::tool_use_event`.
    pub name: String,
    /// Human-readable description. Claude uses this to decide WHEN to call
    /// the tool; it's the most important knob for tool-selection quality.
    pub description: String,
    /// JSON Schema for the tool's input object, as a raw string. Parsed +
    /// validated by [`ToolDef::parse_input_schema`] at startup.
    pub input_schema_json: String,
}

impl ToolDef {
    /// Parse `input_schema_json` as a JSON value.
    ///
    /// # Errors
    /// Returns an error if the string is not valid JSON.
    pub fn parse_input_schema(&self) -> anyhow::Result<serde_json::Value> {
        serde_json::from_str(&self.input_schema_json)
            .map_err(|e| anyhow::anyhow!("tool `{}`: invalid input_schema_json: {e}", self.name))
    }
}

fn yes() -> bool {
    true
}
fn no() -> bool {
    false
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuthMode {
    ApiKey,
    Profile,
    Credentials,
    DefaultChain,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Thinking {
    #[serde(default = "default_thinking_mode")]
    pub mode: ThinkingMode,
    #[serde(default = "default_effort")]
    pub effort: Effort,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

fn default_thinking_mode() -> ThinkingMode {
    ThinkingMode::Adaptive
}
fn default_effort() -> Effort {
    Effort::Max
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ThinkingMode {
    #[default]
    Adaptive,
    Enabled,
    Off,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Effort {
    Low,
    Medium,
    High,
    #[default]
    Max,
}

impl Config {
    /// Validate the parsed config.
    ///
    /// # Errors
    /// Returns an error if `enable_1m_context` is set on a model that does
    /// not support the 1M-context beta. Currently: Claude Opus 4.6, Opus 4.7,
    /// and Sonnet 4.7 per Anthropic's published support matrix (2026-Q1).
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.bedrock.enable_1m_context && !supports_1m_context(&self.bedrock.model) {
            anyhow::bail!(
                "1M context requires Opus 4.6, Opus 4.7, or Sonnet 4.7; current model: {}",
                self.bedrock.model
            );
        }
        // Eagerly parse every tool's JSON Schema so typos fail at startup
        // rather than on the first request dispatch.
        for t in &self.bedrock.tools {
            t.parse_input_schema()?;
        }
        Ok(())
    }

    /// Validate and also return non-fatal warnings.
    ///
    /// # Errors
    /// Returns the same hard errors as [`Config::validate`]. Non-fatal
    /// inconsistencies are returned as a `Vec<String>` of warnings.
    pub fn validate_with_warnings(&self) -> anyhow::Result<Vec<String>> {
        self.validate()?;
        let mut ws = Vec::new();
        if matches!(self.bedrock.thinking.mode, ThinkingMode::Off) {
            if !matches!(self.bedrock.thinking.effort, Effort::Max) {
                ws.push(format!(
                    "thinking.mode=\"off\" but effort=\"{:?}\" is set; effort will be ignored",
                    self.bedrock.thinking.effort
                ));
            }
            if self.bedrock.thinking.budget_tokens.is_some() {
                ws.push("thinking.mode=\"off\" but budget_tokens is set; ignored".to_string());
            }
        }
        Ok(ws)
    }
}

/// Returns `true` if the model id belongs to a Claude family that supports
/// the 1M-context beta (`anthropic_beta: ["context-1m-2025-08-07"]`).
///
/// Per Anthropic's support matrix as of 2026-Q1:
///   * Opus 4.6 / 4.7 — supported
///   * Sonnet 4.7 — supported (added 2025-12)
///   * Sonnet 4.5 / Haiku / earlier Opus — NOT supported
///
/// Both the `:1m` suffix (our own marker) and the CRI/global prefix
/// (`us.`, `eu.`, `apac.`, `jp.`, `au.`, `global.`) are stripped before
/// matching so validation works uniformly on raw configs and wire-shaped
/// ids.
fn supports_1m_context(model: &str) -> bool {
    // Strip trailing `:1m` marker.
    let core = match model.rsplit_once(':') {
        Some((head, "1m")) => head,
        _ => model,
    };
    // Strip leading inference-profile prefix if present.
    let stripped = ["us.", "eu.", "apac.", "jp.", "au.", "global."]
        .iter()
        .find_map(|p| core.strip_prefix(*p))
        .unwrap_or(core);
    stripped.contains("claude-opus-4-6")
        || stripped.contains("claude-opus-4-7")
        || stripped.contains("claude-sonnet-4-7")
}
