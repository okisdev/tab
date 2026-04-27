//! Aggregate per-cwd project detection + history filters.
//!
//! Each tool lives in its own module (`scripts.rs` for pnpm/npm/yarn/bun;
//! `cargo_ctx.rs`, `go_ctx.rs`, `python_ctx.rs`, `make_ctx.rs`,
//! `compose_ctx.rs`). This file wires them into a single entry point so
//! `query.rs` doesn't grow a case per tool.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tab_core::Candidate;

use crate::{cargo_ctx, compose_ctx, go_ctx, make_ctx, python_ctx, scripts};

pub struct CwdContext {
    pub pm: scripts::Project,
    pub cargo: cargo_ctx::Project,
    pub go: go_ctx::Project,
    pub python: python_ctx::Project,
    pub make: make_ctx::Project,
    pub compose: compose_ctx::Project,
}

/// Per-cwd cache TTL. Short enough that creating a `package.json` in the
/// current directory and immediately querying picks it up; long enough that
/// a burst of keystrokes (IME commit, paste, fast typing) shares one set of
/// detector results across every request.
const CACHE_TTL: Duration = Duration::from_secs(2);
const CACHE_CAP: usize = 64;

type Cache = Mutex<HashMap<PathBuf, (Instant, Arc<CwdContext>)>>;
static CWD_CACHE: OnceLock<Cache> = OnceLock::new();

fn cache() -> &'static Cache {
    CWD_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl CwdContext {
    /// Eagerly runs every tool's detector. Each detector is a couple of
    /// `stat`/`read` calls on a small well-known file; total cost ≤ ~30 µs
    /// warm per query.
    pub fn detect(cwd: &str) -> Self {
        Self {
            pm: scripts::detect_project(cwd),
            cargo: cargo_ctx::detect(cwd),
            go: go_ctx::detect(cwd),
            python: python_ctx::detect(cwd),
            make: make_ctx::detect(cwd),
            compose: compose_ctx::detect(cwd),
        }
    }

    /// Cached variant. A burst of N requests for the same cwd within `CACHE_TTL`
    /// (e.g. the keystrokes of an IME-committed phrase) shares one detection
    /// pass instead of redoing six file probes per keystroke.
    pub fn detect_cached(cwd: &str) -> Arc<Self> {
        let key = PathBuf::from(cwd);
        let now = Instant::now();
        if let Ok(cache) = cache().lock() {
            if let Some((t, ctx)) = cache.get(&key) {
                if now.duration_since(*t) < CACHE_TTL {
                    return Arc::clone(ctx);
                }
            }
        }
        let ctx = Arc::new(Self::detect(cwd));
        if let Ok(mut cache) = cache().lock() {
            if cache.len() >= CACHE_CAP {
                cache.retain(|_, (t, _)| now.duration_since(*t) < CACHE_TTL);
            }
            cache.insert(key, (now, Arc::clone(&ctx)));
        }
        ctx
    }

    /// Candidates produced from the *buffer* (not from history): PM scripts
    /// for `pnpm run <Tab>`, etc. Currently only PM provides these; make /
    /// just / task target-completion can slot in here too.
    pub fn script_candidates(&self, buffer: &str, max: usize) -> Vec<Candidate> {
        scripts::query_scripts_with(buffer, self.pm.scripts(), max)
    }

    /// Strip history entries that cannot meaningfully run from this cwd.
    /// Order doesn't matter — each filter only acts on its own verb family.
    pub fn filter_history(&self, cands: Vec<Candidate>) -> Vec<Candidate> {
        let cands = scripts::filter_pm_history(cands, &self.pm);
        let cands = cargo_ctx::filter(cands, &self.cargo);
        let cands = go_ctx::filter(cands, &self.go);
        let cands = python_ctx::filter(cands, &self.python);
        let cands = make_ctx::filter(cands, &self.make);
        compose_ctx::filter(cands, &self.compose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tab_core::CandidateSource;

    fn c(text: &str) -> Candidate {
        Candidate {
            text: text.into(),
            score: 1.0,
            match_positions: vec![],
            source: CandidateSource::History,
        }
    }

    #[test]
    fn plain_tempdir_filters_all_project_tools_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let cands = vec![
            c("pnpm dev"),
            c("cargo build"),
            c("go test ./..."),
            c("poetry install"),
            c("make deploy"),
            c("docker compose up"),
            c("git status"), // not project-scoped — must survive
            c("ls -la"),
        ];
        let kept = ctx.filter_history(cands);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.contains(&"pnpm dev"));
        assert!(!texts.contains(&"cargo build"));
        assert!(!texts.contains(&"go test ./..."));
        assert!(!texts.contains(&"poetry install"));
        assert!(!texts.contains(&"make deploy"));
        assert!(!texts.contains(&"docker compose up"));
        assert!(texts.contains(&"git status"));
        assert!(texts.contains(&"ls -la"));
    }

    #[test]
    fn rust_workspace_keeps_cargo_filters_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let kept = ctx.filter_history(vec![
            c("cargo build"),
            c("cargo test"),
            c("pnpm dev"), // still filtered — not a Node project
        ]);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"cargo build"));
        assert!(texts.contains(&"cargo test"));
        assert!(!texts.contains(&"pnpm dev"));
    }

    #[test]
    fn detect_cached_returns_same_arc_within_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let a = CwdContext::detect_cached(path);
        let b = CwdContext::detect_cached(path);
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "second hit within TTL must return the cached Arc, not re-detect"
        );
    }

    #[test]
    fn detect_cached_invalidates_when_package_json_appears() {
        // After TTL elapses, a freshly-created `package.json` is picked up.
        // Use a longer-than-TTL sleep to avoid flakiness on slow CI.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let before = CwdContext::detect_cached(path);
        assert!(matches!(before.pm, scripts::Project::NotNode));
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        std::thread::sleep(CACHE_TTL + std::time::Duration::from_millis(100));
        let after = CwdContext::detect_cached(path);
        assert!(
            matches!(&after.pm, scripts::Project::Scripts(s) if s.iter().any(|n| n == "dev")),
            "post-TTL re-detect must observe the new package.json"
        );
    }

    #[test]
    fn mixed_repo_each_filter_answers_independently() {
        // A repo with BOTH Cargo.toml and package.json — legitimate for tools
        // like tauri / nx / cargo-leptos. Both cargo AND pnpm history survive.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let kept = ctx.filter_history(vec![c("cargo build"), c("pnpm dev"), c("pnpm build")]);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"cargo build"));
        assert!(texts.contains(&"pnpm dev")); // script declared
        assert!(!texts.contains(&"pnpm build")); // not declared
    }
}
