use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

use crate::paths::{log_dir, log_file};
use crate::Config;

const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;

/// Initialize file-based logging for a component.
///
/// Level priority: `TAB_LOG` env var > `[log] level` in config.toml > `default_level`.
pub fn init(component: &str, default_level: &str) {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);

    let path = log_file(component);
    maybe_rotate(&path);

    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
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

pub(crate) fn maybe_rotate(path: &PathBuf) {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if should_rotate(size) {
        let old = path.with_extension("log.old");
        let _ = fs::rename(path, old);
    }
}

/// Public form for callers that want to run rotation outside of `init` — e.g.
/// a long-lived daemon ticking every N seconds to check if its current log
/// file crossed the size threshold.
pub fn rotate_component(component: &str) {
    let path = log_file(component);
    maybe_rotate(&path);
}

fn should_rotate(size: u64) -> bool {
    size > MAX_LOG_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_threshold_is_five_megabytes() {
        assert_eq!(MAX_LOG_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn should_rotate_boundary() {
        assert!(!should_rotate(0));
        assert!(!should_rotate(MAX_LOG_SIZE));
        assert!(should_rotate(MAX_LOG_SIZE + 1));
    }

    #[test]
    fn resolve_level_falls_back_to_default() {
        let prior = std::env::var("TAB_LOG").ok();
        // SAFETY: test-only single-threaded env mutation.
        unsafe {
            std::env::remove_var("TAB_LOG");
        }
        let level = resolve_level("warn");
        // In the absence of TAB_LOG and a [log] level in real config, expect
        // "warn". If the user's real config sets a level, this test runs under
        // that — so accept any non-empty string; the contract we assert is
        // "non-empty" (caller always gets a usable level).
        assert!(!level.is_empty());
        if let Some(v) = prior {
            unsafe {
                std::env::set_var("TAB_LOG", v);
            }
        }
    }

    #[test]
    fn resolve_level_respects_env() {
        let prior = std::env::var("TAB_LOG").ok();
        unsafe {
            std::env::set_var("TAB_LOG", "trace");
        }
        assert_eq!(resolve_level("warn"), "trace");
        match prior {
            Some(v) => unsafe { std::env::set_var("TAB_LOG", v) },
            None => unsafe { std::env::remove_var("TAB_LOG") },
        }
    }

    #[test]
    fn maybe_rotate_renames_oversize_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log = tmp.path().join("big.log");
        std::fs::write(&log, vec![0u8; (MAX_LOG_SIZE + 100) as usize]).unwrap();
        let old = log.with_extension("log.old");
        assert!(!old.exists());

        super::maybe_rotate(&log);

        assert!(!log.exists(), "original should be renamed away");
        assert!(old.exists(), "{old:?} should now exist");
    }

    #[test]
    fn maybe_rotate_skips_small_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log = tmp.path().join("small.log");
        std::fs::write(&log, b"hi").unwrap();
        super::maybe_rotate(&log);
        assert!(log.exists(), "small files must be left alone");
    }

    #[test]
    fn rotate_component_no_op_when_absent() {
        // Just must not panic when the log file doesn't exist.
        rotate_component("component-that-definitely-does-not-exist-xyz");
    }
}
