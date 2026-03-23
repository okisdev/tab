use anyhow::Result;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

/// Run in hook/coprocess mode: bridge stdin/stdout to the daemon's Unix socket.
///
/// Communication model:
/// - "context" messages: fire-and-forget (daemon doesn't respond, overlay handles display)
/// - "navigate" messages: request-response (daemon returns updated candidates+selection)
/// - "accept" messages: request-response (daemon returns inject text)
/// - "dismiss" messages: fire-and-forget
///
/// The shell only does `read -p` after navigate/accept, avoiding buffer desync.
pub fn run_hook(_shell: &str, _session: &str) -> Result<()> {
    let socket_path = tab_core::shell_socket_path();
    let stream = connect_with_retry(&socket_path, 5)?;

    // Set a read timeout so we don't block forever
    stream.set_read_timeout(Some(std::time::Duration::from_millis(100)))?;

    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let mut stdin_line = String::new();
    let mut response_line = String::new();

    loop {
        stdin_line.clear();
        let n = stdin.lock().read_line(&mut stdin_line)?;
        if n == 0 {
            break; // EOF, shell exited
        }

        let trimmed = stdin_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Determine if this message expects a response
        let expects_response = trimmed.contains("\"navigate\"") || trimmed.contains("\"accept\"");

        // Forward to daemon
        writer.write_all(trimmed.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;

        // If we expect a response, read it and forward to stdout
        if expects_response {
            response_line.clear();
            match reader.read_line(&mut response_line) {
                Ok(n) if n > 0 => {
                    stdout.write_all(response_line.as_bytes())?;
                    stdout.flush()?;
                }
                _ => {
                    // Timeout or error — send empty line so shell doesn't hang
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                }
            }
        }
    }

    Ok(())
}

fn connect_with_retry(
    path: &std::path::Path,
    max_attempts: u32,
) -> Result<UnixStream> {
    for attempt in 1..=max_attempts {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if attempt == max_attempts {
                    anyhow::bail!("failed to connect to daemon at {path:?}: {e}");
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    unreachable!()
}
