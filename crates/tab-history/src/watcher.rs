use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::parser::parse_zsh_history;
use crate::scorer::HistoryIndex;

/// Watches a history file and rebuilds the index on changes.
pub struct HistoryWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<()>,
    path: PathBuf,
}

impl HistoryWatcher {
    /// Start watching a history file. Returns the watcher and an initial index.
    pub fn new(path: &Path) -> anyhow::Result<(Self, HistoryIndex)> {
        let entries = if path.exists() {
            parse_zsh_history(path)?
        } else {
            Vec::new()
        };
        let index = HistoryIndex::from_entries(&entries);
        tracing::info!("loaded {} history entries from {:?}", entries.len(), path);

        let (tx, rx) = mpsc::channel();
        let watch_path = path.to_path_buf();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        let _ = tx.send(());
                    }
                    _ => {}
                }
            }
        })?;

        // Watch the parent directory (history file may be replaced atomically)
        let parent = path.parent().unwrap_or(Path::new("/"));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;

        Ok((
            HistoryWatcher {
                _watcher: watcher,
                rx,
                path: watch_path,
            },
            index,
        ))
    }

    /// Check if the history file changed and rebuild index if so.
    /// Returns `Some(new_index)` if rebuilt, `None` if unchanged.
    pub fn check_reload(&self) -> Option<HistoryIndex> {
        // Drain all pending notifications
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }

        if !changed {
            return None;
        }

        match parse_zsh_history(&self.path) {
            Ok(entries) => {
                tracing::info!("reloaded {} history entries", entries.len());
                Some(HistoryIndex::from_entries(&entries))
            }
            Err(e) => {
                tracing::warn!("failed to reload history: {e}");
                None
            }
        }
    }
}
