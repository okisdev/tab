use anyhow::Result;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::time::Duration;

use tab_core::{ipc, CandidateSource, QueryRequest, QueryResponse};

/// Long-lived coprocess: shell writes JSON requests on stdin, we reply with
/// `\x1f`-separated display lines on stdout.
///
/// Reconnects with exponential backoff (500 ms → 10 s) so a crash-looping
/// daemon doesn't produce a busy loop.
pub fn run_hook() -> Result<()> {
    let mut delay = Duration::from_millis(500);
    loop {
        match run_session() {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!("hook session ended: {e}; retrying in {:?}", delay);
                std::thread::sleep(delay);
                delay = next_backoff(delay);
            }
        }
    }
}

fn next_backoff(d: Duration) -> Duration {
    const CAP: Duration = Duration::from_secs(10);
    let doubled = d.saturating_mul(2);
    if doubled > CAP {
        CAP
    } else {
        doubled
    }
}

fn run_session() -> Result<()> {
    let stream = connect_with_retry(30)?;
    let mut reader = BufReader::new(stream);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let mut stdin_line = String::new();
    let mut daemon_line_buf = Vec::<u8>::new();

    loop {
        stdin_line.clear();
        let n = stdin.lock().read_line(&mut stdin_line)?;
        if n == 0 {
            return Ok(()); // shell exited
        }
        let trimmed = stdin_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req_buffer = match serde_json::from_str::<QueryRequest>(trimmed) {
            Ok(req) => req.buffer,
            Err(_) => continue,
        };

        {
            let w = reader.get_mut();
            w.write_all(trimmed.as_bytes())?;
            w.write_all(b"\n")?;
            w.flush()?;
        }

        daemon_line_buf.clear();
        let daemon_line = read_line(&mut reader, &mut daemon_line_buf);

        let output = match daemon_line {
            Some(line) => format_display(&req_buffer, line.trim()),
            None => sanitize(&req_buffer),
        };
        stdout.write_all(output.as_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
}

fn read_line<'a, R: Read>(reader: &mut BufReader<R>, buf: &'a mut Vec<u8>) -> Option<&'a str> {
    loop {
        let chunk = reader.fill_buf().ok()?;
        if chunk.is_empty() {
            return None;
        }
        if let Some(pos) = chunk.iter().position(|b| *b == b'\n') {
            buf.extend_from_slice(&chunk[..pos]);
            reader.consume(pos + 1);
            break;
        } else {
            let len = chunk.len();
            buf.extend_from_slice(chunk);
            reader.consume(len);
        }
    }
    std::str::from_utf8(buf).ok()
}

/// Build `\x1f`-separated display: first field echoes the buffer (for zsh
/// correlation), rest are `TYPE TEXT` where TYPE ∈ {H,S,B,P}.
fn format_display(req_buffer: &str, json: &str) -> String {
    let resp: QueryResponse = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(_) => return sanitize(req_buffer),
    };
    let mut parts = Vec::with_capacity(resp.candidates.len() + 1);
    parts.push(sanitize(req_buffer));
    for c in &resp.candidates {
        let t = match c.source {
            CandidateSource::History => 'H',
            CandidateSource::Script => 'S',
            CandidateSource::ScriptHistory => 'B',
            CandidateSource::Path => 'P',
        };
        parts.push(format!("{t} {}", sanitize(&c.text)));
    }
    parts.join("\x1f")
}

fn sanitize(text: &str) -> String {
    text.replace('\r', "").replace(['\n', '\t', '\x1f'], " ")
}

fn connect_with_retry(max_attempts: u32) -> Result<interprocess::local_socket::Stream> {
    for attempt in 1..=max_attempts {
        match ipc::connect_sync() {
            Ok(s) => return Ok(s),
            Err(e) => {
                if attempt == max_attempts {
                    anyhow::bail!("cannot connect to daemon: {e}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_display_prepends_buffer_echo() {
        let json = r#"{"candidates":[{"text":"cd foo/","score":0.9,"match_positions":[],"source":"history"}]}"#;
        let out = format_display("cd", json);
        let parts: Vec<&str> = out.split('\x1f').collect();
        assert_eq!(parts[0], "cd");
        assert_eq!(parts[1], "H cd foo/");
    }

    #[test]
    fn format_display_empty_candidates_still_echoes_buffer() {
        let out = format_display("cd", r#"{"candidates":[]}"#);
        assert_eq!(out, "cd");
    }

    #[test]
    fn sanitize_strips_field_separator() {
        assert_eq!(sanitize("a\x1fb"), "a b");
        assert_eq!(sanitize("a\nb"), "a b");
    }

    #[test]
    fn sanitize_drops_carriage_returns() {
        assert_eq!(sanitize("a\r\nb"), "a b");
    }

    #[test]
    fn sanitize_is_idempotent() {
        let s = "hello world";
        assert_eq!(sanitize(&sanitize(s)), sanitize(s));
    }

    #[test]
    fn format_display_all_sources_codes() {
        use tab_core::CandidateSource as S;
        for (src, expected_byte) in [
            (S::History, 'H'),
            (S::Script, 'S'),
            (S::ScriptHistory, 'B'),
            (S::Path, 'P'),
        ] {
            let resp = QueryResponse {
                candidates: vec![tab_core::Candidate {
                    text: "foo".into(),
                    score: 1.0,
                    match_positions: vec![],
                    source: src,
                }],
            };
            let json = serde_json::to_string(&resp).unwrap();
            let out = format_display("q", &json);
            let parts: Vec<&str> = out.split('\x1f').collect();
            assert_eq!(parts[1].chars().next().unwrap(), expected_byte);
        }
    }

    #[test]
    fn format_display_bad_json_echoes_buffer() {
        let out = format_display("cd", "not json");
        assert_eq!(out, "cd");
    }

    #[test]
    fn format_display_with_many_candidates() {
        let resp = QueryResponse {
            candidates: (0..5)
                .map(|i| tab_core::Candidate {
                    text: format!("cmd{i}"),
                    score: 1.0,
                    match_positions: vec![],
                    source: tab_core::CandidateSource::History,
                })
                .collect(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let out = format_display("q", &json);
        let parts: Vec<&str> = out.split('\x1f').collect();
        assert_eq!(parts.len(), 6, "1 buffer echo + 5 candidates");
    }

    #[test]
    fn backoff_grows_exponentially_with_cap() {
        let mut d = Duration::from_millis(500);
        let mut seen = vec![d];
        for _ in 0..10 {
            d = next_backoff(d);
            seen.push(d);
        }
        assert_eq!(seen[0], Duration::from_millis(500));
        assert_eq!(seen[1], Duration::from_secs(1));
        assert_eq!(seen[2], Duration::from_secs(2));
        // Eventually caps at 10s.
        assert!(seen.last().unwrap() <= &Duration::from_secs(10));
        assert_eq!(*seen.last().unwrap(), Duration::from_secs(10));
    }
}
