use std::path::{Path, PathBuf};

use tab_core::{paths::default_history_path, Config};

use crate::HistoryEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    Pwsh,
}

impl ShellKind {
    pub fn all() -> &'static [ShellKind] {
        &[
            ShellKind::Zsh,
            ShellKind::Bash,
            ShellKind::Fish,
            ShellKind::Pwsh,
        ]
    }

    pub fn parse_from(name: &str) -> Option<ShellKind> {
        match name.to_ascii_lowercase().as_str() {
            "zsh" => Some(ShellKind::Zsh),
            "bash" => Some(ShellKind::Bash),
            "fish" => Some(ShellKind::Fish),
            "pwsh" | "powershell" | "psreadline" => Some(ShellKind::Pwsh),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ShellKind::Zsh => "zsh",
            ShellKind::Bash => "bash",
            ShellKind::Fish => "fish",
            ShellKind::Pwsh => "pwsh",
        }
    }
}

pub trait HistorySource {
    fn parse(path: &Path) -> anyhow::Result<Vec<HistoryEntry>>;
}

/// Resolve the configured `(shell, path)` tuples regardless of whether the
/// file currently exists. Used by the watcher to pick which parent dirs to
/// monitor so newly-created history files are picked up live.
pub fn configured_sources(config: &Config) -> Vec<(ShellKind, PathBuf)> {
    let mut out = Vec::new();

    let wants = |s: ShellKind| -> bool {
        config.history.sources.iter().any(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "auto" || v == s.as_str()
        })
    };

    for shell in ShellKind::all() {
        if !wants(*shell) {
            continue;
        }
        let override_path = match shell {
            ShellKind::Zsh => config.history.zsh_path.clone(),
            ShellKind::Bash => config.history.bash_path.clone(),
            ShellKind::Fish => config.history.fish_path.clone(),
            ShellKind::Pwsh => config.history.pwsh_path.clone(),
        };
        let path = override_path.or_else(|| default_history_path(shell.as_str()));
        if let Some(p) = path {
            out.push((*shell, p));
        }
    }

    out
}

/// Existing-file subset of [`configured_sources`]. What `load_all` actually
/// reads.
pub fn resolve_sources(config: &Config) -> Vec<(ShellKind, PathBuf)> {
    configured_sources(config)
        .into_iter()
        .filter(|(_, p)| p.exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tab_core::config::{CompletionConfig, HistoryConfig, LogConfig};
    use tempfile::TempDir;

    fn cfg(sources: &[&str], overrides: HistoryConfig) -> Config {
        Config {
            completion: CompletionConfig::default(),
            log: LogConfig::default(),
            history: HistoryConfig {
                sources: sources.iter().map(|s| (*s).into()).collect(),
                ..overrides
            },
        }
    }

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.path().join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn explicit_override_wins_over_default() {
        let dir = TempDir::new().unwrap();
        let zsh_override = write(&dir, "my_zsh_history", ": 1700000000:0;echo hi\n");

        let config = cfg(
            &["zsh"],
            HistoryConfig {
                zsh_path: Some(zsh_override.clone()),
                ..HistoryConfig::default()
            },
        );

        let sources = resolve_sources(&config);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].0, ShellKind::Zsh);
        assert_eq!(sources[0].1, zsh_override);
    }

    #[test]
    fn subset_excludes_other_shells() {
        let dir = TempDir::new().unwrap();
        let zsh = write(&dir, "zsh_hist", ": 1:0;a\n");
        let bash = write(&dir, "bash_hist", "a\n");

        let config = cfg(
            &["zsh"],
            HistoryConfig {
                zsh_path: Some(zsh),
                bash_path: Some(bash),
                ..HistoryConfig::default()
            },
        );

        let shells: Vec<ShellKind> = resolve_sources(&config)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(shells, vec![ShellKind::Zsh]);
    }

    #[test]
    fn missing_file_is_skipped() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does_not_exist");

        let config = cfg(
            &["zsh"],
            HistoryConfig {
                zsh_path: Some(missing),
                ..HistoryConfig::default()
            },
        );

        assert!(resolve_sources(&config).is_empty());
    }

    #[test]
    fn case_insensitive_source_names() {
        let dir = TempDir::new().unwrap();
        let zsh = write(&dir, "zsh_hist", ": 1:0;a\n");
        let config = cfg(
            &["ZSH"],
            HistoryConfig {
                zsh_path: Some(zsh),
                ..HistoryConfig::default()
            },
        );
        assert_eq!(resolve_sources(&config).len(), 1);
    }

    #[test]
    fn configured_sources_includes_absent_paths() {
        // configured_sources must include paths that don't exist yet — the
        // watcher uses it to watch parent dirs so newly-created history
        // files trigger a reload without a daemon restart.
        let dir = TempDir::new().unwrap();
        let future_path = dir.path().join("not_yet_created");
        let config = cfg(
            &["zsh"],
            HistoryConfig {
                zsh_path: Some(future_path.clone()),
                ..HistoryConfig::default()
            },
        );
        let conf = configured_sources(&config);
        assert_eq!(conf.len(), 1);
        assert_eq!(conf[0].1, future_path);

        // resolve_sources filters them out.
        assert!(resolve_sources(&config).is_empty());
    }
}

/// Load + merge entries from all configured history files.
pub fn load_all(config: &Config) -> Vec<HistoryEntry> {
    let mut all = Vec::new();
    for (shell, path) in resolve_sources(config) {
        let result = match shell {
            ShellKind::Zsh => crate::zsh::Zsh::parse(&path),
            ShellKind::Bash => crate::bash::Bash::parse(&path),
            ShellKind::Fish => crate::fish::Fish::parse(&path),
            ShellKind::Pwsh => crate::pwsh::Pwsh::parse(&path),
        };
        match result {
            Ok(mut entries) => {
                tracing::info!(
                    "loaded {} entries from {} ({:?})",
                    entries.len(),
                    shell.as_str(),
                    path
                );
                all.append(&mut entries);
            }
            Err(e) => tracing::warn!(
                "failed to parse {} history at {:?}: {e}",
                shell.as_str(),
                path
            ),
        }
    }
    all
}
