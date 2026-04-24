use std::path::Path;

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tab_core::{Candidate, CandidateSource};

const PM_PREFIXES: &[(&str, &str)] = &[
    ("pnpm run ", "pnpm run "),
    ("pnpm ", "pnpm "),
    ("npm run ", "npm run "),
    ("yarn run ", "yarn run "),
    ("yarn ", "yarn "),
    ("bun run ", "bun run "),
    ("bun ", "bun "),
];

/// What we know about the current working directory as a Node.js project.
/// Drives whether package-manager commands in history (`pnpm dev`, etc.)
/// are worth showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Project {
    /// `package.json` is absent or unreadable — `pnpm run foo` would fail.
    NotNode,
    /// `package.json` exists but has no `scripts` section.
    NoScripts,
    /// `package.json` exists; these are the declared script names, in file order.
    Scripts(Vec<String>),
}

impl Project {
    pub fn scripts(&self) -> &[String] {
        match self {
            Project::Scripts(s) => s,
            _ => &[],
        }
    }
}

/// Detect the cwd's project state. Does a single `fs::read_to_string` + JSON
/// parse; returns `NotNode` on any IO / parse failure so that stale PM history
/// gets filtered rather than shown by default.
pub fn detect_project(cwd: &str) -> Project {
    let pkg = Path::new(cwd).join("package.json");
    let content = match std::fs::read_to_string(&pkg) {
        Ok(c) => c,
        Err(_) => return Project::NotNode,
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Project::NotNode,
    };
    let Some(scripts_obj) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Project::NoScripts;
    };
    let names: Vec<String> = scripts_obj.keys().cloned().collect();
    if names.is_empty() {
        Project::NoScripts
    } else {
        Project::Scripts(names)
    }
}

pub fn query_scripts_with(buffer: &str, scripts: &[String], max_results: usize) -> Vec<Candidate> {
    let (query, cmd_prefix) = match detect_prefix(buffer) {
        Some(v) => v,
        None => return vec![],
    };

    if scripts.is_empty() {
        return vec![];
    }

    if query.is_empty() {
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
            // nucleo returns char indices into `Utf32Str`; the TUI consumes
            // char indices too (via `text.chars().enumerate()`), so shift by
            // the char count of the prefix rather than its byte length.
            let prefix_len = cmd_prefix.chars().count() as u32;
            Candidate {
                text: format!("{cmd_prefix}{name}"),
                score,
                match_positions: positions.iter().map(|p| p + prefix_len).collect(),
                source: CandidateSource::Script,
            }
        })
        .collect()
}

fn detect_prefix(buffer: &str) -> Option<(&str, &str)> {
    for &(buf_prefix, cmd_prefix) in PM_PREFIXES {
        if let Some(rest) = buffer.strip_prefix(buf_prefix) {
            if !rest.contains(' ') {
                return Some((rest, cmd_prefix));
            }
        }
    }
    None
}

fn extract_pm_script(command: &str) -> Option<&str> {
    for &(prefix, _) in PM_PREFIXES {
        if let Some(rest) = command.strip_prefix(prefix) {
            let first_word = rest.split_whitespace().next().unwrap_or("");
            if first_word.is_empty() || first_word.starts_with('-') {
                return None;
            }
            // bun / pnpm also run a file directly (`bun index.ts`, `bun ./server.mjs`);
            // those aren't PM scripts and should pass the filter untouched.
            if looks_like_path(first_word) {
                return None;
            }
            return Some(first_word);
        }
    }
    None
}

fn looks_like_path(w: &str) -> bool {
    if w.contains('/') || w.contains('\\') || w.starts_with("./") || w.starts_with("../") {
        return true;
    }
    // common runnable source extensions — bun / node / deno targets.
    const CODE_EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"];
    CODE_EXTS.iter().any(|ext| w.ends_with(ext))
}

