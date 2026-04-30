use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    ApiKey,
    Profile,
    Credentials,
    #[default]
    DefaultChain,
}

impl From<crate::config::AuthMode> for AuthMode {
    fn from(m: crate::config::AuthMode) -> Self {
        match m {
            crate::config::AuthMode::ApiKey => AuthMode::ApiKey,
            crate::config::AuthMode::Profile => AuthMode::Profile,
            crate::config::AuthMode::Credentials => AuthMode::Credentials,
            crate::config::AuthMode::DefaultChain => AuthMode::DefaultChain,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AuthInputs {
    pub mode: AuthMode,
    pub api_key: Option<String>,
    pub profile: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub session_token: Option<String>,
    pub region: Option<String>,
    /// `AWS_BEDROCK_SKIP_AUTH=1` equivalent.
    pub skip_auth: bool,
}

#[derive(Debug, Clone)]
pub enum ResolvedAuth {
    BearerToken(String),
    Profile(String),
    ExplicitKeys {
        access_key: String,
        secret_key: String,
        session_token: Option<String>,
    },
    DefaultChain,
    Skipped,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("api key is empty or whitespace-only")]
    ApiKeyEmpty,
    #[error("api key is a literal 'Bearer' prefix with no token")]
    ApiKeyBearerOnly,
    #[error("profile mode requires a profile name")]
    ProfileMissing,
    #[error("credentials mode requires access_key")]
    AccessKeyMissing,
    #[error("credentials mode requires secret_key")]
    SecretKeyMissing,
}

/// Resolve the auth inputs into a concrete `ResolvedAuth` variant.
///
/// # Errors
/// Returns [`AuthError`] when the selected mode's required fields are missing,
/// blank, or malformed (e.g. an api key that is only whitespace or a literal
/// `Bearer` prefix).
pub fn resolve_auth(inp: &AuthInputs) -> Result<ResolvedAuth, AuthError> {
    if inp.skip_auth {
        return Ok(ResolvedAuth::Skipped);
    }
    match inp.mode {
        AuthMode::ApiKey => {
            let key = inp.api_key.as_deref().unwrap_or("");
            let trimmed = key.trim();
            if trimmed.is_empty() {
                return Err(AuthError::ApiKeyEmpty);
            }
            if trimmed.eq_ignore_ascii_case("bearer")
                || trimmed.starts_with("Bearer ") && trimmed[7..].trim().is_empty()
            {
                return Err(AuthError::ApiKeyBearerOnly);
            }
            Ok(ResolvedAuth::BearerToken(trimmed.to_string()))
        }
        AuthMode::Profile => match inp.profile.as_deref() {
            Some(p) if !p.trim().is_empty() => Ok(ResolvedAuth::Profile(p.to_string())),
            _ => Err(AuthError::ProfileMissing),
        },
        AuthMode::Credentials => {
            let ak = inp
                .access_key
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or(AuthError::AccessKeyMissing)?;
            let sk = inp
                .secret_key
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or(AuthError::SecretKeyMissing)?;
            Ok(ResolvedAuth::ExplicitKeys {
                access_key: ak.to_string(),
                secret_key: sk.to_string(),
                session_token: inp.session_token.clone(),
            })
        }
        AuthMode::DefaultChain => Ok(ResolvedAuth::DefaultChain),
    }
}
