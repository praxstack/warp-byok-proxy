use anyhow::Context;
use clap::Parser;

mod cli;
mod logging;

fn main() -> anyhow::Result<()> {
    logging::init();
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Run { ref bind } => {
            tracing::info!(%bind, "warp-byok-proxy starting");
            if std::env::var("WARP_BYOK_PROXY_SELFTEST_EXIT").is_ok() {
                return Ok(());
            }
            println!("run on {bind} — not implemented yet");
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
