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
    // Path-completion branch short-circuits entirely — no project detection
    // needed, and a path buffer never matches a PM / make / compose prefix.
    if crate::paths::is_path_command(&req.buffer) {
        let path_candidates = crate::paths::query_paths(&req.buffer, &req.cwd, MAX_CANDIDATES);
        return QueryResponse {
            candidates: path_candidates,
        };
    }

    let ctx = CwdContext::detect(&req.cwd);
    let script_candidates = ctx.script_candidates(&req.buffer, MAX_CANDIDATES);

    let match_mode = if req.match_mode.is_empty() {
        config.completion.match_mode.clone()
    } else {
        req.match_mode.clone()
    };
    let max = config.completion.max_results.clamp(1, 32);

    let history_candidates = history.query(&req.buffer, &req.cwd, max, &match_mode);
    let history_candidates = ctx.filter_history(history_candidates);

    let candidates = merge(script_candidates, history_candidates, max);
    QueryResponse { candidates }
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
}
