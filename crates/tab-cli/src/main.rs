mod hook;
mod init;
mod logs;
mod service;
mod settings;
mod term;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "tab", about = "Cross-platform terminal autocomplete plugin")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Emit shell integration script for eval/source
    Init {
        /// zsh, bash, fish, or pwsh
        shell: String,
    },

    /// Long-lived coprocess bridging a shell to the daemon
    Hook,

    /// Interactive picker — print the selected text to stdout
    Complete {
        #[arg(long)]
        buffer: String,
        #[arg(long)]
        cwd: String,
    },

    /// Run the daemon in the foreground (for manual testing)
    Start,

    /// Report daemon and service status
    Status,

    /// Install the daemon as a login-time service
    Install,

    /// Stop and remove the service
    Uninstall,

    /// Interactive settings
    Settings,

    /// Show or tail tab log files
    Logs {
        #[arg(default_value = "all")]
        component: String,
        #[arg(short, long)]
        follow: bool,
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: u32,
    },

    /// Diagnose the environment (shells, daemon, history paths)
    Doctor,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Complete { .. } => {
            // TUI needs a clean stderr — don't attach a logger.
        }
        Commands::Hook => {
            // File logging only — stdout/stderr belong to the shell protocol.
            // `info` default: reconnect attempts + session lifecycle land in
            // the log so "completions stopped working" has something to read.
            tab_core::logging::init("hook", "info");
        }
        _ => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_env("TAB_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .with_writer(std::io::stderr)
                .try_init();
        }
    }

    match cli.command {
        Commands::Init { shell } => init::print_init_script(&shell)?,
        Commands::Hook => hook::run_hook()?,
        Commands::Complete { buffer, cwd } => match tui::run(&buffer, &cwd)? {
            Some(text) => {
                print!("{text}");
                std::process::exit(0);
            }
            None => std::process::exit(1),
        },
        Commands::Start => service::start_foreground()?,
        Commands::Status => service::status()?,
        Commands::Install => service::install()?,
        Commands::Uninstall => service::uninstall()?,
        Commands::Settings => settings::run()?,
        Commands::Logs {
            component,
            follow,
            lines,
        } => logs::show(&component, follow, lines)?,
        Commands::Doctor => service::doctor()?,
    }

    Ok(())
}
