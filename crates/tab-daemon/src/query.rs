use std::collections::HashSet;
use std::sync::Arc;

use tab_core::{Candidate, CandidateSource, Config, QueryRequest, QueryResponse};

use crate::context::CwdContext;

const MAX_CANDIDATES: usize = 8;

pub fn handle(
    req: QueryRequest,
    history: &Arc<tab_history::HistoryIndex>,
    config: &Arc<Config>,
) -> QueryResponse {
    let match_mode = if req.match_mode.is_empty() {
        config.completion.match_mode.clone()
    } else {
        req.match_mode.clone()
    };
    let max = config.completion.max_results.clamp(1, 32);

    let ctx = CwdContext::detect_cached(&req.cwd);
    let history_candidates = history.query(&req.buffer, &req.cwd, max, &match_mode);
    let history_candidates = ctx.filter_history(history_candidates);

    // Path-command buffers (e.g. `cd `, `vim foo`) get filesystem-driven
    // completions on top of history. Previously this branch short-circuited
    // and threw away history entirely, so typing a single space after `cd`
    // would collapse the menu from a full set of recent directories down to
    // whatever happened to live in the current cwd. Merge the two so the
    // suggestion list stays useful as the user fills in a path.
    //
    // Filter history to entries that start with `<cmd><space>`. The fuzzy
    // scorer happily returns matches like `claude mcp list` for the query
    // `cd m` (c-d-m chars appear in order), but those aren't completions for
    // a `cd ...` buffer and just clutter the menu.
    if crate::paths::is_path_command(&req.buffer) {
        let path_candidates = crate::paths::query_paths(&req.buffer, &req.cwd, MAX_CANDIDATES);
        let cmd_prefix = command_prefix(&req.buffer);
        let history_for_path: Vec<Candidate> = history_candidates
            .into_iter()
            .filter(|c| c.text.starts_with(&cmd_prefix))
            .collect();
        let candidates = merge_path_history(path_candidates, history_for_path, max);
        return QueryResponse { candidates };
    }

    let script_candidates = ctx.script_candidates(&req.buffer, MAX_CANDIDATES);
    let candidates = merge(script_candidates, history_candidates, max);
    QueryResponse { candidates }
}

/// "cd m" → "cd ". Used to scope path-mode history results to the same verb
/// the user typed, so fuzzy matches across unrelated commands don't leak in.
fn command_prefix(buffer: &str) -> String {
    match buffer.split_once(' ') {
        Some((cmd, _)) => format!("{cmd} "),
        None => format!("{buffer} "),
    }
}

/// Combine concrete path completions with history hits. Path entries come
/// first (they exist on disk right now and are guaranteed to work), history
/// fills the rest up to `max` so the user keeps seeing previously-typed
/// directories that aren't necessarily under the current cwd. Source tags are
/// preserved as-is so the icon column still reflects where each entry came
/// from.
pub(crate) fn merge_path_history(
    paths: Vec<Candidate>,
    history: Vec<Candidate>,
    max: usize,
) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(max);
    for c in paths {
        if seen.insert(c.text.clone()) {
            result.push(c);
        }
        if result.len() >= max {
            return result;
        }
    }
    for c in history {
        if seen.insert(c.text.clone()) {
            result.push(c);
        }
        if result.len() >= max {
            break;
        }
    }
    result
}

pub(crate) fn merge(
    scripts: Vec<Candidate>,
    history: Vec<Candidate>,
    max: usize,
) -> Vec<Candidate> {
    let history_texts: HashSet<&str> = history.iter().map(|c| c.text.as_str()).collect();

    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(max);

    for mut c in scripts {
        if seen.insert(c.text.clone()) {
            if history_texts.contains(c.text.as_str()) {
                c.source = CandidateSource::ScriptHistory;
            }
            result.push(c);
        }
        if result.len() >= max {
            return result;
        }
    }

    for c in history {
        if seen.insert(c.text.clone()) {
            result.push(c);
        }
        if result.len() >= max {
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(text: &str, source: CandidateSource) -> Candidate {
        Candidate {
            text: text.into(),
            score: 1.0,
            match_positions: vec![],
            source,
        }
    }

    #[test]
    fn merge_prefers_scripts_but_dedups() {
        let scripts = vec![c("pnpm run dev", CandidateSource::Script)];
        let history = vec![
            c("pnpm run dev", CandidateSource::History),
            c("git status", CandidateSource::History),
        ];
        let merged = merge(scripts, history, 8);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "pnpm run dev");
        assert_eq!(merged[0].source, CandidateSource::ScriptHistory);
        assert_eq!(merged[1].text, "git status");
    }

    #[test]
    fn merge_respects_max() {
        let scripts = vec![
            c("s1", CandidateSource::Script),
            c("s2", CandidateSource::Script),
            c("s3", CandidateSource::Script),
        ];
        let history = vec![
            c("h1", CandidateSource::History),
            c("h2", CandidateSource::History),
        ];
        assert_eq!(merge(scripts, history, 3).len(), 3);
    }

    #[test]
    fn merge_empty() {
        assert!(merge(vec![], vec![], 8).is_empty());
    }

    #[test]
    fn merge_history_only_when_no_scripts() {
        let history = vec![c("ls -la", CandidateSource::History)];
        let merged = merge(vec![], history, 8);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, CandidateSource::History);
    }

    #[test]
    fn merge_path_history_path_first_then_history_fills() {
        let paths = vec![c("cd crates/", CandidateSource::Path)];
        let history = vec![
            c("cd /tmp/old-project", CandidateSource::History),
            c("cd ~/work", CandidateSource::History),
        ];
        let merged = merge_path_history(paths, history, 8);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].text, "cd crates/");
        assert_eq!(merged[0].source, CandidateSource::Path);
        assert_eq!(merged[1].text, "cd /tmp/old-project");
        assert_eq!(merged[1].source, CandidateSource::History);
    }

    #[test]
    fn merge_path_history_dedupes_against_history() {
        // If a directory exists locally AND has been cd'd to before, show it
        // once with the Path source — concrete-on-disk wins the visual slot.
        let paths = vec![c("cd crates/", CandidateSource::Path)];
        let history = vec![c("cd crates/", CandidateSource::History)];
        let merged = merge_path_history(paths, history, 8);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, CandidateSource::Path);
    }

    #[test]
    fn merge_path_history_no_paths_falls_back_to_history() {
        // Regression for the `cd <space>` collapse: when no local dirs match,
        // history must still surface so the menu doesn't go empty.
        let history = vec![
            c("cd /tmp/foo", CandidateSource::History),
            c("cd /var/log", CandidateSource::History),
        ];
        let merged = merge_path_history(vec![], history, 8);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().all(|c| c.source == CandidateSource::History));
    }

    #[test]
    fn command_prefix_extracts_verb_with_trailing_space() {
        assert_eq!(command_prefix("cd "), "cd ");
        assert_eq!(command_prefix("cd m"), "cd ");
        assert_eq!(command_prefix("cd /tmp/foo"), "cd ");
        assert_eq!(command_prefix("vim README.md"), "vim ");
    }

    #[test]
    fn merge_path_history_respects_max() {
        let paths = vec![
            c("cd a/", CandidateSource::Path),
            c("cd b/", CandidateSource::Path),
        ];
        let history = vec![
            c("cd c/", CandidateSource::History),
            c("cd d/", CandidateSource::History),
            c("cd e/", CandidateSource::History),
        ];
        assert_eq!(merge_path_history(paths, history, 3).len(), 3);
    }
}
