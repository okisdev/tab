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

__tab_bin="${commands[tab]:-tab}"
__tab_selected=0
__tab_active=0
__tab_candidates=()
__tab_sources=()

# ── Coprocess ──

__tab_start_coproc() {
    if [[ -z "${__tab_coproc_pid:-}" ]] || ! kill -0 "$__tab_coproc_pid" 2>/dev/null; then
        setopt LOCAL_OPTIONS NO_MONITOR NO_NOTIFY 2>/dev/null
        coproc { trap '' INT; exec "$__tab_bin" hook; } 2>/dev/null
        __tab_coproc_pid=$!
    fi
}

__tab_send_recv() {
    __tab_start_coproc
    __tab_response=""
    # Drain stale responses from the pipe (user typed faster than daemon responded)
    while read -t 0 -p __tab_discard 2>/dev/null; do :; done
    if print -p -- "$1" 2>/dev/null; then
        read -t 0.2 -p __tab_response 2>/dev/null
    else
        __tab_coproc_pid=""
        __tab_start_coproc
        print -p -- "$1" 2>/dev/null && read -t 0.2 -p __tab_response 2>/dev/null
    fi
}

# ── Parse response into candidates array ──

__tab_parse() {
    __tab_candidates=()
    __tab_sources=()
    [[ -z "$__tab_response" ]] && return 1

    local _sep=$'\x1f'
    local -a entries=("${(@ps.$_sep.)__tab_response}")

    local entry
    for entry in "${entries[@]}"; do
        [[ -z "$entry" ]] && continue
        __tab_sources+=("${entry[1]}")
        __tab_candidates+=("${entry[3,-1]}")
    done
    (( ${#__tab_candidates[@]} > 0 ))
}

# ── Render candidates via zle -M + ghost text via POSTDISPLAY ──

__tab_render() {
    local n=${#__tab_candidates[@]}
    if (( n == 0 )); then
        POSTDISPLAY=""
        return
    fi

    # Candidate list below prompt
    local msg="" i icon
    for (( i = 1; i <= n; i++ )); do
        case "${__tab_sources[$i]}" in
            H) icon="🕘" ;; S) icon="⚡" ;; B) icon="⚡🕘" ;; *) icon="📁" ;;
        esac
        if (( i - 1 == __tab_selected )); then
            msg+=" ▸ $icon ${__tab_candidates[$i]}"
        else
            msg+="   $icon ${__tab_candidates[$i]}"
        fi
        (( i < n )) && msg+=$'\n'
    done
    zle -M "$msg"

    # Ghost text: show remainder of selected candidate after cursor
    local selected="${__tab_candidates[$(( __tab_selected + 1 ))]}"
    if [[ "$selected" == "$BUFFER"* ]]; then
        POSTDISPLAY="${selected#$BUFFER}"
    else
        POSTDISPLAY=""
    fi
}

# ── Core actions ──

__tab_update() {
    [[ -z "$BUFFER" ]] && { __tab_active=0; __tab_candidates=(); POSTDISPLAY=""; return; }
    __tab_active=1
    __tab_selected=0
    local buf="${BUFFER//\\/\\\\}"
    buf="${buf//\"/\\\"}"
    local cwd="${PWD//\\/\\\\}"
    cwd="${cwd//\"/\\\"}"
    __tab_send_recv "{\"buffer\":\"$buf\",\"cwd\":\"$cwd\"}"
    if __tab_parse; then
        __tab_render
    else
        __tab_active=0
    fi
}

__tab_accept() {
    (( __tab_active )) || { zle expand-or-complete; return; }
    local text="${__tab_candidates[$(( __tab_selected + 1 ))]}"
    if [[ -n "$text" ]]; then
        BUFFER="$text"
        CURSOR=${#BUFFER}
        POSTDISPLAY=""  # clear autosuggestions ghost text
    fi
    __tab_active=0
    __tab_candidates=()
}

# ── Widget wrappers ──

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

__tab_wrap_widget self-insert
__tab_wrap_widget backward-delete-char
__tab_wrap_widget delete-char
__tab_wrap_widget backward-kill-word
__tab_wrap_widget kill-line
__tab_wrap_widget kill-word

# Tab: accept selected candidate
__tab_accept_widget() { __tab_accept; }
zle -N __tab_accept_widget
bindkey '^I' __tab_accept_widget

# Right arrow: accept selected candidate if active, otherwise normal movement
__tab_forward_char() {
    if [[ $CURSOR -eq ${#BUFFER} ]] && (( __tab_active )); then
        __tab_accept
    else
        zle .forward-char
    fi
}
zle -N __tab_forward_char
bindkey '\e[C' __tab_forward_char
bindkey '\eOC' __tab_forward_char

# Up/Down: navigate candidates (fall through to history if inactive)
__tab_nav_up() {
    if (( __tab_active )); then
        (( __tab_selected > 0 )) && (( __tab_selected-- ))
        __tab_render
    else
        zle up-line-or-history
    fi
}
__tab_nav_down() {
    if (( __tab_active )); then
        (( __tab_selected < ${#__tab_candidates[@]} - 1 )) && (( __tab_selected++ ))
        __tab_render
    else
        zle down-line-or-history
    fi
}
zle -N __tab_nav_up
zle -N __tab_nav_down
bindkey '\e[A' __tab_nav_up
bindkey '\e[B' __tab_nav_down
bindkey '\eOA' __tab_nav_up
bindkey '\eOB' __tab_nav_down

# preexec: reset
__tab_preexec() { __tab_active=0; __tab_candidates=(); }
autoload -Uz add-zsh-hook
add-zsh-hook preexec __tab_preexec

# Cleanup
__tab_cleanup() {
    [[ -n "${__tab_coproc_pid:-}" ]] && kill "$__tab_coproc_pid" 2>/dev/null
}
trap __tab_cleanup EXIT
"#;
