use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use tab_core::{socket_dir, Candidate, CandidateSource, QueryRequest, QueryResponse};
use tab_history::{HistoryIndex, HistoryWatcher};

const MAX_CANDIDATES: usize = 8;

struct DaemonState {
    history: HistoryIndex,
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

    let state = Arc::new(Mutex::new(DaemonState { history }));

    // Periodic history reload
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
    state: Arc<Mutex<DaemonState>>,
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

async fn handle_query(req: QueryRequest, state: &Arc<Mutex<DaemonState>>) -> QueryResponse {
    let mut state = state.lock().await;

    let match_mode = if req.match_mode.is_empty() {
        let config = tab_core::Config::load();
        config.completion.match_mode
    } else {
        req.match_mode.clone()
    };

    let script_candidates = crate::scripts::query_scripts(&req.buffer, &req.cwd, MAX_CANDIDATES);

    // For path commands (cd, ls, etc.), only show filesystem candidates — no history noise
    if crate::paths::is_path_command(&req.buffer) {
        let path_candidates = crate::paths::query_paths(&req.buffer, &req.cwd, MAX_CANDIDATES);
        let candidates = merge_candidates(script_candidates, path_candidates, MAX_CANDIDATES);
        return QueryResponse { candidates };
    }

    let history_candidates =
        state
            .history
            .query(&req.buffer, &req.cwd, MAX_CANDIDATES, &match_mode);
    let history_candidates =
        crate::scripts::filter_irrelevant_pm_commands(history_candidates, &req.cwd);

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
