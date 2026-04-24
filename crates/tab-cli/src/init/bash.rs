pub const SCRIPT: &str = r#"
# tab - terminal autocomplete plugin (bash)
# Install: eval "$(tab init bash)"
#
# Bash does not expose POSTDISPLAY-style ghost text, so Tab opens the
# interactive picker. Selected command replaces the current line.

# `bind -x` requires bash 4.0+. macOS ships bash 3.2 by default — warn once
# and bail so the user is told why Tab does nothing.
if [[ -z "${BASH_VERSINFO[0]:-}" || "${BASH_VERSINFO[0]}" -lt 4 ]]; then
    printf 'tab: bash >= 4 is required (have %s). On macOS: brew install bash\n' \
        "${BASH_VERSION:-<unknown>}" >&2
    return 0 2>/dev/null || exit 0
fi

__tab_bin="${TAB_BIN:-tab}"

__tab_complete() {
    local selected
    selected=$(command "$__tab_bin" complete --buffer "$READLINE_LINE" --cwd "$PWD" 2>/dev/null </dev/tty)
    local rc=$?
    if [[ $rc -eq 0 && -n "$selected" ]]; then
        READLINE_LINE="$selected"
        READLINE_POINT=${#READLINE_LINE}
    fi
}

# Only bind if interactive and readline is available
if [[ $- == *i* ]] && type bind >/dev/null 2>&1; then
    bind -x '"\C-i": __tab_complete' 2>/dev/null || \
    bind -x '"\t": __tab_complete' 2>/dev/null || true
fi
"#;
