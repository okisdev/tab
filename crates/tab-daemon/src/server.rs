use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use tab_core::{
    socket_dir, DaemonToShellMessage, Direction, OverlayMessage, ShellMessage,
};
use tab_history::{HistoryIndex, HistoryWatcher};

use crate::session::Session;

const MAX_CANDIDATES: usize = 8;

struct DaemonState {
    history: HistoryIndex,
    sessions: HashMap<String, Session>,
    /// Connected overlay writer (if any)
    overlay_writer: Option<tokio::net::unix::OwnedWriteHalf>,
    /// Whether overlay process has been spawned
    overlay_spawned: bool,
}

impl DaemonState {
    async fn send_to_overlay(&mut self, msg: &OverlayMessage) {
        // Auto-spawn overlay if not yet running
        if !self.overlay_spawned {
            self.spawn_overlay();
        }

        if let Some(ref mut writer) = self.overlay_writer {
            let json = match serde_json::to_string(msg) {
                Ok(j) => j,
                Err(_) => return,
            };
            let result = async {
                writer.write_all(json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await
            }
            .await;

            if result.is_err() {
                tracing::warn!("overlay connection lost");
                self.overlay_writer = None;
                self.overlay_spawned = false;
            }
        }
    }

    fn spawn_overlay(&mut self) {
        // Find tab-overlay binary next to tab-daemon
        let overlay_bin = match std::env::current_exe() {
            Ok(p) => p.parent().unwrap().join("tab-overlay"),
            Err(_) => return,
        };

        if !overlay_bin.exists() {
            tracing::warn!("tab-overlay not found at {:?}", overlay_bin);
            return;
        }

        match std::process::Command::new(&overlay_bin)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                tracing::info!("spawned tab-overlay");
                self.overlay_spawned = true;
            }
            Err(e) => {
                tracing::error!("failed to spawn tab-overlay: {e}");
            }
        }
    }
}

pub async fn run() -> Result<()> {
    let sock_dir = socket_dir();
    std::fs::create_dir_all(&sock_dir)?;

    let shell_sock = sock_dir.join("shell.sock");
    let overlay_sock = sock_dir.join("overlay.sock");

    // Remove stale socket files
    let _ = std::fs::remove_file(&shell_sock);
    let _ = std::fs::remove_file(&overlay_sock);

    // Load ZSH history with file watcher
    let history_path = dirs::home_dir()
        .map(|h: std::path::PathBuf| h.join(".zsh_history"))
        .unwrap_or_default();

    let (watcher, history) = HistoryWatcher::new(&history_path)?;

    let state = Arc::new(Mutex::new(DaemonState {
        history,
        sessions: HashMap::new(),
        overlay_writer: None,
        overlay_spawned: false,
    }));

    // Periodic history reload check (every 2 seconds)
    let reload_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            if let Some(new_index) = watcher.check_reload() {
                reload_state.lock().await.history = new_index;
            }
        }
    });

    // Set socket directory permissions (user-only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sock_dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let shell_listener = UnixListener::bind(&shell_sock)?;
    tracing::info!("shell socket: {:?}", shell_sock);

    let overlay_listener = UnixListener::bind(&overlay_sock)?;
    tracing::info!("overlay socket: {:?}", overlay_sock);

    // Accept overlay connections
    let overlay_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            match overlay_listener.accept().await {
                Ok((stream, _)) => {
                    tracing::info!("overlay connected");
                    let (_reader, writer) = stream.into_split();
                    overlay_state.lock().await.overlay_writer = Some(writer);
                }
                Err(e) => {
                    tracing::error!("overlay accept error: {e}");
                }
            }
        }
    });

    // Accept shell connections
    loop {
        let (stream, _) = shell_listener.accept().await?;
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state).await {
                tracing::error!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    state: Arc<Mutex<DaemonState>>,
) -> Result<()> {
    use std::time::Duration;
    use tokio::time::{sleep, Instant};

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    tracing::info!("new shell connection");

    // Debounce state for context messages
    let mut pending_context: Option<tab_core::ShellContext> = None;
    let mut debounce_deadline: Option<Instant> = None;
    const DEBOUNCE_MS: u64 = 50;

    loop {
        line.clear();
        let n = if let Some(deadline) = debounce_deadline {
            tokio::select! {
                result = reader.read_line(&mut line) => result?,
                _ = sleep(deadline.duration_since(Instant::now()).max(Duration::ZERO)) => 0,
            }
        } else {
            reader.read_line(&mut line).await?
        };

        if n == 0 && debounce_deadline.is_none() {
            tracing::info!("shell connection closed");
            break;
        }

        // If we got data, parse it
        if n > 0 {
            let msg: ShellMessage = match serde_json::from_str(line.trim()) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("invalid message: {e}");
                    continue;
                }
            };

            match msg {
                ShellMessage::Context(ctx) => {
                    // Debounce: store and wait
                    pending_context = Some(ctx);
                    debounce_deadline = Some(Instant::now() + Duration::from_millis(DEBOUNCE_MS));
                    continue;
                }
                other => {
                    // Navigate/Accept/Dismiss: process immediately, flush pending context first
                    if let Some(ctx) = pending_context.take() {
                        debounce_deadline = None;
                        handle_message(ShellMessage::Context(ctx), &state).await;
                    }

                    let response = handle_message(other, &state).await;
                    if let Some(resp) = response {
                        let json = serde_json::to_string(&resp)?;
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                    }
                }
            }
        } else if debounce_deadline.is_some() {
            // Timeout fired — process the pending context
            if let Some(ctx) = pending_context.take() {
                debounce_deadline = None;
                handle_message(ShellMessage::Context(ctx), &state).await;
                // Context has no response to shell
            }
        }
    }

    Ok(())
}

