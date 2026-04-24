use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("tab/logs")
}

pub fn log_file(component: &str) -> PathBuf {
    log_dir().join(format!("{component}.log"))
}

#[cfg(unix)]
pub fn runtime_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join(format!("tab-{uid}"));
        }
    }
    let tmpdir = std::env::var_os("TMPDIR")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/tmp".into());
    PathBuf::from(tmpdir).join(format!("tab-{uid}"))
}

#[cfg(windows)]
pub fn runtime_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_default())
        .join("tab")
}

pub fn socket_file() -> PathBuf {
    runtime_dir().join("shell.sock")
}

pub fn pid_file() -> PathBuf {
    runtime_dir().join("daemon.pid")
}

pub fn default_history_path(shell: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    match shell {
        "zsh" => Some(
            std::env::var_os("HISTFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".zsh_history")),
        ),
        "bash" => Some(
            std::env::var_os("HISTFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".bash_history")),
        ),
        "fish" => {
            if let Some(base) = dirs::data_dir() {
                Some(base.join("fish/fish_history"))
            } else {
                Some(home.join(".local/share/fish/fish_history"))
            }
        }
        "pwsh" => {
            #[cfg(windows)]
            {
                dirs::data_dir().map(|d| {
                    d.join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt")
                })
            }
            #[cfg(not(windows))]
            {
                Some(home.join(".local/share/powershell/PSReadLine/ConsoleHost_history.txt"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_lands_under_log_dir() {
        let p = log_file("daemon");
        assert!(p.starts_with(log_dir()));
        assert!(p.to_string_lossy().ends_with("daemon.log"));
    }

    #[test]
    fn socket_file_under_runtime_dir() {
        let p = socket_file();
        assert!(p.starts_with(runtime_dir()));
        assert!(p.to_string_lossy().ends_with("shell.sock"));
    }

    #[test]
    fn pid_file_under_runtime_dir() {
        assert!(pid_file().starts_with(runtime_dir()));
    }

    #[test]
    fn default_history_paths_known_shells() {
        for shell in &["zsh", "bash", "fish", "pwsh"] {
            let p = default_history_path(shell);
            assert!(p.is_some(), "{shell} should have a default path");
        }
    }

    #[test]
    fn default_history_unknown_shell() {
        assert!(default_history_path("tcsh").is_none());
        assert!(default_history_path("").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_dir_uses_tmpdir_when_no_xdg() {
        // Can't mutate process env safely in tests, but we can at least verify
        // the current-process runtime_dir is absolute (not relative like
        // "tab-501" from the earlier empty-XDG bug).
        let d = runtime_dir();
        assert!(d.is_absolute(), "runtime_dir must be absolute: {d:?}");
    }

    #[test]
    fn log_dir_is_absolute() {
        assert!(log_dir().is_absolute());
    }
}
