use clap::Parser;

mod cli;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Run { ref bind } => {
            println!("run on {bind} — not implemented yet");
        }
        cli::Command::Cert { install } => {
            println!("cert install={install} — not implemented yet");
        }
        cli::Command::Login { ref mode } => {
            println!("login mode={mode} — not implemented yet");
        }
    }
    Ok(())
}
