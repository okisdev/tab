use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

use crate::Config;

const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024; // 5 MB

/// Returns the log directory: `~/.local/share/tab/logs/`
pub fn log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("tab/logs")
}

/// Returns the log file path for a component.
pub fn log_file(component: &str) -> PathBuf {
    log_dir().join(format!("{component}.log"))
}

/// Initialize file-based logging for a tab component.
///
/// Log level priority: `TAB_LOG` env var > config.toml `[log] level` > `default_level`.
pub fn init(component: &str, default_level: &str) {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);

    let path = log_file(component);
    maybe_rotate(&path);

    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return, // silently fail — don't crash the component over logging
    };

    let level = resolve_level(default_level);

    let filter = EnvFilter::try_from_env("TAB_LOG").unwrap_or_else(|_| EnvFilter::new(&level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(true)
        .init();
}

/// Resolve log level: TAB_LOG env > config.toml > default.
fn resolve_level(default: &str) -> String {
    if let Ok(val) = std::env::var("TAB_LOG") {
        return val;
    }

    let config = Config::load();
    if !config.log.level.is_empty() {
        return config.log.level;
    }

    default.to_string()
}

/// Rotate if file exceeds MAX_LOG_SIZE. Keeps exactly one `.old` backup.
fn maybe_rotate(path: &PathBuf) {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_LOG_SIZE {
        let old = path.with_extension("log.old");
        let _ = fs::rename(path, old);
    }
}
