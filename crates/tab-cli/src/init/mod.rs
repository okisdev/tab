mod bash;
mod fish;
mod pwsh;
mod zsh;

use anyhow::Result;

pub fn print_init_script(shell: &str) -> Result<()> {
    print!("{}", script_for(shell)?);
    Ok(())
}

/// Isolate the shell → script mapping for unit tests.
pub(crate) fn script_for(shell: &str) -> Result<&'static str> {
    Ok(match shell.to_ascii_lowercase().as_str() {
        "zsh" => zsh::SCRIPT,
        "bash" => bash::SCRIPT,
        "fish" => fish::SCRIPT,
        "pwsh" | "powershell" => pwsh::SCRIPT,
        other => anyhow::bail!("unsupported shell: {other} (try zsh, bash, fish, pwsh)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_supported_shells_emit_non_empty_script() {
        for shell in ["zsh", "bash", "fish", "pwsh", "powershell"] {
            let s = script_for(shell).expect(shell);
            assert!(!s.is_empty(), "{shell} script is empty");
        }
    }

    #[test]
    fn case_insensitive_shell_name() {
        assert!(script_for("ZSH").is_ok());
        assert!(script_for("Bash").is_ok());
        assert!(script_for("PWSH").is_ok());
    }

    #[test]
    fn unknown_shell_errors() {
        assert!(script_for("tcsh").is_err());
        assert!(script_for("").is_err());
    }

    #[test]
    fn zsh_script_mentions_coproc_and_zle() {
        let s = script_for("zsh").unwrap();
        assert!(s.contains("coproc"));
        assert!(s.contains("zle -N"));
        assert!(s.contains("POSTDISPLAY"));
    }

    #[test]
    fn bash_script_uses_bind_x() {
        let s = script_for("bash").unwrap();
        assert!(s.contains("bind -x"));
        assert!(s.contains("READLINE_LINE"));
    }

    #[test]
    fn fish_script_binds_tab_in_both_modes() {
        let s = script_for("fish").unwrap();
        assert!(s.contains("bind \\t"));
        assert!(s.contains("bind -M insert"));
    }

    #[test]
    fn pwsh_script_uses_psreadline_handler() {
        let s = script_for("pwsh").unwrap();
        assert!(s.contains("Set-PSReadLineKeyHandler"));
        assert!(s.contains("PSConsoleReadLine"));
    }

    #[test]
    fn zsh_script_strips_control_chars() {
        // Regression guard: buffer must be sanitized before JSON encoding.
        let s = script_for("zsh").unwrap();
        assert!(
            s.contains("${BUFFER//$'\\t'/ }"),
            "zsh script must strip literal tabs from BUFFER before JSON encoding"
        );
        assert!(
            s.contains("${buf//$'\\x1f'/ }"),
            "zsh script must strip \\x1f (field separator) from buf"
        );
        assert!(s.contains("${buf//$'\\n'/ }"));
        assert!(s.contains("${buf//$'\\r'/ }"));
    }

    #[test]
    fn zsh_script_compares_echo_to_sanitized_buf() {
        // Regression: previously the echo-correlation used `$BUFFER` (raw),
        // but the daemon echoes the sanitized (post-JSON-parse) value, so
        // multi-line / tab-containing paste always failed to render.
        let s = script_for("zsh").unwrap();
        assert!(
            s.contains(r#"__tab_send_recv "{\"buffer\":\"$json_buf\",\"cwd\":\"$cwd\"}" "$buf""#),
            "echo comparison must use sanitized `$buf`, not raw `$BUFFER`"
        );
    }

    #[test]
    fn zsh_script_resets_state_on_new_line() {
        // Regression: Ctrl-C left __tab_active=1, causing the next Tab to
        // insert a stale candidate. zle-line-init widget must reset.
        let s = script_for("zsh").unwrap();
        assert!(s.contains("zle -N zle-line-init __tab_line_init_widget"));
        assert!(s.contains("__tab_reset_state"));
    }

    #[test]
    fn zsh_script_disables_autosuggestions() {
        // zsh-autosuggestions also writes POSTDISPLAY and wraps self-insert;
        // when both are loaded, whichever sourced last silently wins. Turn
        // autosuggestions off so tab's ghost text is authoritative.
        let s = script_for("zsh").unwrap();
        assert!(s.contains("_zsh_autosuggest_disable"));
    }

    #[test]
    fn bash_script_rejects_old_bash() {
        // macOS default bash is 3.2 — `bind -x` requires 4+. The script
        // should tell the user rather than silently no-op.
        let s = script_for("bash").unwrap();
        assert!(s.contains("BASH_VERSINFO"));
        assert!(s.contains("brew install bash"));
    }

    #[test]
    fn pwsh_script_resolves_binary_dynamically() {
        // Resolving `tab` once at profile source time made later installs
        // invisible. Lookup must happen inside the key handler.
        let s = script_for("pwsh").unwrap();
        assert!(
            s.contains("$tabBin = (Get-Command tab"),
            "tab binary must be resolved per-invocation, not at source time"
        );
        // Confirm the old source-time snapshot is gone.
        assert!(!s.contains("$script:TabBin"));
    }
}
