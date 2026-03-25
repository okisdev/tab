use anyhow::Result;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use tab_core::{CandidateSource, QueryResponse};

/// Run in hook/coprocess mode: bridge stdin/stdout to daemon.
///
/// Reads JSON QueryRequest from stdin, forwards to daemon, formats
/// response as display-ready text for zle -M, writes to stdout.
pub fn run_hook() -> Result<()> {
    loop {
        match run_session() {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("tab hook: {e}, reconnecting...");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn run_session() -> Result<()> {
    let socket_path = tab_core::shell_socket_path();
    let stream = connect_with_retry(&socket_path, 30)?;
    stream.set_read_timeout(Some(Duration::from_millis(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let mut daemon_writer = stream.try_clone()?;
    let mut daemon_reader = BufReader::new(stream);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let mut stdin_line = String::new();
    let mut daemon_line = String::new();

    loop {
        stdin_line.clear();
        let n = stdin.lock().read_line(&mut stdin_line)?;
        if n == 0 {
            return Ok(()); // EOF, shell exited
        }

        let trimmed = stdin_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Forward to daemon
        daemon_writer.write_all(trimmed.as_bytes())?;
        daemon_writer.write_all(b"\n")?;
        daemon_writer.flush()?;

        // Read response
        daemon_line.clear();
        match daemon_reader.read_line(&mut daemon_line) {
            Ok(n) if n > 0 => {
                let output = format_display(daemon_line.trim());
                stdout.write_all(output.as_bytes())?;
                stdout.write_all(b"\n")?;
                stdout.flush()?;
            }
            _ => {
                stdout.write_all(b"\n")?;
                stdout.flush()?;
            }
        }
    }
}

/// Format daemon response as \x1f-separated entries.
/// Each entry: "TYPE TEXT" where TYPE is H/S/B/P.
fn format_display(json: &str) -> String {
    let resp: QueryResponse = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    let mut entries = Vec::new();
    for c in &resp.candidates {
        let type_char = match c.source {
            CandidateSource::History => 'H',
            CandidateSource::Script => 'S',
            CandidateSource::ScriptHistory => 'S',
            CandidateSource::Path => 'P',
        };
        let text = sanitize(&c.text);
        entries.push(format!("{type_char} {text}"));
    }

    entries.join("\x1f")
}

fn sanitize(text: &str) -> String {
    text.replace('\n', " ").replace('\r', "").replace('\t', " ")
}

fn connect_with_retry(path: &std::path::Path, max_attempts: u32) -> Result<UnixStream> {
    for attempt in 1..=max_attempts {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if attempt == max_attempts {
                    anyhow::bail!("cannot connect to daemon at {path:?}: {e}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    unreachable!()
}
