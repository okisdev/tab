use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::Result;
use tab_core::{Candidate, CandidateSource, QueryRequest, QueryResponse};

const MAX_VISIBLE: usize = 8;

struct State {
    input: String,
    cwd: String,
    candidates: Vec<Candidate>,
    selected: usize,
    daemon_reader: BufReader<UnixStream>,
    daemon_writer: UnixStream,
}

/// Run the interactive completion picker.
/// Opens /dev/tty directly for I/O — works inside $() substitution.
/// Returns `Some(text)` on selection, `None` on cancel.
pub fn run(buffer: &str, cwd: &str) -> Result<Option<String>> {
    let (daemon_reader, daemon_writer) = connect_to_daemon()?;

    let mut state = State {
        input: buffer.to_string(),
        cwd: cwd.to_string(),
        candidates: Vec::new(),
        selected: 0,
        daemon_reader,
        daemon_writer,
    };

    query_daemon(&mut state);

    // Open /dev/tty directly — independent of stdin/stdout/stderr
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;

    let tty_fd = tty.as_raw_fd();

    // Save terminal state and enable raw mode
    let orig_termios = termios_get(tty_fd)?;
    let mut raw = orig_termios;
    termios_make_raw(&mut raw);
    termios_set(tty_fd, &raw)?;

    // Flush pending input (the Tab key that triggered us is still in the buffer)
    unsafe { libc::tcflush(tty_fd, libc::TCIFLUSH) };

    let result = event_loop(&mut tty, &mut state);

    // Restore terminal — always
    let _ = termios_set(tty_fd, &orig_termios);
    // Clear the rendered area
    clear_display(&mut tty, &state);

    result
}

fn event_loop(tty: &mut std::fs::File, state: &mut State) -> Result<Option<String>> {
    render(tty, state);

    let mut buf = [0u8; 32];
    loop {
        let n = tty.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }

        match parse_key(&buf[..n]) {
            Some(Key::Esc) | Some(Key::CtrlC) => return Ok(None),

            Some(Key::Enter) | Some(Key::Tab) => {
                let text = if state.candidates.is_empty() {
                    state.input.clone()
                } else {
                    state.candidates[state.selected].text.clone()
                };
                return Ok(Some(text));
            }

            Some(Key::Up) | Some(Key::CtrlP) => {
                if state.selected > 0 {
                    state.selected -= 1;
                    render(tty, state);
                }
            }

            Some(Key::Down) | Some(Key::CtrlN) => {
                if !state.candidates.is_empty()
                    && state.selected < state.candidates.len() - 1
                {
                    state.selected += 1;
                    render(tty, state);
                }
            }

            Some(Key::Backspace) => {
                if state.input.pop().is_some() {
                    state.selected = 0;
                    query_daemon(state);
                    render(tty, state);
                }
            }

            Some(Key::Char(c)) => {
                state.input.push(c);
                state.selected = 0;
                query_daemon(state);
                render(tty, state);
            }

            _ => {}
        }
    }
}

// ── Rendering ──

fn render(tty: &mut std::fs::File, state: &State) {
    let n = state.candidates.len().min(MAX_VISIBLE);
    if n == 0 {
        return;
    }

    let mut out = String::new();

    // Save cursor, create scroll space, go back up
    out.push_str("\x1b[s");
    for _ in 0..n {
        out.push_str("\r\n");
    }
    out.push_str(&format!("\x1b[{}A", n));

    // Render candidates
    for i in 0..n {
        out.push_str("\r\n\x1b[2K");
        let c = &state.candidates[i];
        let icon = source_icon(c.source);

        if i == state.selected {
            out.push_str(&format!("\x1b[36;1m ▸ {icon} \x1b[0m"));
            render_highlighted_text(&mut out, &c.text, &c.match_positions, true);
        } else {
            out.push_str(&format!("\x1b[90m   {icon} \x1b[0m"));
            render_highlighted_text(&mut out, &c.text, &c.match_positions, false);
        }
    }

    // Restore cursor
    out.push_str("\x1b[u");

    let _ = tty.write_all(out.as_bytes());
    let _ = tty.flush();
}

fn render_highlighted_text(out: &mut String, text: &str, positions: &[u32], selected: bool) {
    let pos_set: HashSet<u32> = positions.iter().copied().collect();

    for (i, ch) in text.chars().enumerate() {
        if selected {
            if pos_set.contains(&(i as u32)) {
                out.push_str("\x1b[97;1m"); // bright white bold
            } else {
                out.push_str("\x1b[37m"); // white
            }
        } else if pos_set.contains(&(i as u32)) {
            out.push_str("\x1b[33m"); // yellow
        } else {
            out.push_str("\x1b[90m"); // gray
        }
        out.push(ch);
    }
    out.push_str("\x1b[0m");
}

