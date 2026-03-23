use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;

use tab_core::OverlayMessage;

/// Start a background thread that connects to the daemon's overlay socket
/// and forwards messages to the main thread via the channel.
pub fn start_connection_thread(tx: Sender<OverlayMessage>) {
    std::thread::spawn(move || {
        if let Err(e) = connection_loop(&tx) {
            tracing::error!("overlay connection error: {e}");
        }
    });
}

fn connection_loop(tx: &Sender<OverlayMessage>) -> anyhow::Result<()> {
    let socket_path = tab_core::overlay_socket_path();

    // Retry connection loop
    loop {
        tracing::info!("connecting to daemon at {:?}", socket_path);

        let stream = match connect_with_retry(&socket_path, 10) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to connect: {e}, retrying in 2s");
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };

        tracing::info!("connected to daemon");
        let reader = BufReader::new(stream);

        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let msg: OverlayMessage = match serde_json::from_str(&line) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!("invalid message: {e}");
                            continue;
                        }
                    };
                    if tx.send(msg).is_err() {
                        tracing::info!("main thread dropped, exiting");
                        return Ok(());
                    }
                }
                Err(e) => {
                    tracing::warn!("read error: {e}, reconnecting");
                    break;
                }
            }
        }

        // Connection lost, retry
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn connect_with_retry(
    path: &std::path::Path,
    max_attempts: u32,
) -> anyhow::Result<UnixStream> {
    for attempt in 1..=max_attempts {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if attempt == max_attempts {
                    anyhow::bail!("failed to connect after {max_attempts} attempts: {e}");
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    unreachable!()
}
