use std::io::{self, BufRead, BufReader, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType},
};

use tab_core::{ipc, Candidate, CandidateSource, QueryRequest, QueryResponse};

use crate::term::{reserve_lines, TerminalGuard};

const MAX_VISIBLE: usize = 8;
const DAEMON_TIMEOUT: Duration = Duration::from_millis(500);

struct State {
    input: String,
    cwd: String,
    candidates: Vec<Candidate>,
    selected: usize,
    rendered_lines: u16,
}

/// Bridge a sync UI to the daemon over a background thread. Provides a
/// bounded `query` so the UI never freezes if the daemon stalls.
struct DaemonBridge {
    req_tx: mpsc::Sender<QueryRequest>,
    resp_rx: mpsc::Receiver<QueryResponse>,
}

impl DaemonBridge {
    fn spawn() -> Result<Self> {
        let stream = ipc::connect_sync()?;
        let (req_tx, req_rx) = mpsc::channel::<QueryRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<QueryResponse>();

        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            while let Ok(req) = req_rx.recv() {
                let Ok(json) = serde_json::to_string(&req) else {
                    continue;
                };
                {
                    let w = reader.get_mut();
                    if w.write_all(json.as_bytes()).is_err()
                        || w.write_all(b"\n").is_err()
                        || w.flush().is_err()
                    {
                        break;
                    }
                }
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    break;
                }
                if let Ok(resp) = serde_json::from_str::<QueryResponse>(&line) {
                    if resp_tx.send(resp).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Self { req_tx, resp_rx })
    }

    fn query(&self, req: QueryRequest) -> Option<QueryResponse> {
        // Drain stale responses (user typed faster than daemon replied).
        while self.resp_rx.try_recv().is_ok() {}
        self.req_tx.send(req).ok()?;
        self.resp_rx.recv_timeout(DAEMON_TIMEOUT).ok()
    }
}

/// Render the picker to stderr and print the selected text on stdout at exit.
/// Stderr is used so `$(...)` substitution captures only the final selection.
pub fn run(buffer: &str, cwd: &str) -> Result<Option<String>> {
    let bridge = DaemonBridge::spawn()?;

    let mut state = State {
        input: buffer.to_string(),
        cwd: cwd.to_string(),
        candidates: Vec::new(),
        selected: 0,
        rendered_lines: 0,
    };

    query_daemon(&bridge, &mut state);

    let mut out = io::stderr();
    let _guard = TerminalGuard::enter()?;

    let result = event_loop(&mut out, &bridge, &mut state);
    clear_display(&mut out, &mut state).ok();
    result
}

fn event_loop<W: Write>(
    out: &mut W,
    bridge: &DaemonBridge,
    state: &mut State,
) -> Result<Option<String>> {
    render(out, state)?;
    loop {
        let ev = match event::read() {
            Ok(e) => e,
            Err(e) => anyhow::bail!("event read failed: {e}"),
        };
        let key = match ev {
            Event::Key(k) => k,
            // Re-render on resize so the candidate list doesn't garble.
            Event::Resize(_, _) => {
                state.rendered_lines = 0;
                render(out, state)?;
                continue;
            }
            _ => continue,
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(None),
            (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                let text = if state.candidates.is_empty() {
                    state.input.clone()
                } else {
                    state.candidates[state.selected].text.clone()
                };
                return Ok(Some(text));
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                if state.selected > 0 =>
            {
                state.selected -= 1;
                render(out, state)?;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL)
                if !state.candidates.is_empty() && state.selected + 1 < state.candidates.len() =>
            {
                state.selected += 1;
                render(out, state)?;
            }
            (KeyCode::Backspace, _) if state.input.pop().is_some() => {
                state.selected = 0;
                query_daemon(bridge, state);
                render(out, state)?;
            }
            (KeyCode::Char(c), m) if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                state.input.push(c);
                state.selected = 0;
                query_daemon(bridge, state);
                render(out, state)?;
            }
            _ => {}
        }
    }
}