fn clear_display(tty: &mut std::fs::File, state: &State) {
    let n = state.candidates.len().min(MAX_VISIBLE);
    if n == 0 {
        return;
    }

    let mut out = String::new();
    out.push_str("\x1b[s");
    for _ in 0..n {
        out.push_str("\r\n\x1b[2K");
    }
    out.push_str("\x1b[u");

    let _ = tty.write_all(out.as_bytes());
    let _ = tty.flush();
}

fn source_icon(source: CandidateSource) -> &'static str {
    match source {
        CandidateSource::History => "🕘",
        CandidateSource::Script => "⚡",
        CandidateSource::ScriptHistory => "⚡🕘",
        CandidateSource::Path => "📁",
    }
}

// ── Key parsing ──

enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
    Up,
    Down,
    CtrlC,
    CtrlN,
    CtrlP,
}

fn parse_key(buf: &[u8]) -> Option<Key> {
    match buf {
        [27, 91, 65] => Some(Key::Up),    // \x1b[A
        [27, 91, 66] => Some(Key::Down),  // \x1b[B
        [27, 79, 65] => Some(Key::Up),    // \x1bOA
        [27, 79, 66] => Some(Key::Down),  // \x1bOB
        [27, ..] => Some(Key::Esc),
        [13] => Some(Key::Enter),
        [9] => Some(Key::Tab),
        [127] | [8] => Some(Key::Backspace),
        [3] => Some(Key::CtrlC),
        [14] => Some(Key::CtrlN),
        [16] => Some(Key::CtrlP),
        [b] if *b >= 32 => {
            let c = *b as char;
            Some(Key::Char(c))
        }
        _ => None,
    }
}

// ── Termios helpers ──

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: libc::tcflag_t,
    c_oflag: libc::tcflag_t,
    c_cflag: libc::tcflag_t,
    c_lflag: libc::tcflag_t,
    c_cc: [libc::cc_t; 20],
    c_ispeed: libc::speed_t,
    c_ospeed: libc::speed_t,
}

fn termios_get(fd: i32) -> Result<Termios> {
    unsafe {
        let mut t: Termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t as *mut Termios as *mut libc::termios) != 0 {
            anyhow::bail!("tcgetattr failed");
        }
        Ok(t)
    }
}

fn termios_set(fd: i32, t: &Termios) -> Result<()> {
    unsafe {
        if libc::tcsetattr(fd, libc::TCSANOW, t as *const Termios as *const libc::termios) != 0 {
            anyhow::bail!("tcsetattr failed");
        }
        Ok(())
    }
}

fn termios_make_raw(t: &mut Termios) {
    unsafe {
        libc::cfmakeraw(t as *mut Termios as *mut libc::termios);
    }
    // Minimum 1 byte for read to return
    t.c_cc[libc::VMIN] = 1;
    t.c_cc[libc::VTIME] = 0;
}

// ── Daemon communication ──

fn query_daemon(state: &mut State) {
    let req = QueryRequest {
        buffer: state.input.clone(),
        cwd: state.cwd.clone(),
        match_mode: String::new(),
    };

    let Ok(json) = serde_json::to_string(&req) else { return };
    if state.daemon_writer.write_all(json.as_bytes()).is_err() { return; }
    if state.daemon_writer.write_all(b"\n").is_err() { return; }
    if state.daemon_writer.flush().is_err() { return; }

    let mut line = String::new();
    if state.daemon_reader.read_line(&mut line).is_ok() {
        if let Ok(resp) = serde_json::from_str::<QueryResponse>(&line) {
            state.candidates = resp.candidates;
            if state.selected >= state.candidates.len() {
                state.selected = state.candidates.len().saturating_sub(1);
            }
        }
    }
}

fn connect_to_daemon() -> Result<(BufReader<UnixStream>, UnixStream)> {
    let path = tab_core::shell_socket_path();

    for attempt in 1..=5 {
        match UnixStream::connect(&path) {
            Ok(stream) => {
                stream.set_read_timeout(Some(Duration::from_millis(500)))?;
                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                let writer = stream.try_clone()?;
                return Ok((BufReader::new(stream), writer));
            }
            Err(e) => {
                if attempt == 5 {
                    anyhow::bail!("cannot connect to daemon: {e}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    unreachable!()
}
