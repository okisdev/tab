use tab_core::Candidate;

/// Per-shell-session state tracked by the daemon.
pub struct Session {
    pub selected_index: u32,
    pub last_candidates: Vec<Candidate>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            last_candidates: Vec::new(),
        }
    }

    pub fn navigate_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn navigate_down(&mut self) {
        let max = self.last_candidates.len().saturating_sub(1) as u32;
        if self.selected_index < max {
            self.selected_index += 1;
        }
    }

    pub fn update_candidates(&mut self, candidates: Vec<Candidate>) {
        self.last_candidates = candidates;
        self.selected_index = 0;
    }

    pub fn accepted_text(&self) -> Option<&str> {
        self.last_candidates
            .get(self.selected_index as usize)
            .map(|c| c.text.as_str())
    }
}
