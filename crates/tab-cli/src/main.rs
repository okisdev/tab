mod hook;
mod init;
mod logs;
mod service;
mod settings;
mod tui;

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

    /// Run as coprocess for real-time completions
    Hook,

    /// Interactive completion TUI (called by shell widget on Tab)
    Complete {
        /// Current shell buffer content
        #[arg(long)]
        buffer: String,

        /// Current working directory
        #[arg(long)]
        cwd: String,
    },

    /// Start the daemon manually (foreground)
    Start,

    /// Check daemon status
    Status,

    /// Install tab (launchd service + shell integration hint)
    Install,

    /// Uninstall tab (stop daemon + remove launchd service)
    Uninstall,

    /// Interactive settings configuration
    Settings,

    /// View log files
    Logs {
        /// Component: daemon, hook, shell, or all
        #[arg(default_value = "all")]
        component: String,

        /// Follow/tail the log
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Complete subcommand must not log to stderr (corrupts TUI)
    match &cli.command {
        Commands::Complete { .. } => {}
        _ => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_env("TAB_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .with_writer(std::io::stderr)
                .init();
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
            None => {
                std::process::exit(1);
            }
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
    }

    Ok(())
}
