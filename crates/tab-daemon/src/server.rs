use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::RwLock;

use tab_core::{socket_dir, Candidate, CandidateSource, QueryRequest, QueryResponse};
use tab_history::{HistoryIndex, HistoryWatcher};

const MAX_CANDIDATES: usize = 8;

struct DaemonState {
    history: HistoryIndex,
    config: tab_core::Config,
}

pub async fn run() -> Result<()> {
    let sock_dir = socket_dir();
    std::fs::create_dir_all(&sock_dir)?;

    let shell_sock = sock_dir.join("shell.sock");
    let _ = std::fs::remove_file(&shell_sock);

    let history_path = dirs::home_dir()
        .map(|h: std::path::PathBuf| h.join(".zsh_history"))
        .unwrap_or_default();

    let (watcher, history) = HistoryWatcher::new(&history_path)?;

    let config = tab_core::Config::load();
    let state = Arc::new(RwLock::new(DaemonState { history, config }));

    // Periodic history + config reload
    let reload_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let new_index = watcher.check_reload();
            let new_config = tab_core::Config::load();
            let mut state = reload_state.write().await;
            if let Some(idx) = new_index {
                state.history = idx;
            }
            state.config = new_config;
        }
    });

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sock_dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let listener = UnixListener::bind(&shell_sock)?;
    tracing::info!("listening on {:?}", shell_sock);

    loop {
        let (stream, _) = listener.accept().await?;
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
    state: Arc<RwLock<DaemonState>>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    tracing::info!("new connection");

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let req: QueryRequest = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("invalid request: {e}");
                continue;
            }
        };

        let resp = handle_query(req, &state).await;
        let json = serde_json::to_string(&resp)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    Ok(())
}

async fn handle_query(req: QueryRequest, state: &Arc<RwLock<DaemonState>>) -> QueryResponse {
    // Perform all filesystem I/O before acquiring the lock
    let scripts = crate::scripts::read_scripts(&req.cwd);
    let script_candidates =
        crate::scripts::query_scripts_with(&req.buffer, &scripts, MAX_CANDIDATES);

    // For path commands (cd, ls, etc.), only show filesystem candidates — no history noise
    // No lock needed for this path.
    if crate::paths::is_path_command(&req.buffer) {
        let path_candidates = crate::paths::query_paths(&req.buffer, &req.cwd, MAX_CANDIDATES);
        let candidates = merge_candidates(script_candidates, path_candidates, MAX_CANDIDATES);
        return QueryResponse { candidates };
    }

    // Only hold the lock for in-memory history query
    let mut state = state.write().await;

    let match_mode = if req.match_mode.is_empty() {
        state.config.completion.match_mode.clone()
    } else {
        req.match_mode.clone()
    };

    let history_candidates =
        state
            .history
            .query(&req.buffer, &req.cwd, MAX_CANDIDATES, &match_mode);
    drop(state); // release lock before remaining CPU work

    let history_candidates =
        crate::scripts::filter_irrelevant_pm_commands_with(history_candidates, &scripts);

    let candidates = merge_candidates(script_candidates, history_candidates, MAX_CANDIDATES);

    QueryResponse { candidates }
}

/// Merge script candidates (priority) with history candidates, deduplicating by text.
fn merge_candidates(
    scripts: Vec<Candidate>,
    history: Vec<Candidate>,
    max: usize,
) -> Vec<Candidate> {
    let history_texts: HashSet<&str> = history.iter().map(|c| c.text.as_str()).collect();

    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(max);

    for mut c in scripts {
        if seen.insert(c.text.clone()) {
            if history_texts.contains(c.text.as_str()) {
                c.source = CandidateSource::ScriptHistory;
            }
            result.push(c);
        }
        if result.len() >= max {
            return result;
        }
    }

    for c in history {
        if seen.insert(c.text.clone()) {
            result.push(c);
        }
        if result.len() >= max {
            break;
        }
    }

    result
}
