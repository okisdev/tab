use anyhow::Result;

pub fn print_init_script(shell: &str) -> Result<()> {
    match shell {
        "zsh" => print!("{}", ZSH_INIT),
        "bash" => {
            eprintln!("tab: bash integration coming soon");
        }
        "fish" => {
            eprintln!("tab: fish integration coming soon");
        }
        _ => anyhow::bail!("unsupported shell: {shell}"),
    }
    Ok(())
}

const ZSH_INIT: &str = r#"
# tab - terminal autocomplete plugin
# Add to .zshrc: eval "$(tab init zsh)"

# ── State ──

__tab_sid="tab-$$-$(date +%s)"
__tab_bin="${commands[tab]:-tab}"
__tab_selected=0
__tab_active=0

# ── Coprocess management ──

__tab_start_coproc() {
    if [[ -z "${__tab_coproc_pid:-}" ]] || ! kill -0 "$__tab_coproc_pid" 2>/dev/null; then
        coproc "$__tab_bin" hook --shell zsh --session "$__tab_sid" 2>/dev/null
        __tab_coproc_pid=$!
    fi
}

# Send message (fire-and-forget, no response expected)
__tab_send() {
    __tab_start_coproc
    if ! print -p -- "$1" 2>/dev/null; then
        # Coprocess died, restart and retry once
        __tab_coproc_pid=""
        __tab_start_coproc
        print -p -- "$1" 2>/dev/null
    fi
}

# Send message and read response (blocking with timeout)
__tab_send_recv() {
    __tab_start_coproc
    __tab_response=""
    if ! print -p -- "$1" 2>/dev/null; then
        __tab_coproc_pid=""
        __tab_start_coproc
        print -p -- "$1" 2>/dev/null || return 1
    fi
    read -t 0.1 -p __tab_response 2>/dev/null
}

# ── JSON helpers (pure zsh, no python) ──

# Extract a string value for a given key from flat JSON
__tab_json_get() {
    local json="$1" key="$2"
    # Match "key":"value" or "key":number
    if [[ "$json" =~ \"$key\":\"([^\"]*)\" ]]; then
        echo "${match[1]}"
    elif [[ "$json" =~ \"$key\":([0-9]+) ]]; then
        echo "${match[1]}"
    fi
}

# ── Core actions ──

# Notify daemon of buffer change (fire-and-forget)
__tab_update() {
    [[ -z "$BUFFER" ]] && {
        __tab_active=0
        __tab_send "{\"type\":\"dismiss\",\"session_id\":\"$__tab_sid\"}"
        return
    }
    __tab_active=1
    __tab_selected=0
    local buf="${BUFFER//\\/\\\\}"
    buf="${buf//\"/\\\"}"
    local cwd="${PWD//\\/\\\\}"
    cwd="${cwd//\"/\\\"}"
    __tab_send "{\"type\":\"context\",\"session_id\":\"$__tab_sid\",\"shell\":\"zsh\",\"buffer\":\"$buf\",\"cursor_pos\":$CURSOR,\"cwd\":\"$cwd\",\"columns\":$COLUMNS,\"lines\":$LINES}"
}

# Accept the currently selected completion
__tab_accept() {
    (( __tab_active )) || { zle expand-or-complete; return; }
    __tab_send_recv "{\"type\":\"accept\",\"session_id\":\"$__tab_sid\",\"index\":$__tab_selected}"
    if [[ -n "$__tab_response" ]]; then
        local text=$(__tab_json_get "$__tab_response" "text")
        if [[ -n "$text" ]]; then
            BUFFER="$text"
            CURSOR=${#BUFFER}
            __tab_active=0
            __tab_selected=0
            zle redisplay
            return
        fi
    fi
    # Fallback to default tab completion if no match
    zle expand-or-complete
}

# Navigate candidates
__tab_navigate() {
    (( __tab_active )) || return
    __tab_send_recv "{\"type\":\"navigate\",\"session_id\":\"$__tab_sid\",\"direction\":\"$1\"}"
    if [[ -n "$__tab_response" ]]; then
        local sel=$(__tab_json_get "$__tab_response" "selected")
        [[ -n "$sel" ]] && __tab_selected=$sel
    fi
}

# Dismiss popup
__tab_dismiss() {
    __tab_active=0
    __tab_selected=0
    __tab_send "{\"type\":\"dismiss\",\"session_id\":\"$__tab_sid\"}"
}

# ── ZLE widget wrappers ──

__tab_wrap_widget() {
    local widget="$1"
    eval "
        __tab_orig_${widget}() { zle .${widget}; }
        __tab_wrapped_${widget}() {
            __tab_orig_${widget}
            __tab_update
        }
        zle -N ${widget} __tab_wrapped_${widget}
    "
}

# Wrap standard editing widgets
__tab_wrap_widget self-insert
__tab_wrap_widget backward-delete-char
__tab_wrap_widget delete-char
__tab_wrap_widget kill-word
__tab_wrap_widget backward-kill-word
__tab_wrap_widget yank
__tab_wrap_widget kill-line
__tab_wrap_widget backward-kill-line

# Accept widget (Tab)
__tab_accept_widget() { __tab_accept; }
zle -N __tab_accept_widget

# Navigate up (Ctrl+P)
__tab_nav_up_widget() { __tab_navigate up; }
zle -N __tab_nav_up_widget

# Navigate down (Ctrl+N)
__tab_nav_down_widget() { __tab_navigate down; }
zle -N __tab_nav_down_widget

# Dismiss widget (Escape)
__tab_dismiss_widget() {
    if (( __tab_active )); then
        __tab_dismiss
        zle redisplay
    else
        zle send-break
    fi
}
zle -N __tab_dismiss_widget

# ── Key bindings ──

bindkey '^I'  __tab_accept_widget      # Tab
bindkey '^N'  __tab_nav_down_widget    # Ctrl+N
bindkey '^P'  __tab_nav_up_widget     # Ctrl+P
bindkey '\e'  __tab_dismiss_widget    # Escape

# preexec: dismiss popup when a command runs
__tab_preexec() { __tab_dismiss; }
autoload -Uz add-zsh-hook
add-zsh-hook preexec __tab_preexec

# Cleanup on exit
__tab_cleanup() {
    __tab_dismiss
    [[ -n "${__tab_coproc_pid:-}" ]] && kill "$__tab_coproc_pid" 2>/dev/null
}
trap __tab_cleanup EXIT
"#;
