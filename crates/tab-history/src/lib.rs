pub mod bash;
pub mod fish;
pub mod pwsh;
pub mod scorer;
pub mod source;
pub mod watcher;
pub mod zsh;

pub use scorer::HistoryIndex;
pub use source::{load_all, HistorySource, ShellKind};
pub use watcher::HistoryWatcher;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub command: String,
    pub timestamp: i64,
    pub duration: u32,
}
