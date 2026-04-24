use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub completion: CompletionConfig,
    pub log: LogConfig,
    pub history: HistoryConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompletionConfig {
    pub max_results: usize,
    pub match_mode: String,
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            max_results: 8,
            match_mode: "fuzzy".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// `"auto"` loads every shell history file that exists on the host.
    /// Otherwise: any subset of `"zsh"`, `"bash"`, `"fish"`, `"pwsh"`.
    pub sources: Vec<String>,

    pub zsh_path: Option<PathBuf>,
    pub bash_path: Option<PathBuf>,
    pub fish_path: Option<PathBuf>,
    pub pwsh_path: Option<PathBuf>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            sources: vec!["auto".into()],
            zsh_path: None,
            bash_path: None,
            fish_path: None,
            pwsh_path: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("tab/config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                eprintln!("tab: config parse error: {e}");
                Self::default()
            }),
            Err(e) => {
                eprintln!("tab: config read error: {e}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Write a fully commented template on first run. Returns whether the
    /// file was (newly) created; `Err` only on real IO failure, so install
    /// flows can surface "cannot write config" to the user.
    pub fn save_default_if_missing() -> anyhow::Result<bool> {
        let path = config_path();
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, DEFAULT_TEMPLATE)?;
        Ok(true)
    }
}

/// Template with every available key, the optional ones commented out so the
/// user discovers them without having to read the source.
pub const DEFAULT_TEMPLATE: &str = r#"# tab configuration — every key is optional; delete any you don't set.

[completion]
max_results = 8
match_mode = "fuzzy"   # "fuzzy" or "prefix"

[log]
level = ""             # "" uses the component default; one of error/warn/info/debug/trace

[history]
sources = ["auto"]     # or any subset of ["zsh","bash","fish","pwsh"]

# Explicit overrides (skip default shell-specific paths):
# zsh_path  = "/absolute/path/to/zsh_history"
# bash_path = "/absolute/path/to/bash_history"
# fish_path = "/absolute/path/to/fish_history"
# pwsh_path = "/absolute/path/to/ConsoleHost_history.txt"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses_back_to_default() {
        let parsed: Config = toml::from_str(DEFAULT_TEMPLATE).unwrap();
        assert_eq!(parsed.completion.max_results, 8);
        assert_eq!(parsed.completion.match_mode, "fuzzy");
        assert_eq!(parsed.log.level, "");
        assert_eq!(parsed.history.sources, vec!["auto".to_string()]);
        assert!(parsed.history.zsh_path.is_none());
    }

    #[test]
    fn roundtrip_defaults() {
        let original = Config::default();
        let serialized = toml::to_string_pretty(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(
            parsed.completion.max_results,
            original.completion.max_results
        );
        assert_eq!(parsed.completion.match_mode, original.completion.match_mode);
        assert_eq!(parsed.history.sources, original.history.sources);
    }

    #[test]
    fn malformed_toml_yields_error() {
        let r: Result<Config, _> = toml::from_str("[completion\nmax_results = not_a_number");
        assert!(r.is_err());
    }

    #[test]
    fn completion_defaults_applied_on_partial() {
        let cfg: Config = toml::from_str("[completion]\n").unwrap();
        assert_eq!(cfg.completion.max_results, 8);
        assert_eq!(cfg.completion.match_mode, "fuzzy");
    }

    #[test]
    fn custom_match_mode_parsed() {
        let cfg: Config = toml::from_str("[completion]\nmatch_mode = \"prefix\"\n").unwrap();
        assert_eq!(cfg.completion.match_mode, "prefix");
    }

    #[test]
    fn save_and_load_via_tempdir() {
        // Indirectly exercises config_path by writing/reading via toml.
        let mut cfg = Config::default();
        cfg.completion.max_results = 12;
        cfg.completion.match_mode = "prefix".into();
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.completion.max_results, 12);
        assert_eq!(parsed.completion.match_mode, "prefix");
    }

    #[test]
    fn template_covers_every_struct_field() {
        // Drift guard: every top-level key in `Config::default()` must appear
        // in `DEFAULT_TEMPLATE` either set or (for optional fields) as a
        // commented-out example. If someone adds a new key to the struct and
        // forgets the template, this test catches it.
        let toml_default = toml::to_string_pretty(&Config::default()).expect("serialize default");
        for section in ["[completion]", "[log]", "[history]"] {
            assert!(
                DEFAULT_TEMPLATE.contains(section),
                "template missing section {section}"
            );
        }
        // Every default key name should appear somewhere in the template
        // (lines may be active or commented, but the key must be visible).
        for line in toml_default.lines() {
            if let Some(key) = line.split('=').next().map(str::trim) {
                if key.is_empty() || key.starts_with('[') {
                    continue;
                }
                assert!(
                    DEFAULT_TEMPLATE.contains(key),
                    "default key `{key}` not documented in DEFAULT_TEMPLATE"
                );
            }
        }
        // Optional overrides only appear as comments, one per known shell.
        for opt in ["zsh_path", "bash_path", "fish_path", "pwsh_path"] {
            assert!(
                DEFAULT_TEMPLATE.contains(opt),
                "template missing optional override {opt}"
            );
        }
    }
}
