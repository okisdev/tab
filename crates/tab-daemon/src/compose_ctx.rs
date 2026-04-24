//! Filter `docker compose` / `docker-compose` history by compose-file presence.

use std::path::Path;

use tab_core::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Project {
    NoCompose,
    Compose,
}

const COMPOSE_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
    "compose.override.yaml",
    "compose.override.yml",
    "docker-compose.override.yaml",
    "docker-compose.override.yml",
];

pub fn detect(cwd: &str) -> Project {
    let dir = Path::new(cwd);
    if COMPOSE_FILES.iter().any(|f| dir.join(f).is_file()) {
        Project::Compose
    } else {
        Project::NoCompose
    }
}

/// Subcommands that work without a compose file in cwd.
const COMPOSE_ANYWHERE: &[&str] = &[
    "version",
    "--version",
    "--help",
    "-h",
    "help",
    "config", // `docker compose config` with -f path is fine; allow bare
];

fn compose_verb(cmd: &str) -> Option<&str> {
    let rest = cmd
        .strip_prefix("docker compose ")
        .or_else(|| cmd.strip_prefix("docker-compose "))?;
    rest.split_whitespace().next()
}

/// `-f`/`--file` only means "compose file" when it appears BEFORE the verb
/// (compose's global flags live there). After the verb, `-f` is whatever
/// flag that subcommand defines — e.g. `docker compose logs -f` is follow.
fn has_explicit_file_flag(cmd: &str) -> bool {
    let Some(rest) = cmd
        .strip_prefix("docker compose ")
        .or_else(|| cmd.strip_prefix("docker-compose "))
    else {
        return false;
    };
    for tok in rest.split_whitespace() {
        if !tok.starts_with('-') {
            return false; // reached the verb; stop scanning
        }
        if tok == "-f" || tok == "--file" || tok.starts_with("--file=") {
            return true;
        }
    }
    false
}

pub fn filter(cands: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    cands
        .into_iter()
        .filter(|c| {
            let Some(verb) = compose_verb(&c.text) else {
                return true;
            };
            if matches!(project, Project::Compose) {
                return true;
            }
            if verb.starts_with('-') || COMPOSE_ANYWHERE.contains(&verb) {
                return true;
            }
            has_explicit_file_flag(&c.text)
        })
        .collect()
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
    fn detect_via_any_compose_file() {
        for name in COMPOSE_FILES {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(name), "services:\n").unwrap();
            assert_eq!(
                detect(dir.path().to_str().unwrap()),
                Project::Compose,
                "for {name}"
            );
        }
    }

    #[test]
    fn detect_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NoCompose);
    }

    #[test]
    fn verb_extraction() {
        assert_eq!(compose_verb("docker compose up -d"), Some("up"));
        assert_eq!(compose_verb("docker-compose logs -f"), Some("logs"));
        assert_eq!(compose_verb("docker ps"), None);
        assert_eq!(compose_verb("docker run foo"), None);
    }

    #[test]
    fn outside_compose_drops_project_subcommands() {
        let cands = vec![
            c("docker compose up -d"),
            c("docker compose down"),
            c("docker compose logs -f worker"),
            c("docker compose restart api"),
            c("docker compose build"),
            c("docker compose exec worker sh"),
            c("docker compose version"),
            c("docker compose --help"),
            c("docker compose config"),
            c("docker compose -f prod.yaml up"),
            c("docker-compose up"),
            c("docker ps"),
            c("docker run -it ubuntu bash"),
        ];
        let kept = filter(cands, &Project::NoCompose);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        // dropped (need compose file)
        assert!(!texts.contains(&"docker compose up -d"));
        assert!(!texts.contains(&"docker compose down"));
        assert!(!texts.contains(&"docker compose logs -f worker"));
        assert!(!texts.contains(&"docker compose restart api"));
        assert!(!texts.contains(&"docker compose build"));
        assert!(!texts.contains(&"docker compose exec worker sh"));
        assert!(!texts.contains(&"docker-compose up"));
        // kept (anywhere or explicit -f)
        assert!(texts.contains(&"docker compose version"));
        assert!(texts.contains(&"docker compose --help"));
        assert!(texts.contains(&"docker compose config"));
        assert!(texts.contains(&"docker compose -f prod.yaml up"));
        // untouched non-compose docker commands
        assert!(texts.contains(&"docker ps"));
        assert!(texts.contains(&"docker run -it ubuntu bash"));
    }

    #[test]
    fn inside_compose_keeps_all() {
        let cands = vec![
            c("docker compose up"),
            c("docker compose logs"),
            c("docker compose restart"),
        ];
        let kept = filter(cands, &Project::Compose);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn file_flag_is_positional() {
        // Before verb → compose file (keep outside compose)
        assert!(has_explicit_file_flag("docker compose -f prod.yaml up"));
        assert!(has_explicit_file_flag("docker compose --file=x.yaml up"));
        assert!(has_explicit_file_flag(
            "docker compose --file prod.yaml up -d"
        ));
        // After verb → that verb's -f (follow, force, etc.) — NOT compose file
        assert!(!has_explicit_file_flag("docker compose logs -f worker"));
        assert!(!has_explicit_file_flag("docker compose rm -f"));
        assert!(!has_explicit_file_flag("docker compose up"));
    }
}
