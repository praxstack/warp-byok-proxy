use anyhow::{Context, Result};
use clap::Parser;
use std::sync::Arc;

mod cli;
mod logging;

fn main() -> Result<()> {
    logging::init();
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Run { ref bind } => {
            tracing::info!(%bind, "warp-byok-proxy starting");
            if std::env::var("WARP_BYOK_PROXY_SELFTEST_EXIT").is_ok() {
                return Ok(());
            }
            run_server(bind)?;
        }
        cli::Command::Cert { install } => {
            let out = dirs::config_dir()
                .context("no config_dir")?
                .join("warp-byok-proxy");
            let paths =
                warp_byok_proxy::cert::generate_self_signed(&out, &["127.0.0.1", "app.warp.dev"])?;
            tracing::info!(cert = %paths.cert_pem.display(), "generated self-signed cert");
            println!("cert: {}", paths.cert_pem.display());
            println!("key:  {}", paths.key_pem.display());
            if install {
                warp_byok_proxy::cert::install_to_keychain(&paths.cert_pem)?;
                tracing::info!("cert installed to System.keychain");
            }
        }
        cli::Command::Login { ref mode } => {
            tracing::info!(%mode, "warp-byok-proxy login");
            println!("login mode={mode} — not implemented yet");
        }
    }
    Ok(())
}

/// Load config, build the real-Bedrock provider, and serve until Ctrl-C.
fn run_server(bind: &str) -> Result<()> {
    use warp_byok_proxy::{
        auth::{self, AuthInputs},
        bedrock_client::{self, BedrockLike, RealBedrock},
        config::Config,
        server,
    };

    // 1. Load config.
    let cfg_path = dirs::config_dir()
        .context("no config_dir")?
        .join("warp-byok-proxy/config.toml");
    tracing::info!(cfg = %cfg_path.display(), "loading config");
    let cfg_text = std::fs::read_to_string(&cfg_path)
        .with_context(|| format!("read config at {}", cfg_path.display()))?;
    let cfg: Config = toml::from_str(&cfg_text).context("parse config.toml")?;
    cfg.validate().context("config.validate()")?;
    for w in cfg.validate_with_warnings().unwrap_or_default() {
        tracing::warn!(%w, "config warning");
    }

    // 2. Resolve auth. For api-key mode the SDK reads AWS_BEARER_TOKEN_BEDROCK
    //    from the env directly, but we still validate that it's present.
    let api_key = std::env::var("AWS_BEARER_TOKEN_BEDROCK").ok();
    let inputs = AuthInputs {
        mode: cfg.bedrock.auth_mode.clone().into(),
        api_key,
        profile: cfg.bedrock.profile.clone(),
        region: Some(cfg.bedrock.region.clone()),
        ..Default::default()
    };
    let resolved = auth::resolve_auth(&inputs).context("resolve auth")?;
    tracing::info!(auth_kind = ?std::mem::discriminant(&resolved), "auth resolved");

    // 3. Locate the cert/key minted by `cert --install`.
    let cert_dir = dirs::config_dir()
        .context("no config_dir")?
        .join("warp-byok-proxy");
    let cert_pem = cert_dir.join("cert.pem");
    let key_pem = cert_dir.join("key.pem");
    anyhow::ensure!(
        cert_pem.exists() && key_pem.exists(),
        "cert not found at {}. Run `warp-byok-proxy cert --install` first.",
        cert_pem.display()
    );

    // 4. Build the AWS SDK client + wrap it in RealBedrock.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    let bedrock: Arc<dyn BedrockLike> = rt.block_on(async {
        let client = bedrock_client::build_client(
            &resolved,
            &cfg.bedrock.region,
            cfg.bedrock.endpoint.as_deref(),
        )
        .await?;
        // Resolve tool configuration once at startup — typos in
        // cfg.bedrock.tools[].input_schema_json already failed config.validate()
        // above, so a late parse failure here is unexpected but still propagated.
        let tool_config = warp_byok_proxy::sdk_translator::tools_to_sdk(&cfg.bedrock.tools)
            .context("translate cfg.bedrock.tools to ToolConfiguration")?;
        if let Some(tc) = tool_config.as_ref() {
            tracing::info!(
                n_tools = tc.tools().len(),
                "tool config loaded from cfg.bedrock.tools"
            );
        }
        Ok::<_, anyhow::Error>(Arc::new(RealBedrock {
            client,
            tool_config,
        }) as Arc<dyn BedrockLike>)
    })?;

    // 5. Spawn the server and wait for Ctrl-C.
    rt.block_on(async {
        let (addr, shutdown) =
            server::spawn(bind, &cert_pem, &key_pem, Arc::new(cfg), bedrock).await?;
        tracing::info!(%addr, "server ready — Ctrl-C to quit");
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("shutdown signal received");
        let _ = shutdown.send(());
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}