async fn handle_message(
    msg: ShellMessage,
    state: &Arc<Mutex<DaemonState>>,
) -> Option<DaemonToShellMessage> {
    let mut state = state.lock().await;

    match msg {
        ShellMessage::Context(ctx) => {
            let candidates = state.history.query(&ctx.buffer, &ctx.cwd, MAX_CANDIDATES);

            let session = state
                .sessions
                .entry(ctx.session_id.clone())
                .or_insert_with(Session::new);
            session.update_candidates(candidates.clone());

            // Forward to overlay only (no response to shell for context)
            let overlay_msg = OverlayMessage::Show {
                session_id: ctx.session_id,
                candidates,
                selected: 0,
            };
            state.send_to_overlay(&overlay_msg).await;

            None
        }

        ShellMessage::Accept { session_id, index } => {
            let session = state.sessions.get_mut(&session_id)?;
            session.selected_index = index;
            let text = session.accepted_text()?.to_string();

            // Hide overlay
            let overlay_msg = OverlayMessage::Hide {
                session_id: session_id.clone(),
            };
            state.send_to_overlay(&overlay_msg).await;

            Some(DaemonToShellMessage::Inject {
                session_id,
                text,
                replace_from: 0,
            })
        }

        ShellMessage::Navigate {
            session_id,
            direction,
        } => {
            let session = state.sessions.get_mut(&session_id)?;
            match direction {
                Direction::Up => session.navigate_up(),
                Direction::Down => session.navigate_down(),
            }

            let selected_index = session.selected_index;
            let items = session.last_candidates.clone();

            // Update overlay selection
            let overlay_msg = OverlayMessage::Select {
                session_id: session_id.clone(),
                index: selected_index,
            };
            state.send_to_overlay(&overlay_msg).await;

            Some(DaemonToShellMessage::Candidates {
                session_id,
                items,
                selected: selected_index,
            })
        }

        ShellMessage::Dismiss { session_id } => {
            // Hide overlay
            let overlay_msg = OverlayMessage::Hide {
                session_id: session_id.clone(),
            };
            state.send_to_overlay(&overlay_msg).await;

            state.sessions.remove(&session_id);
            None
        }
    }
}
