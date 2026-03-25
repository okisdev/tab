mod paths;
mod scripts;
mod server;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tab_core::logging::init("daemon", "info");

    tracing::info!("tab-daemon starting");
    server::run().await
}
