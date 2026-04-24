use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{RwLock, Semaphore};

use interprocess::local_socket::tokio::prelude::*;
use tab_core::{ipc, Config, QueryRequest};
use tab_history::HistoryWatcher;

/// Maximum bytes read from a single client request line. Above this we drop
/// the connection — either a runaway client or an attacker.
const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Soft cap on the user's `buffer` query string. Anything longer is truncated
/// to protect the scorer from O(N*M) blowup.
const MAX_BUFFER_CHARS: usize = 2048;

/// Max concurrent client connections. A misbehaving shell can't block everyone.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

/// Idle-read timeout — drop slowloris clients.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct DaemonState {
    pub history: Arc<tab_history::HistoryIndex>,
    pub config: Arc<Config>,
}

pub async fn run() -> Result<()> {
    let config = Config::load();
    let (watcher, history) = HistoryWatcher::new(&config)?;

    let state = Arc::new(RwLock::new(DaemonState {
        history: Arc::new(history),
        config: Arc::new(config),
    }));

    // Reload loop: refresh config + index on disk changes + rotate log.
    let reload_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut tick_count: u64 = 0;
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            tick_count = tick_count.wrapping_add(1);

            let new_config = Config::load();
            let new_index = watcher.check_reload(&new_config);
            let mut guard = reload_state.write().await;
            if let Some(idx) = new_index {
                guard.history = Arc::new(idx);
            }
            guard.config = Arc::new(new_config);
            drop(guard);

            // Check log rotation every ~60s (30 ticks × 2s) — cheap stat call.
            if tick_count.is_multiple_of(30) {
                tab_core::logging::rotate_component("daemon");
            }
        }
    });

    let listener = ipc::prepare_listener()?;
    tracing::info!("listening on local socket");

    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    let mut accept_fail_streak = 0u32;
    let mut accept_backoff = Duration::from_millis(50);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received; exiting");
                return Ok(());
            }
            result = listener.accept() => {
                let stream = match result {
                    Ok(s) => {
                        accept_fail_streak = 0;
                        accept_backoff = Duration::from_millis(50);
                        s
                    }
                    Err(e) => {
                        accept_fail_streak += 1;
                        tracing::error!("accept error #{accept_fail_streak}: {e}");
                        if accept_fail_streak > 50 {
                            anyhow::bail!("too many accept errors, exiting");
                        }
                        tokio::time::sleep(accept_backoff).await;
                        accept_backoff = (accept_backoff * 2).min(Duration::from_secs(5));
                        continue;
                    }
                };

                let permit = match sem.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!("connection cap {MAX_CONCURRENT_CONNECTIONS} hit; dropping new client");
                        drop(stream);
                        continue;
                    }
                };

                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = handle_connection(stream, state).await {
                        tracing::debug!("connection ended: {e}");
                    }
                });
            }
        }
    }
}

/// Await Ctrl-C on any platform + SIGTERM on Unix.
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn handle_connection<S>(stream: S, state: Arc<RwLock<DaemonState>>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut line = Vec::<u8>::new();

    tracing::debug!("new connection");

    loop {
        line.clear();
        let read_result =
            tokio::time::timeout(READ_TIMEOUT, reader.read_until(b'\n', &mut line)).await;

        let n = match read_result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                tracing::debug!("read timeout, dropping connection");
                return Ok(());
            }
        };

        if n == 0 {
            break;
        }
        if line.len() > MAX_REQUEST_BYTES {
            tracing::warn!("request exceeded {MAX_REQUEST_BYTES} bytes, dropping connection");
            return Ok(());
        }

        let text = match std::str::from_utf8(&line) {
            Ok(s) => s.trim(),
            Err(_) => {
                tracing::warn!("non-UTF8 request, dropping line");
                continue;
            }
        };

        let mut req: QueryRequest = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("invalid request: {e}");
                continue;
            }
        };

        // Clamp buffer length: nothing the scorer can meaningfully use is
        // longer than a few hundred chars, and a huge buffer is a blowup vector.
        if req.buffer.chars().count() > MAX_BUFFER_CHARS {
            let truncated: String = req.buffer.chars().take(MAX_BUFFER_CHARS).collect();
            req.buffer = truncated;
        }

        // Snapshot under read lock, release before CPU-heavy query so
        // concurrent clients aren't serialised.
        let (history, config) = {
            let guard = state.read().await;
            (Arc::clone(&guard.history), Arc::clone(&guard.config))
        };

        let resp = crate::query::handle(req, &history, &config);
        let json = serde_json::to_string(&resp)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    Ok(())
}
