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
            tracing::info!(%install, "warp-byok-proxy cert");
            println!("cert install={install} — not implemented yet");
        }
        cli::Command::Login { ref mode } => {
            tracing::info!(%mode, "warp-byok-proxy login");
            println!("login mode={mode} — not implemented yet");
        }
    }
    Ok(())
}
