use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

use anyhow::Result;
use tab_core::Config;

struct Setting {
    label: &'static str,
    options: Vec<String>,
    selected: usize,
}

pub fn run() -> Result<()> {
    let config = Config::load();

    let mut settings = vec![
        Setting {
            label: "Match mode",
            options: vec!["fuzzy".into(), "prefix".into()],
            selected: if config.completion.match_mode == "prefix" { 1 } else { 0 },
        },
        Setting {
            label: "Max results",
            options: (4..=16).map(|n| n.to_string()).collect(),
            selected: (config.completion.max_results.clamp(4, 16) - 4),
        },
        Setting {
            label: "Log level",
            options: vec!["".into(), "error".into(), "warn".into(), "info".into(), "debug".into(), "trace".into()],
            selected: match config.log.level.as_str() {
                "error" => 1,
                "warn" => 2,
                "info" => 3,
                "debug" => 4,
                "trace" => 5,
                _ => 0,
            },
        },
    ];

    let mut cursor = 0usize;

    let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let tty_fd = tty.as_raw_fd();

    let orig = termios_get(tty_fd)?;
    let mut raw = orig;
    termios_make_raw(&mut raw);
    termios_set(tty_fd, &raw)?;

    // Hide cursor
    let _ = tty.write_all(b"\x1b[?25l");

    render(&mut tty, &settings, cursor);

    let mut buf = [0u8; 32];
    let saved;
    loop {
        let n = tty.read(&mut buf)?;
        if n == 0 { saved = false; break; }

        match &buf[..n] {
            // Esc or Ctrl-C
            [27] | [3] => { saved = false; break; }

            // Enter
            [13] => { saved = true; break; }

            // Up
            [27, 91, 65] | [27, 79, 65] => {
                cursor = cursor.saturating_sub(1);
                render(&mut tty, &settings, cursor);
            }

            // Down
            [27, 91, 66] | [27, 79, 66] => {
                if cursor < settings.len() - 1 { cursor += 1; }
                render(&mut tty, &settings, cursor);
            }

            // Left
            [27, 91, 68] | [27, 79, 68] => {
                let s = &mut settings[cursor];
                if s.selected > 0 { s.selected -= 1; }
                render(&mut tty, &settings, cursor);
            }

            // Right
            [27, 91, 67] | [27, 79, 67] => {
                let s = &mut settings[cursor];
                if s.selected < s.options.len() - 1 { s.selected += 1; }
                render(&mut tty, &settings, cursor);
            }

            _ => {}
        }
    }

    // Restore terminal
    let _ = tty.write_all(b"\x1b[?25h"); // show cursor
    let _ = termios_set(tty_fd, &orig);
    // Clear rendered area
    let lines = settings.len() + 3; // header + settings + footer + blank
    let mut out = String::new();
    out.push_str("\x1b[s");
    for _ in 0..lines {
        out.push_str("\r\n\x1b[2K");
    }
    out.push_str("\x1b[u");
    let _ = tty.write_all(out.as_bytes());
    let _ = tty.flush();

    if saved {
        let mut new_config = config;
        new_config.completion.match_mode = settings[0].options[settings[0].selected].clone();
        new_config.completion.max_results = settings[1].options[settings[1].selected]
            .parse()
            .unwrap_or(8);
        new_config.log.level = settings[2].options[settings[2].selected].clone();
        new_config.save()?;
        println!("Settings saved.");
    } else {
        println!("Cancelled.");
    }

    Ok(())
}

fn render(tty: &mut std::fs::File, settings: &[Setting], cursor: usize) {
    let mut out = String::new();

    let total_lines = settings.len() + 3;
    // Create space
    out.push_str("\x1b[s");
    for _ in 0..total_lines {
        out.push_str("\r\n");
    }
    out.push_str(&format!("\x1b[{}A", total_lines));

    // Header
    out.push_str("\r\n\x1b[2K");
    out.push_str("\x1b[1m  tab settings\x1b[0m");
    out.push_str("\r\n\x1b[2K");
    out.push_str("\x1b[90m  ─────────────────────────────────\x1b[0m");

    // Settings
    for (i, s) in settings.iter().enumerate() {
        out.push_str("\r\n\x1b[2K");
        let marker = if i == cursor { "\x1b[36m▸\x1b[0m" } else { " " };
        out.push_str(&format!("  {marker} \x1b[1m{:<14}\x1b[0m", s.label));

        for (j, opt) in s.options.iter().enumerate() {
            if j == s.selected {
                out.push_str(&format!(" \x1b[7m {opt} \x1b[0m"));
            } else {
                out.push_str(&format!(" \x1b[90m {opt} \x1b[0m"));
            }
        }
    }

    // Footer
    out.push_str("\r\n\x1b[2K");
    out.push_str("\x1b[90m  ↑↓ navigate  ←→ change  Enter save  Esc cancel\x1b[0m");

    out.push_str("\x1b[u");
    let _ = tty.write_all(out.as_bytes());
    let _ = tty.flush();
}

// ── Termios helpers (same as tui.rs) ──

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
    unsafe { libc::cfmakeraw(t as *mut Termios as *mut libc::termios); }
    t.c_cc[libc::VMIN] = 1;
    t.c_cc[libc::VTIME] = 0;
}
