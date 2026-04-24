use std::cell::RefCell;
use std::collections::HashMap;

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use tab_core::{Candidate, CandidateSource};

use crate::HistoryEntry;

/// Skip entries whose **byte length** exceeds this — almost always captured
/// build output rather than typed commands. Byte length (not char count) keeps
/// the cutoff independent of character set.
const MAX_COMMAND_BYTES: usize = 200;

// Scoring weights. Sum to 1.0 (`prefix_bonus` is a separate tie-breaker via
/// candidate tier, not part of the normalised score — see `query`).
const W_FUZZY: f64 = 0.45;
const W_FREQUENCY: f64 = 0.30;
const W_RECENCY: f64 = 0.25;
const RECENCY_LAMBDA: f64 = 0.05;

/// Minimum span budget for non-prefix fuzzy matches. Prevents 2-3 char queries
/// from being universally rejected against mid-length commands (e.g.
/// `dcb → docker compose build` needs span ≥ 16, but cap would otherwise be 15).
const SPARSE_SPAN_MIN: usize = 18;

/// Multiplier on `query_chars` when computing span cap. Final cap is
/// `max(q_chars × SPARSE_SPAN_FACTOR, SPARSE_SPAN_MIN)`.
const SPARSE_SPAN_FACTOR: usize = 5;

thread_local! {
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

/// Immutable snapshot of a parsed + deduplicated history. Safe to share across
/// threads via `Arc`; queries take `&self` and run lock-free.
pub struct HistoryIndex {
    commands: Vec<CommandMeta>,
    ln_max_freq: f64,
}

struct CommandMeta {
    command: String,
    command_lower: String,
    frequency: u32,
    last_used: i64,
}

impl HistoryIndex {
    pub fn from_entries(entries: &[HistoryEntry]) -> Self {
        let mut freq_map: HashMap<String, (u32, i64)> = HashMap::with_capacity(entries.len());

        for entry in entries {
            if entry.command.len() > MAX_COMMAND_BYTES {
                continue;
            }
            // Avoid the extra clone when the key already exists.
            if let Some(v) = freq_map.get_mut(&entry.command) {
                v.0 += 1;
                if entry.timestamp > v.1 {
                    v.1 = entry.timestamp;
                }
            } else {
                freq_map.insert(entry.command.clone(), (1, entry.timestamp));
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
                }
            })
            .collect();

        let max_freq = commands.iter().map(|c| c.frequency).max().unwrap_or(1) as f64;

        HistoryIndex {
            commands,
            ln_max_freq: (max_freq + 1.0).ln(),
        }
    }

    pub fn entry_count(&self) -> usize {
        self.commands.len()
    }

    /// Search the index. Takes `&self` — safe to call concurrently from
    /// multiple threads, each using its own thread-local `Matcher`.
    pub fn query(
        &self,
        query: &str,
        _cwd: &str,
        max_results: usize,
        match_mode: &str,
    ) -> Vec<Candidate> {
        if query.is_empty() {
            return self.recent_commands(max_results);
        }

        let is_prefix_mode = match_mode == "prefix";
        let query_lower = query.to_ascii_lowercase();
        let q_chars = query.chars().count().max(1);
        let sparse_cap = (q_chars * SPARSE_SPAN_FACTOR).max(SPARSE_SPAN_MIN);

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

        // Two tiers: prefix matches always outrank any fuzzy match, and
        // frequency / recency break ties within each tier.
        let mut prefix: Vec<(usize, f64, Vec<u32>)> = Vec::new();
        let mut fuzzy: Vec<(usize, f64, Vec<u32>)> = Vec::new();

        MATCHER.with(|m| {
            let matcher = &mut *m.borrow_mut();
            let mut haystack_buf = Vec::new();

            for (i, cmd) in self.commands.iter().enumerate() {
                let is_prefix =
                    cmd.command.starts_with(query) || cmd.command_lower.starts_with(&query_lower);

                if is_prefix_mode && !is_prefix {
                    continue;
                }

                // Cheap pre-filter: if the query chars don't appear in order
                // anywhere in command_lower, nucleo can't possibly match.
                // Cuts ~50-70% of the candidate list for typical queries.
                if !is_prefix && !contains_all_in_order(&cmd.command_lower, &query_lower) {
                    continue;
                }

                let haystack = Utf32Str::new(&cmd.command, &mut haystack_buf);
                let mut indices = Vec::with_capacity(q_chars);
                let Some(raw_score) = pattern.indices(haystack, matcher, &mut indices) else {
                    haystack_buf.clear();
                    continue;
                };

                if !is_prefix && is_match_too_sparse(&indices, sparse_cap) {
                    haystack_buf.clear();
                    continue;
                }

                let total = composite_score(
                    raw_score,
                    q_chars,
                    cmd.frequency,
                    cmd.last_used,
                    now,
                    self.ln_max_freq,
                );

                let entry = (i, total, indices.clone());
                if is_prefix {
                    prefix.push(entry);
                } else {
                    fuzzy.push(entry);
                }
                haystack_buf.clear();
            }
        });

        prefix.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        fuzzy.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut out: Vec<Candidate> = Vec::with_capacity(max_results);
        for (i, score, positions) in prefix.into_iter().chain(fuzzy) {
            if out.len() >= max_results {
                break;
            }
            out.push(Candidate {
                text: self.commands[i].command.clone(),
                score,
                match_positions: positions,
                source: CandidateSource::History,
            });
        }
        out
    }

    fn recent_commands(&self, max_results: usize) -> Vec<Candidate> {
        let mut indexed: Vec<(usize, i64, u32)> = self
            .commands
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.last_used, c.frequency))
            .collect();
        // Reverse-chrono primary, frequency secondary — timestamp-less entries
        // (plain bash / PSReadLine) then fall back to most-used first.
        indexed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
        indexed.truncate(max_results);

        indexed
            .into_iter()
            .map(|(i, _, _)| Candidate {
                text: self.commands[i].command.clone(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            })
            .collect()
    }
}

