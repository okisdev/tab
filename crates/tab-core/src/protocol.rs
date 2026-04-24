use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub buffer: String,
    pub cwd: String,
    #[serde(default)]
    pub match_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub text: String,
    pub score: f64,
    pub match_positions: Vec<u32>,
    pub source: CandidateSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateSource {
    History,
    Path,
    Script,
    ScriptHistory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_roundtrip() {
        let req = QueryRequest {
            buffer: "git sta".into(),
            cwd: "/home/user".into(),
            match_mode: String::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: QueryRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.buffer, "git sta");
        assert_eq!(parsed.cwd, "/home/user");
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

    #[test]
    fn response_roundtrip() {
        let resp = QueryResponse {
            candidates: vec![Candidate {
                text: "git status".into(),
                score: 0.9,
                match_positions: vec![],
                source: CandidateSource::Script,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: QueryResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.candidates[0].source, CandidateSource::Script);
    }
}
