mod parser;
mod scorer;
mod watcher;

pub use parser::parse_zsh_history;
pub use scorer::HistoryIndex;
pub use watcher::HistoryWatcher;

/// A single history entry parsed from a shell history file.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub command: String,
    pub timestamp: i64,
    pub duration: u32,
}
