use crate::HistoryEntry;
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::collections::HashMap;
use tab_core::Candidate;
use tab_core::CandidateSource;

/// In-memory history index with fuzzy matching.
pub struct HistoryIndex {
    /// Deduplicated commands with metadata
    commands: Vec<CommandMeta>,
    /// Reusable matcher
    matcher: Matcher,
    /// Pre-computed ln(max_freq + 1) for scoring (avoids O(N) scan per query)
    ln_max_freq: f64,
}

struct CommandMeta {
    command: String,
    /// Pre-computed lowercase for prefix matching (avoids per-query allocation)
    command_lower: String,
    frequency: u32,
    last_used: i64,
    /// Directories this command was run in (future use)
    _directories: Vec<String>,
}

/// Skip history entries longer than this — they're almost certainly
/// build logs or other non-command output captured into history.
const MAX_COMMAND_LEN: usize = 200;

// Scoring weights
const W_FUZZY: f64 = 0.40;
const W_FREQUENCY: f64 = 0.25;
const W_RECENCY: f64 = 0.20;
const W_CONTEXT: f64 = 0.15;

// Recency decay: half-life of ~14 days
const RECENCY_LAMBDA: f64 = 0.05;

impl HistoryIndex {
    /// Build an index from parsed history entries.
    pub fn from_entries(entries: &[HistoryEntry]) -> Self {
        let mut freq_map: HashMap<String, (u32, i64)> = HashMap::new();

        for entry in entries {
            // Skip absurdly long entries (build logs, etc.)
            if entry.command.len() > MAX_COMMAND_LEN {
                continue;
            }
            let e = freq_map.entry(entry.command.clone()).or_insert((0, 0));
            e.0 += 1;
            if entry.timestamp > e.1 {
                e.1 = entry.timestamp;
            }
        }

        let commands: Vec<CommandMeta> = freq_map
            .into_iter()
            .map(|(command, (frequency, last_used))| {
                let command_lower = command.to_ascii_lowercase();
                CommandMeta {
                    command,
                    command_lower,
                    frequency,
                    last_used,
                    _directories: Vec::new(),
                }
            })
            .collect();

        let max_freq = commands.iter().map(|c| c.frequency).max().unwrap_or(1) as f64;

        HistoryIndex {
            commands,
            matcher: Matcher::new(Config::DEFAULT),
            ln_max_freq: (max_freq + 1.0).ln(),
        }
    }

    /// Query the index and return ranked candidates.
    /// `match_mode`: "prefix" = only prefix matches, "fuzzy" (default) = fuzzy + prefix bonus.
    pub fn query(
        &mut self,
        query: &str,
        cwd: &str,
        max_results: usize,
        match_mode: &str,
    ) -> Vec<Candidate> {
        if query.is_empty() {
            return self.recent_commands(max_results);
        }

        let is_prefix_mode = match_mode == "prefix";

        let pattern = Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let query_lower = query.to_ascii_lowercase();
        let mut scored: Vec<(usize, f64, Vec<u32>)> = Vec::new();
        let mut buf = Vec::new();

        for (i, cmd) in self.commands.iter().enumerate() {
            let is_prefix = cmd.command.starts_with(query)
                || cmd.command_lower.starts_with(&query_lower);

            // In prefix mode, skip non-prefix matches entirely
            if is_prefix_mode && !is_prefix {
                continue;
            }

            let haystack = Utf32Str::new(&cmd.command, &mut buf);
            let mut indices = Vec::new();
            if let Some(fuzzy_score) = pattern.indices(haystack, &mut self.matcher, &mut indices) {
                let fuzzy_norm = (fuzzy_score as f64) / (cmd.command.len() as f64 * 100.0 + 1.0);

                let freq_score = (cmd.frequency as f64 + 1.0).ln() / self.ln_max_freq;

                let days_ago = ((now - cmd.last_used) as f64) / 86400.0;
                let recency_score = (-RECENCY_LAMBDA * days_ago).exp();

                let _ = cwd;
                let context_score = 0.0_f64;

                let prefix_bonus = if is_prefix { 0.5 } else { 0.0 };

                let total = fuzzy_norm * W_FUZZY
                    + freq_score * W_FREQUENCY
                    + recency_score * W_RECENCY
                    + context_score * W_CONTEXT
                    + prefix_bonus;

                let match_positions: Vec<u32> = indices.to_vec();
                scored.push((i, total, match_positions));
            }
            buf.clear();
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_results);

        scored
            .into_iter()
            .map(|(i, score, positions)| Candidate {
                text: self.commands[i].command.clone(),
                score,
                match_positions: positions,
                source: CandidateSource::History,
            })
            .collect()
    }

    /// Return the most recently used commands (when query is empty).
    fn recent_commands(&self, max_results: usize) -> Vec<Candidate> {
        let mut indexed: Vec<(usize, i64)> = self
            .commands
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.last_used))
            .collect();
        indexed.sort_by(|a, b| b.1.cmp(&a.1));
        indexed.truncate(max_results);

        indexed
            .into_iter()
            .map(|(i, _)| Candidate {
                text: self.commands[i].command.clone(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.commands.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HistoryEntry;

    fn make_entries() -> Vec<HistoryEntry> {
        vec![
            HistoryEntry {
                command: "git status".into(),
                timestamp: 1700000000,
                duration: 0,
            },
            HistoryEntry {
                command: "git commit -m 'fix'".into(),
                timestamp: 1700000100,
                duration: 0,
            },
            HistoryEntry {
                command: "cargo build".into(),
                timestamp: 1700000200,
                duration: 1,
            },
            HistoryEntry {
                command: "cargo test".into(),
                timestamp: 1700000300,
                duration: 2,
            },
            HistoryEntry {
                command: "git status".into(),
                timestamp: 1700000400,
                duration: 0,
            },
            HistoryEntry {
                command: "ls -la".into(),
                timestamp: 1700000500,
                duration: 0,
            },
        ]
    }

    #[test]
    fn index_deduplicates() {
        let entries = make_entries();
        let index = HistoryIndex::from_entries(&entries);
        // "git status" appears twice but should be deduplicated
        assert_eq!(index.entry_count(), 5);
    }

    #[test]
    fn fuzzy_match_git() {
        let entries = make_entries();
        let mut index = HistoryIndex::from_entries(&entries);
        let results = index.query("gst", "", 10, "fuzzy");
        assert!(!results.is_empty());
        // "git status" should rank high for "gst"
        assert!(results[0].text.contains("git"));
    }

    #[test]
    fn empty_query_returns_recent() {
        let entries = make_entries();
        let mut index = HistoryIndex::from_entries(&entries);
        let results = index.query("", "", 3, "fuzzy");
        assert_eq!(results.len(), 3);
        // Most recent should be first
        assert_eq!(results[0].text, "ls -la");
    }

    #[test]
    fn no_match_returns_empty() {
        let entries = make_entries();
        let mut index = HistoryIndex::from_entries(&entries);
        let results = index.query("zzzznotexist", "", 10, "fuzzy");
        assert!(results.is_empty());
    }
}
