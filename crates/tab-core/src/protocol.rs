use serde::{Deserialize, Serialize};

// ── Shell → Daemon ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShellMessage {
    /// Shell buffer changed (sent on every keystroke)
    #[serde(rename = "context")]
    Context(ShellContext),

    /// User accepted a completion (Tab/Enter)
    #[serde(rename = "accept")]
    Accept { session_id: String, index: u32 },

    /// Navigate the candidate list (Up/Down)
    #[serde(rename = "navigate")]
    Navigate {
        session_id: String,
        direction: Direction,
    },

    /// Dismiss the popup (Escape or command executed)
    #[serde(rename = "dismiss")]
    Dismiss { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellContext {
    pub session_id: String,
    pub shell: ShellType,
    pub buffer: String,
    pub cursor_pos: u32,
    pub cwd: String,
    pub columns: u32,
    pub lines: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellType {
    Zsh,
    Bash,
    Fish,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
}

// ── Daemon → Shell ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonToShellMessage {
    /// Inject completion text into the shell buffer
    #[serde(rename = "inject")]
    Inject {
        session_id: String,
        text: String,
        replace_from: u32,
    },

    /// Candidates update with current selection
    #[serde(rename = "candidates")]
    Candidates {
        session_id: String,
        items: Vec<Candidate>,
        selected: u32,
    },
}

// ── Daemon → Overlay ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OverlayMessage {
    /// Show/update completion candidates
    #[serde(rename = "show")]
    Show {
        session_id: String,
        candidates: Vec<Candidate>,
        selected: u32,
    },

    /// Update selection index only
    #[serde(rename = "select")]
    Select { session_id: String, index: u32 },

    /// Hide the popup
    #[serde(rename = "hide")]
    Hide { session_id: String },
}

// ── Overlay → Daemon ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OverlayToDaemonMessage {
    /// User clicked a candidate
    #[serde(rename = "selected")]
    Selected { session_id: String, index: u32 },

    /// Popup was dismissed by overlay
    #[serde(rename = "dismissed")]
    Dismissed { session_id: String },
}

// ── Shared types ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub text: String,
    pub score: f64,
    /// Character indices in `text` that matched the query (for highlighting)
    pub match_positions: Vec<u32>,
    pub source: CandidateSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateSource {
    History,
    Path,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_context_roundtrip() {
        let msg = ShellMessage::Context(ShellContext {
            session_id: "test-123".into(),
            shell: ShellType::Zsh,
            buffer: "git sta".into(),
            cursor_pos: 7,
            cwd: "/home/user".into(),
            columns: 120,
            lines: 40,
        });

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ShellMessage = serde_json::from_str(&json).unwrap();

        match parsed {
            ShellMessage::Context(ctx) => {
                assert_eq!(ctx.buffer, "git sta");
                assert_eq!(ctx.cursor_pos, 7);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn candidate_serialization() {
        let c = Candidate {
            text: "git status".into(),
            score: 0.95,
            match_positions: vec![4, 5, 6],
            source: CandidateSource::History,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"history\""));
    }
}
