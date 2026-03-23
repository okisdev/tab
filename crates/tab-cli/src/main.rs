mod hook;
mod init;
mod service;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "tab", about = "Terminal autocomplete plugin")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Output shell integration script
    Init {
        /// Shell type: zsh, bash, fish
        shell: String,
    },

    /// Run in hook/coprocess mode (used by shell integration)
    Hook {
        /// Shell type
        #[arg(long)]
        shell: String,

        /// Session identifier
        #[arg(long)]
        session: String,
    },

    /// Start the daemon manually (foreground)
    Start,

    /// Check daemon status
    Status,

    /// Install tab (launchd service + shell integration hint)
    Install,

    /// Uninstall tab (stop daemon + remove launchd service)
    Uninstall,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { shell } => init::print_init_script(&shell)?,
        Commands::Hook { shell, session } => hook::run_hook(&shell, &session)?,
        Commands::Start => service::start_foreground()?,
        Commands::Status => service::status()?,
        Commands::Install => service::install()?,
        Commands::Uninstall => service::uninstall()?,
    }

    Ok(())
}
