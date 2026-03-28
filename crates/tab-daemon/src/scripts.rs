use std::path::Path;

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tab_core::{Candidate, CandidateSource};

/// Recognized package manager prefixes and how to form the full command.
/// (buffer_prefix, command_prefix_for_candidate)
const PM_PREFIXES: &[(&str, &str)] = &[
    ("pnpm run ", "pnpm run "),
    ("pnpm ", "pnpm "),
    ("npm run ", "npm run "),
    ("yarn run ", "yarn run "),
    ("yarn ", "yarn "),
    ("bun run ", "bun run "),
    ("bun ", "bun "),
];

/// If the buffer looks like a package-manager invocation, return script
/// candidates using pre-read scripts list.
pub fn query_scripts_with(buffer: &str, scripts: &[String], max_results: usize) -> Vec<Candidate> {
    // Find which prefix matches
    let (query, cmd_prefix) = match detect_prefix(buffer) {
        Some(v) => v,
        None => return vec![],
    };

    if scripts.is_empty() {
        return vec![];
    }

    if query.is_empty() {
        // No query yet — return all scripts (up to max)
        return scripts
            .iter()
            .take(max_results)
            .map(|name| Candidate {
                text: format!("{cmd_prefix}{name}"),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::Script,
            })
            .collect();
    }

    // Fuzzy match
    let pattern = Atom::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(String, f64, Vec<u32>)> = Vec::new();

    for name in scripts {
        let haystack = Utf32Str::new(name, &mut buf);
        let mut indices = Vec::new();
        if let Some(score) = pattern.indices(haystack, &mut matcher, &mut indices) {
            scored.push((name.to_string(), score as f64, indices));
        }
        buf.clear();
    }

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_results);

    scored
        .into_iter()
        .map(|(name, score, positions)| {
            let prefix_len = cmd_prefix.len() as u32;
            Candidate {
                text: format!("{cmd_prefix}{name}"),
                score,
                // Shift match positions by prefix length so highlighting is correct
                match_positions: positions.iter().map(|p| p + prefix_len).collect(),
                source: CandidateSource::Script,
            }
        })
        .collect()
}

/// Detect if `buffer` starts with a known package manager prefix.
/// Returns `(script_query, command_prefix)`.
fn detect_prefix(buffer: &str) -> Option<(&str, &str)> {
    // Try longer prefixes first (e.g. "pnpm run " before "pnpm ")
    for &(buf_prefix, cmd_prefix) in PM_PREFIXES {
        if let Some(rest) = buffer.strip_prefix(buf_prefix) {
            // Only match if the rest doesn't contain spaces (we're completing the script name)
            if !rest.contains(' ') {
                return Some((rest, cmd_prefix));
            }
        }
    }
    None
}

/// Extract the script name from a completed PM command (e.g. "pnpm run dev --watch" → "dev").
/// Returns None if the command doesn't look like a PM script invocation.
fn extract_pm_script(command: &str) -> Option<&str> {
    for &(prefix, _) in PM_PREFIXES {
        if let Some(rest) = command.strip_prefix(prefix) {
            let first_word = rest.split_whitespace().next().unwrap_or("");
            if !first_word.is_empty() && !first_word.starts_with('-') {
                return Some(first_word);
            }
        }
    }
    None
}

/// Common PM built-in subcommands that are NOT script names.
const PM_BUILTINS: &[&str] = &[
    "install",
    "i",
    "ci",
    "add",
    "remove",
    "rm",
    "uninstall",
    "update",
    "up",
    "list",
    "ls",
    "outdated",
    "audit",
    "exec",
    "dlx",
    "create",
    "init",
    "publish",
    "pack",
    "link",
    "unlink",
    "why",
    "prune",
    "rebuild",
    "config",
    "login",
    "logout",
    "whoami",
    "cache",
    "doctor",
    "dedupe",
    "help",
    "version",
    "test",
    "t",
    "start",
    "stop",
    "restart",
    "info",
    "view",
    "search",
    "bin",
    "root",
    "prefix",
    "store",
    "setup",
    "import",
    "patch",
    "deploy",
];

/// Filter out history candidates that are PM script invocations for scripts
/// not present in the pre-read scripts list.
pub fn filter_irrelevant_pm_commands_with(
    candidates: Vec<Candidate>,
    scripts: &[String],
) -> Vec<Candidate> {
    // If no scripts available, skip filtering (no package.json or no scripts section)
    if scripts.is_empty() {
        return candidates;
    }


    candidates
        .into_iter()
        .filter(|c| match extract_pm_script(&c.text) {
            Some(name) if PM_BUILTINS.contains(&name) => true,
            Some(name) => scripts.iter().any(|s| s == name),
            None => true,
        })
        .collect()
}

/// Read script names from package.json in the given directory.
pub fn read_scripts(cwd: &str) -> Vec<String> {
    let pkg_path = Path::new(cwd).join("package.json");
    let content = match std::fs::read_to_string(&pkg_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return vec![];
    };

    scripts.keys().cloned().collect()
}
