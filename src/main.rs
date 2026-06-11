mod bot;
mod config;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "tiktok-streak-saver",
    about = "Automates daily TikTok messages to keep your streaks alive",
    version
)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the bot with the TUI dashboard (default)
    Run,
    /// Interactive configuration wizard
    Setup,
    /// Log in to TikTok via a browser window and save cookies
    Auth,
    /// Run once without TUI — for Docker, cron, or systemd
    Once,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Setup => {
            config::setup_wizard(&cli.config)?;
        }
        Commands::Auth => {
            bot::browser_auth(&cli.config).await?;
        }
        Commands::Once => {
            let cfg = config::load_or_create(&cli.config)?;
            bot::run_once(&cfg).await?;
        }
        Commands::Run => {
            let cfg = config::load_or_create(&cli.config)?;
            tui::run(cfg).await?;
        }
    }

    Ok(())
}