/// Walk `haystack` once, advancing a pointer into `needle`. Returns true iff
/// every char of `needle` appears in `haystack` in the same order. O(N) with
/// zero allocations. Used as a cheap fuzzy pre-filter.
fn contains_all_in_order(haystack_lower: &str, needle_lower: &str) -> bool {
    let mut hi = haystack_lower.chars();
    'outer: for nc in needle_lower.chars() {
        for hc in hi.by_ref() {
            if hc == nc {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

/// Reject when matched positions span more than `sparse_cap` chars — too thin
/// to be a plausible fuzzy hit. See `SPARSE_SPAN_MIN` / `SPARSE_SPAN_FACTOR`.
fn is_match_too_sparse(indices: &[u32], sparse_cap: usize) -> bool {
    if indices.len() < 2 {
        return false;
    }
    let first = *indices.first().unwrap() as usize;
    let last = *indices.last().unwrap() as usize;
    let span = last.saturating_sub(first) + 1;
    span > sparse_cap
}

/// Nucleo's raw score depends on how many chars matched (~100 per word-start,
/// +40 for consecutive). For a `q_chars`-char query the theoretical ceiling is
/// around `q_chars × 200`. Clamp the normalised score to [0, 1] so the fuzzy
/// component actually exerts its weighted share of the total.
fn fuzzy_norm(raw_score: u16, q_chars: usize) -> f64 {
    let denom = (q_chars as f64 * 200.0).max(1.0);
    ((raw_score as f64) / denom).clamp(0.0, 1.0)
}

fn composite_score(
    raw_score: u16,
    q_chars: usize,
    frequency: u32,
    last_used: i64,
    now: i64,
    ln_max_freq: f64,
) -> f64 {
    let fuzzy = fuzzy_norm(raw_score, q_chars);
    let freq = (frequency as f64 + 1.0).ln() / ln_max_freq.max(1e-9);
    let recency = if last_used > 0 {
        let days_ago = ((now - last_used) as f64) / 86400.0;
        (-RECENCY_LAMBDA * days_ago).exp()
    } else {
        0.0
    };
    fuzzy * W_FUZZY + freq * W_FREQUENCY + recency * W_RECENCY
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn deduplicates() {
        let index = HistoryIndex::from_entries(&make_entries());
        assert_eq!(index.entry_count(), 5);
    }

    #[test]
    fn fuzzy_matches() {
        let index = HistoryIndex::from_entries(&make_entries());
        let results = index.query("gst", "", 10, "fuzzy");
        assert!(!results.is_empty());
        assert!(results[0].text.contains("git"));
    }

    #[test]
    fn empty_query_returns_recent() {
        let index = HistoryIndex::from_entries(&make_entries());
        let recent = index.recent_commands(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "ls -la");
    }

    #[test]
    fn prefix_mode_filters_non_prefix() {
        let index = HistoryIndex::from_entries(&make_entries());
        let results = index.query("git", "", 10, "prefix");
        assert!(results.iter().all(|c| c.text.starts_with("git")));
    }

    #[test]
    fn no_match_returns_empty() {
        let index = HistoryIndex::from_entries(&make_entries());
        let results = index.query("zzznotexist", "", 10, "fuzzy");
        assert!(results.is_empty());
    }

    #[test]
    fn prefix_beats_non_prefix() {
        let entries = vec![
            HistoryEntry {
                command: "xg".into(),
                timestamp: 1700000000,
                duration: 0,
            },
            HistoryEntry {
                command: "g".into(),
                timestamp: 1700000000,
                duration: 0,
            },
        ];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("g", "", 10, "fuzzy");
        assert_eq!(
            results[0].text, "g",
            "prefix tier always outranks non-prefix"
        );
    }

    #[test]
    fn prefix_beats_non_prefix_even_when_frequent() {
        // High-frequency non-prefix should still lose to a rare prefix match.
        let mut entries = Vec::new();
        for _ in 0..50 {
            entries.push(HistoryEntry {
                command: "xgxxx".into(),
                timestamp: 1700000000,
                duration: 0,
            });
        }
        entries.push(HistoryEntry {
            command: "gfoo".into(),
            timestamp: 1700000000,
            duration: 0,
        });
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("g", "", 10, "fuzzy");
        assert_eq!(results[0].text, "gfoo");
    }

    #[test]
    fn within_prefix_tier_frequency_wins() {
        // Both prefix matches; the more frequent one should rank first.
        let mut entries = Vec::new();
        for _ in 0..10 {
            entries.push(HistoryEntry {
                command: "git status".into(),
                timestamp: 1700000000,
                duration: 0,
            });
        }
        entries.push(HistoryEntry {
            command: "git stash".into(),
            timestamp: 1700000000,
            duration: 0,
        });
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("git st", "", 10, "fuzzy");
        assert_eq!(results[0].text, "git status");
    }

    #[test]
    fn long_commands_are_dropped() {
        let long_cmd = "x".repeat(MAX_COMMAND_BYTES + 10);
        let entries = vec![
            HistoryEntry {
                command: long_cmd,
                timestamp: 1700000000,
                duration: 0,
            },
            HistoryEntry {
                command: "ls".into(),
                timestamp: 1700000000,
                duration: 0,
            },
        ];
        let index = HistoryIndex::from_entries(&entries);
        assert_eq!(index.entry_count(), 1);
    }

    #[test]
    fn match_positions_are_char_indices() {
        let entries = vec![HistoryEntry {
            command: "日本語テスト".into(),
            timestamp: 1700000000,
            duration: 0,
        }];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("語", "", 10, "fuzzy");
        assert!(!results.is_empty());
        // "語" is the 3rd character (index 2), not byte offset 6.
        assert_eq!(results[0].match_positions, vec![2u32]);
    }

    #[test]
    fn zero_timestamp_entries_get_neutral_recency() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let entries = vec![
            HistoryEntry {
                command: "foo_recent".into(),
                timestamp: now,
                duration: 0,
            },
            HistoryEntry {
                command: "foo_notimestamp".into(),
                timestamp: 0,
                duration: 0,
            },
        ];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("foo", "", 10, "fuzzy");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].text, "foo_recent");
        assert_eq!(results[1].text, "foo_notimestamp");
    }

    #[test]
    fn recent_commands_frequency_tiebreaker() {
        let entries = vec![
            HistoryEntry {
                command: "rare".into(),
                timestamp: 0,
                duration: 0,
            },
            HistoryEntry {
                command: "common".into(),
                timestamp: 0,
                duration: 0,
            },
            HistoryEntry {
                command: "common".into(),
                timestamp: 0,
                duration: 0,
            },
            HistoryEntry {
                command: "common".into(),
                timestamp: 0,
                duration: 0,
            },
        ];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.recent_commands(2);
        assert_eq!(results[0].text, "common");
    }

    #[test]
    fn scoring_is_deterministic() {
        let entries = make_entries();
        let idx1 = HistoryIndex::from_entries(&entries);
        let idx2 = HistoryIndex::from_entries(&entries);
        let r1 = idx1.query("git", "", 10, "fuzzy");
        let r2 = idx2.query("git", "", 10, "fuzzy");
        let t1: Vec<&str> = r1.iter().map(|c| c.text.as_str()).collect();
        let t2: Vec<&str> = r2.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(t1, t2);
    }

    #[test]
    fn empty_history_returns_empty() {
        let index = HistoryIndex::from_entries(&[]);
        assert!(index.recent_commands(10).is_empty());
        assert!(index.query("anything", "", 10, "fuzzy").is_empty());
    }

    #[test]
    fn weights_sum_to_one() {
        let sum = W_FUZZY + W_FREQUENCY + W_RECENCY;
        assert!((sum - 1.0).abs() < 1e-9, "weights = {sum}");
    }

    // ── Sparse gate ────────────────────────────────────────────────────────

    #[test]
    fn sparsity_single_char_never_sparse() {
        assert!(!is_match_too_sparse(&[0], 20));
        assert!(!is_match_too_sparse(&[50], 20));
    }

    #[test]
    fn sparsity_dense_fuzzy_accepted() {
        // "gst" → "git status" at positions 0,4,5 → span 6 ≤ cap 18.
        assert!(!is_match_too_sparse(&[0, 4, 5], 18));
    }

    #[test]
    fn sparsity_pathological_rejected() {
        // 7-char query across 90 chars → span 86 > cap 35.
        assert!(is_match_too_sparse(&[5, 12, 25, 40, 60, 75, 90], 35));
    }

    #[test]
    fn sparsity_boundary() {
        assert!(!is_match_too_sparse(&[0, 9], 10));
        assert!(is_match_too_sparse(&[0, 10], 10));
    }

    #[test]
    fn dcb_matches_docker_compose_build() {
        // Regression: with the earlier cap=q*5 only (no floor), span 16 > 15
        // rejected this legitimate 3-char abbreviation. With floor=18 it passes.
        let entries = vec![HistoryEntry {
            command: "docker compose build".into(),
            timestamp: 1700000000,
            duration: 0,
        }];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("dcb", "", 10, "fuzzy");
        assert!(!results.is_empty(), "dcb → docker compose build must match");
    }

    #[test]
    fn kgp_matches_kubectl_get_pods() {
        let entries = vec![HistoryEntry {
            command: "kubectl get pods".into(),
            timestamp: 1700000000,
            duration: 0,
        }];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("kgp", "", 10, "fuzzy");
        assert!(!results.is_empty());
    }

    #[test]
    fn openssl_does_not_match_xargs_giant_command() {
        let giant = "xargs -L 1 /Applications/Visual Studio Code.app/Contents/Resources/app/bin/code --install-extension < /tmp/vscode-extensions.txt";
        let entries = vec![HistoryEntry {
            command: giant.into(),
            timestamp: 1700000000,
            duration: 0,
        }];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("openssl", "", 10, "fuzzy");
        assert!(
            results.is_empty(),
            "sparse match should stay filtered, got {:?}",
            results.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn openssl_still_matches_real_openssl_usage() {
        let entries = vec![
            HistoryEntry {
                command: "openssl req -new -x509 -out cert.pem".into(),
                timestamp: 1700000000,
                duration: 0,
            },
            HistoryEntry {
                command: "xargs -L 1 /Applications/Visual Studio Code.app/Contents/Resources/app/bin/code --install-extension < /tmp/vscode-extensions.txt".into(),
                timestamp: 1700000000,
                duration: 0,
            },
        ];
        let index = HistoryIndex::from_entries(&entries);
        let results = index.query("openssl", "", 10, "fuzzy");
        assert_eq!(results.len(), 1);
        assert!(results[0].text.starts_with("openssl "));
    }

    // ── Cheap pre-filter ───────────────────────────────────────────────────

    #[test]
    fn order_filter_hits() {
        assert!(contains_all_in_order("git status", "gst"));
        assert!(contains_all_in_order("docker compose build", "dcb"));
        assert!(contains_all_in_order("日本語テスト", "語テ"));
    }

    #[test]
    fn order_filter_misses() {
        assert!(!contains_all_in_order("git status", "sgt")); // wrong order
        assert!(!contains_all_in_order("abc", "abcd")); // needle longer
        assert!(!contains_all_in_order("", "x")); // empty haystack
    }

    #[test]
    fn order_filter_empty_needle() {
        assert!(contains_all_in_order("anything", ""));
    }

    // ── Fuzzy norm ────────────────────────────────────────────────────────

    #[test]
    fn fuzzy_norm_clamps_to_unit() {
        assert_eq!(fuzzy_norm(0, 1), 0.0);
        let n = fuzzy_norm(u16::MAX, 1);
        assert!((0.0..=1.0).contains(&n), "got {n}");
    }

    #[test]
    fn fuzzy_norm_scales_with_query_length() {
        // Same raw score, longer query → smaller normalised score.
        let short = fuzzy_norm(200, 1);
        let long = fuzzy_norm(200, 10);
        assert!(short > long);
    }
}
