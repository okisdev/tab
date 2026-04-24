use std::path::PathBuf;
use std::sync::mpsc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use tab_core::Config;

use crate::scorer::HistoryIndex;
use crate::source::{configured_sources, load_all};

/// Watches every configured history file and rebuilds the index on changes.
pub struct HistoryWatcher {
    _watcher: Option<RecommendedWatcher>,
    rx: mpsc::Receiver<()>,
    watched_paths: Vec<PathBuf>,
}

impl HistoryWatcher {
    pub fn new(config: &Config) -> anyhow::Result<(Self, HistoryIndex)> {
        let entries = load_all(config);
        let index = HistoryIndex::from_entries(&entries);
        tracing::info!("indexed {} history entries", index.entry_count());

        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                if matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    let _ = tx.send(());
                }
            }
        })?;

        // Watch the *parent* directory of every configured history path —
        // including those that don't yet exist — so later creation triggers
        // a reload without daemon restart.
        let sources = configured_sources(config);
        let mut watched: Vec<PathBuf> = Vec::new();
        for (_, path) in &sources {
            if let Some(parent) = path.parent() {
                if !watched.iter().any(|p| p == parent) {
                    match watcher.watch(parent, RecursiveMode::NonRecursive) {
                        Ok(()) => watched.push(parent.to_path_buf()),
                        Err(e) => tracing::debug!("watch {parent:?} skipped: {e}"),
                    }
                }
            }
        }

        let watched_paths: Vec<PathBuf> = sources.into_iter().map(|(_, p)| p).collect();

        Ok((
            HistoryWatcher {
                _watcher: Some(watcher),
                rx,
                watched_paths,
            },
            index,
        ))
    }

    /// Drain pending events and rebuild if any file we care about changed.
    pub fn check_reload(&self, config: &Config) -> Option<HistoryIndex> {
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }
        if !changed {
            return None;
        }
        let entries = load_all(config);
        tracing::info!("reloaded history, {} entries", entries.len());
        Some(HistoryIndex::from_entries(&entries))
    }

    pub fn watched(&self) -> &[PathBuf] {
        &self.watched_paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tab_core::config::{CompletionConfig, HistoryConfig, LogConfig};

    fn empty_config() -> Config {
        Config {
            completion: CompletionConfig::default(),
            log: LogConfig::default(),
            history: HistoryConfig {
                sources: vec![], // no sources → no files watched
                ..HistoryConfig::default()
            },
        }
    }

    #[test]
    fn new_with_no_sources_builds_empty_index() {
        let (watcher, index) = HistoryWatcher::new(&empty_config()).expect("watcher");
        assert_eq!(index.entry_count(), 0);
        assert!(watcher.watched().is_empty());
    }

    #[test]
    fn new_with_tempfile_source_builds_non_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        let mut f = std::fs::File::create(&hist).unwrap();
        writeln!(f, ": 1700000000:0;ls -la").unwrap();
        writeln!(f, ": 1700000100:0;cd /tmp").unwrap();
        drop(f);

        let config = Config {
            completion: CompletionConfig::default(),
            log: LogConfig::default(),
            history: HistoryConfig {
                sources: vec!["zsh".into()],
                zsh_path: Some(hist),
                ..HistoryConfig::default()
            },
        };

        let (_watcher, index) = HistoryWatcher::new(&config).expect("watcher");
        assert_eq!(index.entry_count(), 2);
    }

    #[test]
    fn check_reload_returns_none_without_changes() {
        let (watcher, _index) = HistoryWatcher::new(&empty_config()).unwrap();
        let reloaded = watcher.check_reload(&empty_config());
        assert!(reloaded.is_none());
    }

    fn zsh_config(path: PathBuf) -> Config {
        Config {
            completion: CompletionConfig::default(),
            log: LogConfig::default(),
            history: HistoryConfig {
                sources: vec!["zsh".into()],
                zsh_path: Some(path),
                ..HistoryConfig::default()
            },
        }
    }

    // poll up to ~3s to absorb fsevents/inotify latency.
    fn wait_for_reload(watcher: &HistoryWatcher, config: &Config) -> Option<HistoryIndex> {
        for _ in 0..30 {
            if let Some(idx) = watcher.check_reload(config) {
                return Some(idx);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
    }

    #[test]
    fn reload_on_append() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        let mut f = std::fs::File::create(&hist).unwrap();
        writeln!(f, ": 1700000000:0;ls").unwrap();
        drop(f);

        let config = zsh_config(hist.clone());
        let (watcher, index) = HistoryWatcher::new(&config).unwrap();
        assert_eq!(index.entry_count(), 1);

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&hist)
            .unwrap();
        writeln!(f, ": 1700000100:0;pwd").unwrap();
        drop(f);

        let idx = wait_for_reload(&watcher, &config).expect("append should trigger reload");
        assert_eq!(idx.entry_count(), 2);
    }

    #[test]
    fn reload_on_atomic_replace() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        std::fs::write(&hist, ": 1:0;old\n").unwrap();

        let config = zsh_config(hist.clone());
        let (watcher, index) = HistoryWatcher::new(&config).unwrap();
        assert_eq!(index.entry_count(), 1);

        let tmp = dir.path().join("hist.tmp");
        std::fs::write(&tmp, ": 1:0;old\n: 2:0;fresh\n: 3:0;more\n").unwrap();
        std::fs::rename(&tmp, &hist).unwrap();

        let idx = wait_for_reload(&watcher, &config).expect("rename should trigger reload");
        assert_eq!(idx.entry_count(), 3);
    }

    #[test]
    fn reload_on_truncate() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        std::fs::write(&hist, ": 1:0;a\n: 2:0;b\n: 3:0;c\n").unwrap();

        let config = zsh_config(hist.clone());
        let (watcher, index) = HistoryWatcher::new(&config).unwrap();
        assert_eq!(index.entry_count(), 3);

        // open w/ truncate (equivalent to `echo > hist`)
        std::fs::File::create(&hist).unwrap();

        let idx = wait_for_reload(&watcher, &config).expect("truncate should trigger reload");
        assert_eq!(idx.entry_count(), 0);
    }

    #[test]
    fn reload_on_delete_then_recreate() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        std::fs::write(&hist, ": 1:0;a\n").unwrap();

        let config = zsh_config(hist.clone());
        let (watcher, index) = HistoryWatcher::new(&config).unwrap();
        assert_eq!(index.entry_count(), 1);

        std::fs::remove_file(&hist).unwrap();
        let removed = wait_for_reload(&watcher, &config);
        assert!(removed.is_some(), "rm should trigger reload");
        assert_eq!(removed.unwrap().entry_count(), 0);

        std::fs::write(&hist, ": 1:0;a\n: 2:0;b\n").unwrap();
        let recreated = wait_for_reload(&watcher, &config)
            .expect("recreate under same parent dir should still fire");
        assert_eq!(recreated.entry_count(), 2);
    }

    #[test]
    fn rapid_writes_coalesce_into_single_reload() {
        let dir = tempfile::tempdir().unwrap();
        let hist = dir.path().join("hist");
        std::fs::write(&hist, "").unwrap();

        let config = zsh_config(hist.clone());
        let (watcher, _) = HistoryWatcher::new(&config).unwrap();

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&hist)
            .unwrap();
        for i in 0..20 {
            writeln!(f, ": {}:0;cmd{}", 1_700_000_000 + i, i).unwrap();
        }
        drop(f);

        let idx = wait_for_reload(&watcher, &config).expect("reload");
        assert_eq!(idx.entry_count(), 20);
        // drain-all pattern: no extra reload queued after first check returned Some.
        assert!(watcher.check_reload(&config).is_none());
    }
}
