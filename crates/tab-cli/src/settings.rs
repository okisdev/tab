use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType},
};

use tab_core::Config;

use crate::term::{reserve_lines, TerminalGuard};

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
            selected: if config.completion.match_mode == "prefix" {
                1
            } else {
                0
            },
        },
        Setting {
            label: "Max results",
            options: (4..=16).map(|n| n.to_string()).collect(),
            selected: config.completion.max_results.clamp(4, 16) - 4,
        },
        Setting {
            label: "Log level",
            options: vec![
                "".into(),
                "error".into(),
                "warn".into(),
                "info".into(),
                "debug".into(),
                "trace".into(),
            ],
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

    let mut out = io::stderr();
    let _guard = TerminalGuard::enter_hidden(&mut out)?;

    let total_lines: u16 = (settings.len() + 3) as u16;
    reserve_lines(&mut out, total_lines)?;
    queue!(out, cursor::SavePosition)?;
    out.flush()?;

    let mut cursor_i = 0usize;
    render(&mut out, &settings, cursor_i)?;

    let saved = loop {
        match event::read() {
            Ok(Event::Key(key)) => {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break false,
                    (KeyCode::Enter, _) => break true,
                    (KeyCode::Up, _) => {
                        cursor_i = cursor_i.saturating_sub(1);
                        render(&mut out, &settings, cursor_i)?;
                    }
                    (KeyCode::Down, _) => {
                        if cursor_i + 1 < settings.len() {
                            cursor_i += 1;
                        }
                        render(&mut out, &settings, cursor_i)?;
                    }
                    (KeyCode::Left, _) => {
                        let s = &mut settings[cursor_i];
                        if s.selected > 0 {
                            s.selected -= 1;
                        }
                        render(&mut out, &settings, cursor_i)?;
                    }
                    (KeyCode::Right, _) => {
                        let s = &mut settings[cursor_i];
                        if s.selected + 1 < s.options.len() {
                            s.selected += 1;
                        }
                        render(&mut out, &settings, cursor_i)?;
                    }
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(_) => break false,
        }
    };

    queue!(out, cursor::RestorePosition)?;
    for _ in 0..total_lines {
        queue!(
            out,
            cursor::MoveToNextLine(1),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(out, cursor::RestorePosition)?;
    out.flush()?;
    // _guard drops → cursor::Show + disable_raw_mode

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

fn render<W: Write>(out: &mut W, settings: &[Setting], cursor_i: usize) -> Result<()> {
    queue!(out, cursor::RestorePosition)?;

    queue!(
        out,
        Clear(ClearType::CurrentLine),
        SetAttribute(Attribute::Bold),
        Print("  tab settings"),
        ResetColor
    )?;

    queue!(
        out,
        cursor::MoveToNextLine(1),
        Clear(ClearType::CurrentLine),
        SetForegroundColor(Color::DarkGrey),
        Print("  ─────────────────────────────────"),
        ResetColor
    )?;

    for (i, s) in settings.iter().enumerate() {
        queue!(
            out,
            cursor::MoveToNextLine(1),
            Clear(ClearType::CurrentLine)
        )?;
        let marker = if i == cursor_i { "▸" } else { " " };
        queue!(
            out,
            Print("  "),
            SetForegroundColor(Color::Cyan),
            Print(marker),
            ResetColor,
            Print(" "),
            SetAttribute(Attribute::Bold),
            Print(format!("{:<14}", s.label)),
            ResetColor
        )?;
        for (j, opt) in s.options.iter().enumerate() {
            let label = if opt.is_empty() {
                "(default)"
            } else {
                opt.as_str()
            };
            if j == s.selected {
                queue!(
                    out,
                    Print(" "),
                    SetBackgroundColor(Color::White),
                    SetForegroundColor(Color::Black),
                    Print(format!(" {label} ")),
                    ResetColor
                )?;
            } else {
                queue!(
                    out,
                    Print(" "),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!(" {label} ")),
                    ResetColor
                )?;
            }
        }
    }

    queue!(
        out,
        cursor::MoveToNextLine(1),
        Clear(ClearType::CurrentLine),
        SetForegroundColor(Color::DarkGrey),
        Print("  ↑↓ navigate  ←→ change  Enter save  Esc cancel"),
        ResetColor
    )?;

    out.flush()?;
    Ok(())
}
