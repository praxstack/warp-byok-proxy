use warp_byok_proxy::auth::{resolve_auth, AuthError, AuthInputs, AuthMode, ResolvedAuth};

fn base(mode: AuthMode) -> AuthInputs {
    AuthInputs {
        mode,
        api_key: None,
        profile: None,
        access_key: None,
        secret_key: None,
        session_token: None,
        region: Some("us-east-1".into()),
        skip_auth: false,
    }
}

#[test]
fn api_key_mode_happy() {
    let inp = AuthInputs {
        api_key: Some("sk-test".into()),
        ..base(AuthMode::ApiKey)
    };
    match resolve_auth(&inp).unwrap() {
        ResolvedAuth::BearerToken(t) => assert_eq!(t, "sk-test"),
        r => panic!("wrong variant: {r:?}"),
    }
}

#[test]
fn api_key_mode_empty_is_err() {
    let inp = AuthInputs {
        api_key: Some("   ".into()),
        ..base(AuthMode::ApiKey)
    };
    matches_err(resolve_auth(&inp), "empty");
}

#[test]
fn api_key_mode_literal_bearer_prefix_is_err() {
    let inp = AuthInputs {
        api_key: Some("Bearer ".into()),
        ..base(AuthMode::ApiKey)
    };
    matches_err(resolve_auth(&inp), "Bearer");
}

#[test]
fn profile_mode_uses_named_profile() {
    let inp = AuthInputs {
        profile: Some("dev-sso".into()),
        ..base(AuthMode::Profile)
    };
    match resolve_auth(&inp).unwrap() {
        ResolvedAuth::Profile(name) => assert_eq!(name, "dev-sso"),
        r => panic!("wrong variant: {r:?}"),
    }
}

#[test]
fn profile_mode_missing_profile_is_err() {
    let inp = base(AuthMode::Profile);
    matches_err(resolve_auth(&inp), "profile");
}

#[test]
fn credentials_mode_ak_sk_ok() {
    let inp = AuthInputs {
        access_key: Some("AKIA".into()),
        secret_key: Some("SECRET".into()),
        ..base(AuthMode::Credentials)
    };
    match resolve_auth(&inp).unwrap() {
        ResolvedAuth::ExplicitKeys {
            access_key,
            secret_key,
            session_token,
        } => {
            assert_eq!(access_key, "AKIA");
            assert_eq!(secret_key, "SECRET");
            assert!(session_token.is_none());
        }
        r => panic!("wrong variant: {r:?}"),
    }
}

#[test]
fn credentials_mode_with_session_token() {
    let inp = AuthInputs {
        access_key: Some("AKIA".into()),
        secret_key: Some("SECRET".into()),
        session_token: Some("TOKEN".into()),
        ..base(AuthMode::Credentials)
    };
    match resolve_auth(&inp).unwrap() {
        ResolvedAuth::ExplicitKeys { session_token, .. } => {
            assert_eq!(session_token.as_deref(), Some("TOKEN"));
        }
        r => panic!("wrong variant: {r:?}"),
    }
}

#[test]
fn credentials_mode_missing_ak_is_err() {
    let inp = AuthInputs {
        secret_key: Some("SECRET".into()),
        ..base(AuthMode::Credentials)
    };
    matches_err(resolve_auth(&inp), "access_key");
}

#[test]
fn credentials_mode_missing_sk_is_err() {
    let inp = AuthInputs {
        access_key: Some("AKIA".into()),
        ..base(AuthMode::Credentials)
    };
    matches_err(resolve_auth(&inp), "secret_key");
}

#[test]
fn default_chain_mode_ok_no_inputs() {
    let inp = base(AuthMode::DefaultChain);
    assert!(matches!(
        resolve_auth(&inp).unwrap(),
        ResolvedAuth::DefaultChain
    ));
}

#[test]
fn skip_auth_env_overrides_api_key() {
    let inp = AuthInputs {
        api_key: Some("sk-test".into()),
        skip_auth: true,
        ..base(AuthMode::ApiKey)
    };
    assert!(matches!(resolve_auth(&inp).unwrap(), ResolvedAuth::Skipped));
}

#[test]
fn skip_auth_env_overrides_profile() {
    let inp = AuthInputs {
        profile: Some("dev".into()),
        skip_auth: true,
        ..base(AuthMode::Profile)
    };
    assert!(matches!(resolve_auth(&inp).unwrap(), ResolvedAuth::Skipped));
}

fn matches_err<T: std::fmt::Debug>(r: Result<T, AuthError>, needle: &str) {
    let e = r.unwrap_err();
    assert!(
        format!("{e}")
            .to_lowercase()
            .contains(&needle.to_lowercase()),
        "expected error containing `{needle}`, got: {e}"
    );
}
