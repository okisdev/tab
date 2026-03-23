#[allow(dead_code)]
mod accessibility;
mod connection;
mod view;
mod window;

use anyhow::Result;
use std::sync::mpsc;
use tracing_subscriber::EnvFilter;

use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::MainThreadMarker;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("tab-overlay starting");

    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Channel for receiving messages from daemon connection thread
    let (tx, rx) = mpsc::channel::<tab_core::OverlayMessage>();

    // Start daemon connection in background thread
    connection::start_connection_thread(tx);

    // Create the overlay panel
    let panel = window::create_panel(mtm);
    let content_view = view::create_candidate_view(mtm);
    window::setup_panel(&panel, &content_view, mtm);

    // Poll for messages using a timer on the main thread
    window::start_message_poll(rx, &panel, &content_view, mtm);

    tracing::info!("entering run loop");
    app.run();

    Ok(())
}