/// Subcommands that work regardless of whether the cwd is a Node project:
/// downloads + runs a one-shot package, scaffolds a project, prints info, etc.
/// `global` covers `yarn global add …` where "global" is itself the verb.
const PM_ANYWHERE_VERBS: &[&str] = &[
    "dlx",
    "x",
    "create",
    "init",
    "version",
    "help",
    "config",
    "cache",
    "login",
    "logout",
    "whoami",
    "doctor",
    "search",
    "view",
    "info",
    "global",
    "--version",
    "-v",
    "--help",
    "-h",
];

/// True if the command has a `-g` or `--global` token somewhere after the PM
/// prefix — `pnpm install -g typescript` succeeds anywhere.
fn is_global_install(command: &str) -> bool {
    for &(prefix, _) in PM_PREFIXES {
        if let Some(rest) = command.strip_prefix(prefix) {
            return rest
                .split_whitespace()
                .any(|t| t == "-g" || t == "--global");
        }
    }
    false
}

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

/// Filter package-manager history by what's meaningful in the current project.
///
/// - `NotNode`: drop every `pnpm/npm/yarn/bun *` — no `package.json` means
///   there's nothing for the PM to run. This is the common case people see
///   when cross-directory history is showing suggestions that can't work.
/// - `NoScripts`: keep only builtins (`pnpm install`, `yarn add`, …).
/// - `Scripts(list)`: keep builtins + script invocations whose target exists.
pub fn filter_pm_history(candidates: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|c| {
            let Some(script) = extract_pm_script(&c.text) else {
                return true; // not a PM invocation at all
            };
            // `pnpm dlx`, `bun x`, `pnpm create`, `pnpm install -g pkg` all work
            // regardless of the cwd — never filter them out.
            if PM_ANYWHERE_VERBS.contains(&script) || is_global_install(&c.text) {
                return true;
            }
            match project {
                Project::NotNode => false,
                Project::NoScripts => PM_BUILTINS.contains(&script),
                Project::Scripts(scripts) => {
                    PM_BUILTINS.contains(&script) || scripts.iter().any(|s| s == script)
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pm_prefix() {
        assert_eq!(detect_prefix("pnpm run dev"), Some(("dev", "pnpm run ")));
        assert_eq!(detect_prefix("npm run "), Some(("", "npm run ")));
        assert_eq!(detect_prefix("bun build"), Some(("build", "bun ")));
        assert_eq!(detect_prefix("git status"), None);
    }

    #[test]
    fn detects_all_pm_flavors() {
        for (buf, expected_prefix) in [
            ("pnpm run dev", "pnpm run "),
            ("pnpm build", "pnpm "),
            ("npm run test", "npm run "),
            ("yarn run lint", "yarn run "),
            ("yarn build", "yarn "),
            ("bun run ci", "bun run "),
            ("bun test", "bun "),
        ] {
            let got = detect_prefix(buf).expect(buf);
            assert_eq!(got.1, expected_prefix, "for {buf}");
        }
    }

    #[test]
    fn detect_prefix_rejects_space_in_query() {
        assert!(detect_prefix("pnpm run dev --watch").is_none());
    }

    #[test]
    fn extract_pm_script_from_text() {
        assert_eq!(extract_pm_script("pnpm run dev --watch"), Some("dev"));
        assert_eq!(extract_pm_script("yarn build"), Some("build"));
        assert_eq!(extract_pm_script("git status"), None);
        assert_eq!(extract_pm_script("pnpm --silent"), None);
    }

    #[test]
    fn bun_runtime_invocations_are_not_scripts() {
        // Regression: `bun index.ts` used to parse `index.ts` as a script
        // name, then filter it out everywhere.
        assert_eq!(extract_pm_script("bun index.ts"), None);
        assert_eq!(extract_pm_script("bun ./server.mjs"), None);
        assert_eq!(extract_pm_script("bun src/main.ts"), None);
        assert_eq!(extract_pm_script("bun ../shared/lib.js"), None);
    }

    #[test]
    fn windows_style_paths_also_pass() {
        assert_eq!(extract_pm_script("bun src\\main.ts"), None);
    }

    #[test]
    fn dash_flag_first_word_is_not_script() {
        assert_eq!(extract_pm_script("pnpm -C pkg run build"), None);
        assert_eq!(extract_pm_script("pnpm --silent install"), None);
    }

    #[test]
    fn is_global_install_detects_flag_tokens() {
        assert!(is_global_install("pnpm install -g typescript"));
        assert!(is_global_install("pnpm add --global prettier"));
        assert!(!is_global_install("pnpm install typescript"));
        assert!(!is_global_install("cargo install -g --foo")); // not a PM prefix
                                                               // `yarn global add` uses `global` as a verb (no -g flag); that path is
                                                               // covered by PM_ANYWHERE_VERBS, not this helper.
        assert!(!is_global_install("yarn global add eslint"));
    }

    #[test]
    fn empty_query_returns_all_scripts() {
        let scripts = vec!["dev".into(), "build".into(), "test".into()];
        let cands = query_scripts_with("pnpm run ", &scripts, 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"pnpm run dev"));
        assert!(texts.contains(&"pnpm run build"));
        assert!(texts.contains(&"pnpm run test"));
    }

    #[test]
    fn fuzzy_query_ranks_matches() {
        let scripts = vec!["dev".into(), "deploy".into(), "docs".into()];
        let cands = query_scripts_with("pnpm run dv", &scripts, 10);
        assert!(!cands.is_empty());
        assert_eq!(cands[0].text, "pnpm run dev");
    }

    #[test]
    fn empty_scripts_returns_empty() {
        let cands = query_scripts_with("pnpm run ", &[], 10);
        assert!(cands.is_empty());
    }

    #[test]
    fn non_pm_buffer_returns_empty() {
        let scripts = vec!["dev".into()];
        assert!(query_scripts_with("git status", &scripts, 10).is_empty());
    }

    #[test]
    fn match_positions_shifted_by_prefix_char_count() {
        let scripts = vec!["build".into()];
        let cands = query_scripts_with("pnpm run bld", &scripts, 10);
        assert_eq!(cands.len(), 1);
        let cand = &cands[0];
        assert!(cand.match_positions.iter().all(|p| *p >= 9));
        assert!(
            cand.match_positions.contains(&9),
            "got {:?}",
            cand.match_positions
        );
    }

    // ── detect_project ────────────────────────────────────────────────────

    #[test]
    fn detect_project_missing_is_not_node() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            detect_project(dir.path().to_str().unwrap()),
            Project::NotNode
        );
    }

    #[test]
    fn detect_project_malformed_is_not_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{ not json").unwrap();
        // Malformed package.json isn't a usable Node project from tab's pov.
        assert_eq!(
            detect_project(dir.path().to_str().unwrap()),
            Project::NotNode
        );
    }

    #[test]
    fn detect_project_no_scripts_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        assert_eq!(
            detect_project(dir.path().to_str().unwrap()),
            Project::NoScripts
        );
    }

    #[test]
    fn detect_project_empty_scripts_object() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"scripts":{}}"#).unwrap();
        assert_eq!(
            detect_project(dir.path().to_str().unwrap()),
            Project::NoScripts
        );
    }

    #[test]
    fn detect_project_with_scripts_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"z":"","a":"","m":""}}"#,
        )
        .unwrap();
        let project = detect_project(dir.path().to_str().unwrap());
        match project {
            Project::Scripts(s) => assert_eq!(s, vec!["z", "a", "m"]),
            other => panic!("expected Scripts, got {other:?}"),
        }
    }

    // ── filter_pm_history ────────────────────────────────────────────────

    fn pm_cands() -> Vec<Candidate> {
        let texts = [
            "pnpm dev",         // script invocation
            "pnpm run build",   // script invocation (explicit run)
            "pnpm install",     // builtin
            "yarn test",        // builtin (aliased to `npm test`)
            "bun add react",    // builtin
            "npm run docs:dev", // script invocation
            "cd /tmp",          // not a PM command — must survive
            "git status",       // not a PM command
        ];
        texts
            .into_iter()
            .map(|t| Candidate {
                text: t.into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            })
            .collect()
    }

    #[test]
    fn not_node_drops_project_scoped_pm_commands() {
        // `pnpm dev` / `pnpm run build` / `yarn test` need a package.json.
        // They should NOT appear in a Rust / plain dir.
        let survivors = filter_pm_history(pm_cands(), &Project::NotNode);
        let texts: Vec<&str> = survivors.iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.contains(&"pnpm dev"));
        assert!(!texts.contains(&"pnpm run build"));
        assert!(!texts.contains(&"pnpm install")); // local install needs package.json too
        assert!(!texts.contains(&"yarn test")); // same
        assert!(texts.contains(&"cd /tmp"));
        assert!(texts.contains(&"git status"));
    }

    #[test]
    fn not_node_keeps_anywhere_pm_commands() {
        // Regression: `pnpm dlx foo` / `bun x bar` / `pnpm create next-app`
        // / `pnpm install -g typescript` don't need a local package.json and
        // must survive the NotNode filter.
        let anywhere = vec![
            "pnpm dlx create-next-app",
            "yarn dlx prettier",
            "bun x tsx script.ts",
            "pnpm create vite",
            "pnpm install -g typescript",
            "pnpm add --global eslint",
            "pnpm --version",
            "bun --help",
            "pnpm search react",
        ];
        let cands: Vec<Candidate> = anywhere
            .iter()
            .map(|t| Candidate {
                text: (*t).into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            })
            .collect();
        let survivors = filter_pm_history(cands, &Project::NotNode);
        let got: Vec<&str> = survivors.iter().map(|c| c.text.as_str()).collect();
        for expected in &anywhere {
            assert!(got.contains(expected), "dropped: {expected}");
        }
    }

    #[test]
    fn bun_runtime_file_survives_every_filter() {
        let runtime = vec!["bun index.ts", "bun ./server.mjs"];
        let cands: Vec<Candidate> = runtime
            .iter()
            .map(|t| Candidate {
                text: (*t).into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            })
            .collect();
        for project in [
            Project::NotNode,
            Project::NoScripts,
            Project::Scripts(vec!["other".into()]),
        ] {
            let got = filter_pm_history(cands.clone(), &project);
            assert_eq!(got.len(), 2, "dropped under {project:?}");
        }
    }

    #[test]
    fn no_scripts_keeps_only_builtins() {
        // package.json exists but `scripts: {}`. `pnpm install` / `yarn test`
        // still work (manipulate deps). `pnpm dev` would fail.
        let survivors = filter_pm_history(pm_cands(), &Project::NoScripts);
        let texts: Vec<&str> = survivors.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"pnpm install"));
        assert!(texts.contains(&"yarn test"));
        assert!(texts.contains(&"bun add react"));
        assert!(texts.contains(&"cd /tmp"));
        assert!(texts.contains(&"git status"));
        assert!(!texts.contains(&"pnpm dev"));
        assert!(!texts.contains(&"pnpm run build"));
        assert!(!texts.contains(&"npm run docs:dev"));
    }

    #[test]
    fn scripts_keeps_builtins_plus_known_scripts() {
        let project = Project::Scripts(vec!["dev".into(), "build".into()]);
        let survivors = filter_pm_history(pm_cands(), &project);
        let texts: Vec<&str> = survivors.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"pnpm dev")); // known script
        assert!(texts.contains(&"pnpm run build")); // known script, explicit run
        assert!(texts.contains(&"pnpm install")); // builtin
        assert!(texts.contains(&"yarn test")); // builtin
        assert!(!texts.contains(&"npm run docs:dev")); // unknown script
    }

    #[test]
    fn filter_preserves_order_within_kept() {
        let cands = vec![
            Candidate {
                text: "pnpm dev".into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            },
            Candidate {
                text: "cd foo".into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            },
            Candidate {
                text: "pnpm install".into(),
                score: 1.0,
                match_positions: vec![],
                source: CandidateSource::History,
            },
        ];
        let out = filter_pm_history(cands, &Project::NotNode);
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["cd foo"]);
    }
}
