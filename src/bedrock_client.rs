//! Thin Bedrock runtime client factory.
//!
//! Maps a [`crate::auth::ResolvedAuth`] onto an `aws_config` loader, optionally
//! pinning an endpoint URL, and hands back an `aws_sdk_bedrockruntime::Client`.
//! Bearer-token auth is not supplied through the SDK credentials chain — it
//! rides as an HTTP header set by the caller (see [`bearer_header`]).

use crate::auth::ResolvedAuth;
use anyhow::Result;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::Client as BedrockClient;

/// Build a Bedrock runtime client honoring the resolved auth mode, region,
/// and optional endpoint override.
///
/// # Errors
/// Currently infallible because the underlying SDK calls do not return errors
/// here, but returns `Result` to reserve space for future endpoint/region
/// validation without a breaking change.
pub async fn build_client(
    auth: &ResolvedAuth,
    region: &str,
    endpoint: Option<&str>,
) -> Result<BedrockClient> {
    let mut loader =
        aws_config::defaults(BehaviorVersion::latest()).region(Region::new(region.to_string()));
    match auth {
        ResolvedAuth::Profile(p) => {
            loader = loader.profile_name(p);
        }
        ResolvedAuth::ExplicitKeys {
            access_key,
            secret_key,
            session_token,
        } => {
            loader = loader.credentials_provider(aws_credential_types::Credentials::new(
                access_key.clone(),
                secret_key.clone(),
                session_token.clone(),
                None,
                "warp-byok-proxy",
            ));
        }
        ResolvedAuth::BearerToken(_) | ResolvedAuth::DefaultChain | ResolvedAuth::Skipped => {
            // bearer → handled as an HTTP header by the caller (see `bearer_header`);
            // default chain → SDK picks up env/IMDS/etc.;
            // skipped → no auth injected.
        }
    }
    if let Some(ep) = endpoint {
        loader = loader.endpoint_url(ep);
    }
    let sdk_config = loader.load().await;
    Ok(BedrockClient::new(&sdk_config))
}

/// Return a `Bearer <token>` string when the resolved auth is a bearer token,
/// otherwise `None`.
#[must_use]
pub fn bearer_header(auth: &ResolvedAuth) -> Option<String> {
    match auth {
        ResolvedAuth::BearerToken(t) => Some(format!("Bearer {t}")),
        ResolvedAuth::Profile(_)
        | ResolvedAuth::ExplicitKeys { .. }
        | ResolvedAuth::DefaultChain
        | ResolvedAuth::Skipped => None,
    }
}
