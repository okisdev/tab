use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub completion: CompletionConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// Log level override: "trace", "debug", "info", "warn", "error".
    /// Empty string means use component default.
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompletionConfig {
    pub max_results: usize,
    /// "fuzzy" (default) or "prefix" (only show commands starting with input)
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

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("tab/config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => eprintln!("tab: config parse error: {e}"),
                },
                Err(e) => eprintln!("tab: config read error: {e}"),
            }
        }
        Config::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn save_default_if_missing() {
        let path = config_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let config = Config::default();
            if let Ok(content) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(&path, content);
            }
        }
    }
}
