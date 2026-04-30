use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "warp-byok-proxy",
    version,
    about = "Warp → Bedrock proxy (Phase 0)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the proxy daemon on 127.0.0.1:443.
    Run {
        #[arg(long, default_value = "127.0.0.1:443")]
        bind: String,
    },
    /// Generate a self-signed cert and trust it in macOS Keychain.
    Cert {
        #[arg(long)]
        install: bool,
    },
    /// Store a Bedrock API key in macOS Keychain.
    Login {
        #[arg(long, default_value = "api-key")]
        mode: String,
    },
}
