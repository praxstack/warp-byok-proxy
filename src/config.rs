use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bedrock: Bedrock,
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
    /// Returns an error if `enable_1m_context` is set on a model that is not
    /// Claude Opus 4.6 or 4.7.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.bedrock.enable_1m_context && !is_opus_4_6_or_4_7(&self.bedrock.model) {
            anyhow::bail!(
                "1M context requires Opus 4.6 or 4.7; current model: {}",
                self.bedrock.model
            );
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

fn is_opus_4_6_or_4_7(model: &str) -> bool {
    // Strip ':1m' suffix and any region/global prefix (handled elsewhere).
    let m = model.rsplit_once(':').map_or(
        model,
        |(head, tail)| if tail == "1m" { head } else { model },
    );
    m.contains("claude-opus-4-6") || m.contains("claude-opus-4-7")
}
