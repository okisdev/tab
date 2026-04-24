use std::io::Write;

use anyhow::Result;
use crossterm::{
    cursor, queue,
    terminal::{disable_raw_mode, enable_raw_mode, ScrollUp},
};

/// RAII guard that always leaves the terminal in a sane state (raw mode off,
/// cursor visible) on `?` / panic / normal drop.
pub struct TerminalGuard {
    show_cursor: bool,
}

impl TerminalGuard {
    /// Enable raw mode. Cursor stays visible.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self { show_cursor: false })
    }

    /// Enable raw mode *and* hide the cursor. Drop restores both.
    pub fn enter_hidden<W: Write>(out: &mut W) -> Result<Self> {
        enable_raw_mode()?;
        queue!(out, cursor::Hide)?;
        out.flush()?;
        Ok(Self { show_cursor: true })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.show_cursor {
            let mut out = std::io::stderr();
            let _ = queue!(out, cursor::Show);
            let _ = out.flush();
        }
        let _ = disable_raw_mode();
    }
}

/// Reserve `n` lines below the cursor without destroying more scrollback than
/// necessary. Only scrolls by the shortfall between needed lines and what's
/// already free below the cursor — unlike an unconditional `ScrollUp(n)` which
/// discards the top `n` rows even when the cursor is mid-viewport.
pub fn reserve_lines<W: Write>(out: &mut W, n: u16) -> Result<()> {
    if n == 0 {
        return Ok(());
    }
    let (_, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (_, row) = crossterm::cursor::position().unwrap_or((0, rows.saturating_sub(1)));
    let needed = scroll_shortfall(row, rows, n);
    if needed > 0 {
        queue!(out, ScrollUp(needed), cursor::MoveUp(needed))?;
    }
    Ok(())
}

/// Pure helper: given cursor row, total rows, and `n` rows required below,
/// returns the number of lines `ScrollUp` must scroll.
fn scroll_shortfall(row: u16, rows: u16, n: u16) -> u16 {
    let below = rows.saturating_sub(row + 1);
    n.saturating_sub(below)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortfall_no_scroll_when_room_below() {
        assert_eq!(scroll_shortfall(10, 30, 5), 0); // 19 rows below, need 5
        assert_eq!(scroll_shortfall(24, 30, 5), 0); // 5 rows below, need 5
    }

    #[test]
    fn shortfall_partial_scroll_near_bottom() {
        assert_eq!(scroll_shortfall(27, 30, 5), 3); // 2 below, need 5 → scroll 3
        assert_eq!(scroll_shortfall(29, 30, 5), 5); // 0 below, need 5 → scroll 5
    }

    #[test]
    fn shortfall_zero_n() {
        assert_eq!(scroll_shortfall(0, 30, 0), 0);
        assert_eq!(scroll_shortfall(29, 30, 0), 0);
    }

    #[test]
    fn shortfall_saturates() {
        // cursor past viewport (shouldn't happen but must not panic)
        assert_eq!(scroll_shortfall(99, 30, 5), 5);
    }
}