fn render<W: Write>(out: &mut W, state: &mut State) -> Result<()> {
    clear_display(out, state)?;

    let n = state.candidates.len().min(MAX_VISIBLE) as u16;
    if n == 0 {
        state.rendered_lines = 0;
        return Ok(());
    }

    reserve_lines(out, n)?;
    queue!(out, cursor::SavePosition)?;

    for (i, c) in state.candidates.iter().take(n as usize).enumerate() {
        queue!(
            out,
            cursor::MoveToNextLine(1),
            Clear(ClearType::CurrentLine)
        )?;
        let icon = source_icon(c.source);
        if i == state.selected {
            queue!(
                out,
                SetForegroundColor(Color::Cyan),
                SetAttribute(Attribute::Bold),
                Print(format!(" ▸ {icon} ")),
                ResetColor
            )?;
            write_highlighted(out, &c.text, &c.match_positions, true)?;
        } else {
            queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("   {icon} ")),
                ResetColor
            )?;
            write_highlighted(out, &c.text, &c.match_positions, false)?;
        }
    }

    queue!(out, cursor::RestorePosition)?;
    out.flush()?;
    state.rendered_lines = n;
    Ok(())
}

fn write_highlighted<W: Write>(
    out: &mut W,
    text: &str,
    positions: &[u32],
    selected: bool,
) -> Result<()> {
    let mut sorted: Vec<u32> = positions.to_vec();
    sorted.sort_unstable();

    for (i, ch) in text.chars().enumerate() {
        let is_match = sorted.binary_search(&(i as u32)).is_ok();
        if selected && is_match {
            queue!(
                out,
                SetForegroundColor(Color::White),
                SetAttribute(Attribute::Bold)
            )?;
        } else if selected {
            queue!(
                out,
                SetForegroundColor(Color::Grey),
                SetAttribute(Attribute::Reset)
            )?;
        } else if is_match {
            queue!(
                out,
                SetForegroundColor(Color::Yellow),
                SetAttribute(Attribute::Reset)
            )?;
        } else {
            queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                SetAttribute(Attribute::Reset)
            )?;
        }
        queue!(out, Print(ch))?;
    }
    queue!(out, ResetColor)?;
    Ok(())
}

fn clear_display<W: Write>(out: &mut W, state: &mut State) -> Result<()> {
    let n = state.rendered_lines;
    if n == 0 {
        return Ok(());
    }
    queue!(out, cursor::SavePosition)?;
    for _ in 0..n {
        queue!(
            out,
            cursor::MoveToNextLine(1),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(out, cursor::RestorePosition)?;
    out.flush()?;
    state.rendered_lines = 0;
    Ok(())
}

pub(crate) fn source_icon(s: CandidateSource) -> &'static str {
    match s {
        CandidateSource::History => "H",
        CandidateSource::Script => "S",
        CandidateSource::ScriptHistory => "B",
        CandidateSource::Path => "P",
    }
}

fn query_daemon(bridge: &DaemonBridge, state: &mut State) {
    let req = QueryRequest {
        buffer: state.input.clone(),
        cwd: state.cwd.clone(),
        match_mode: String::new(),
    };
    if let Some(resp) = bridge.query(req) {
        state.candidates = resp.candidates;
        if state.selected >= state.candidates.len() {
            state.selected = state.candidates.len().saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_covers_every_source() {
        for &s in &[
            CandidateSource::History,
            CandidateSource::Script,
            CandidateSource::ScriptHistory,
            CandidateSource::Path,
        ] {
            // never panic, never empty
            assert!(!source_icon(s).is_empty());
        }
    }

    #[test]
    fn write_highlighted_keeps_original_chars() {
        let mut buf = Vec::<u8>::new();
        write_highlighted(&mut buf, "git status", &[4, 5], false).unwrap();
        let rendered = String::from_utf8_lossy(&buf);
        // text content preserved (between the ANSI codes)
        assert!(rendered.contains('g'));
        assert!(rendered.contains('s'));
        assert!(rendered.ends_with("\x1b[0m"));
    }

    #[test]
    fn write_highlighted_handles_unicode_positions() {
        // "日本" is 2 chars (6 bytes). Match position 1 must highlight "本".
        let mut buf = Vec::<u8>::new();
        write_highlighted(&mut buf, "日本", &[1], false).unwrap();
        let rendered = String::from_utf8_lossy(&buf);
        assert!(rendered.contains('日'));
        assert!(rendered.contains('本'));
    }
}
