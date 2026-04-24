mod cargo_ctx;
mod compose_ctx;
mod context;
mod go_ctx;
mod make_ctx;
mod paths;
mod python_ctx;
mod query;
mod scripts;
mod server;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tab_core::logging::init("daemon", "info");
    tracing::info!("tab-daemon {} starting", env!("CARGO_PKG_VERSION"));

    if tab_core::ipc::ping() {
        tracing::warn!("another daemon appears to be running — exiting");
        eprintln!("tab-daemon: another instance is already running");
        std::process::exit(0);
    }

    server::run().await
}
