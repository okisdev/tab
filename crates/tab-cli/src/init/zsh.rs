pub const SCRIPT: &str = r#"
# tab - zsh integration (async ghost-text). Install: eval "$(tab init zsh)"

__tab_bin="${commands[tab]:-tab}"
__tab_selected=0
__tab_active=0
__tab_dismissed=0
__tab_candidates=()
__tab_sources=()
__tab_coproc_pid=""
__tab_fd_in=""
__tab_fd_out=""

: ${TAB_GHOST_STYLE:=fg=8}
: ${TAB_INPUT_STYLE:=fg=green}
: ${TAB_SHOW_ICONS:=0}

__tab_close_coproc() {
    # `exec REDIR` with no command applies all redirections to the current
    # shell permanently, so a trailing `2>/dev/null` here would silently leave
    # the shell's stderr pointing at /dev/null forever. Wrap the exec in a
    # block so the noise suppression only scopes to the close itself.
    if [[ -n "$__tab_fd_out" ]]; then
        zle -F "$__tab_fd_out" 2>/dev/null
        { exec {__tab_fd_out}<&- } 2>/dev/null
    fi
    if [[ -n "$__tab_fd_in" ]]; then
        { exec {__tab_fd_in}>&- } 2>/dev/null
    fi
    if [[ -n "$__tab_coproc_pid" ]] && kill -0 "$__tab_coproc_pid" 2>/dev/null; then
        kill "$__tab_coproc_pid" 2>/dev/null
    fi
    __tab_coproc_pid=""
    __tab_fd_in=""
    __tab_fd_out=""
}

__tab_start_coproc() {
    if [[ -n "$__tab_coproc_pid" ]] && kill -0 "$__tab_coproc_pid" 2>/dev/null; then
        return 0
    fi
    __tab_close_coproc
    setopt LOCAL_OPTIONS NO_MONITOR NO_NOTIFY 2>/dev/null
    coproc { trap '' INT; exec "$__tab_bin" hook; } 2>/dev/null || return 1
    __tab_coproc_pid=$!
    # Drop the coproc from the job table; otherwise `%j` stays >=1 for the
    # life of the session and lights up prompt segments like powerlevel10k's
    # background_jobs. Liveness is still tracked via $__tab_coproc_pid.
    builtin disown 2>/dev/null
    # zle -F needs a real fd number; the `-p` shorthand isn't accepted there.
    # The `{ ... } 2>/dev/null` wrapping is required; see __tab_close_coproc.
    { exec {__tab_fd_out}<&p } 2>/dev/null || { __tab_close_coproc; return 1; }
    { exec {__tab_fd_in}>&p  } 2>/dev/null || { __tab_close_coproc; return 1; }
    zle -F "$__tab_fd_out" __tab_response_handler 2>/dev/null
    return 0
}

__tab_send_async() {
    __tab_start_coproc || return 1
    if ! print -u "$__tab_fd_in" -- "$1" 2>/dev/null; then
        __tab_close_coproc
        __tab_start_coproc || return 1
        print -u "$__tab_fd_in" -- "$1" 2>/dev/null || return 1
    fi
    return 0
}

# zle -F callbacks run outside widget context (BUFFER/CURSOR/POSTDISPLAY not
# exposed). Drain the pipe here and trampoline into a real widget.
__tab_response_handler() {
    local fd=$1
    local _resp
    while IFS= read -r -u "$fd" -t 0 _resp 2>/dev/null; do
        [[ -z "$_resp" ]] && continue
        zle __tab_apply_response -- "$_resp"
    done
    return 0
}

__tab_apply_response_widget() {
    local _resp="$1"
    local _sep=$'\x1f'
    local _echo="${_resp%%$_sep*}"
    local cur_buf="${BUFFER//$'\t'/ }"
    cur_buf="${cur_buf//$'\n'/ }"
    cur_buf="${cur_buf//$'\r'/ }"
    cur_buf="${cur_buf//$'\x1f'/ }"
    if [[ -z "$BUFFER" || $__tab_dismissed -eq 1 || "$_echo" != "$cur_buf" ]]; then
        return 0
    fi
    __tab_response="$_resp"
    if __tab_parse; then
        __tab_active=1
        __tab_selected=0
        __tab_render
    else
        __tab_active=0
        __tab_candidates=()
        __tab_clear_highlight
        POSTDISPLAY=""
    fi
    zle -R
}
zle -N __tab_apply_response __tab_apply_response_widget

__tab_parse() {
    __tab_candidates=()
    __tab_sources=()
    [[ -z "$__tab_response" ]] && return 1

    local _sep=$'\x1f'
    local -a entries=("${(@ps.$_sep.)__tab_response}")

    local i entry
    for (( i = 2; i <= ${#entries[@]}; i++ )); do
        entry="${entries[$i]}"
        [[ -z "$entry" ]] && continue
        __tab_sources+=("${entry[1]}")
        __tab_candidates+=("${entry[3,-1]}")
    done
    (( ${#__tab_candidates[@]} > 0 ))
}

__tab_clear_highlight() {
    local -a _rh=()
    local _e
    for _e in "${(@)region_highlight}"; do
        # Match memo=tab as a whole token — don't clobber memo=tabular etc.
        [[ "$_e" == *"memo=tab" || "$_e" == *"memo=tab "* ]] || _rh+=("$_e")
    done
    region_highlight=("${_rh[@]}")
}

__tab_render() {
    local n=${#__tab_candidates[@]}
    if (( n == 0 )); then
        __tab_clear_highlight
        POSTDISPLAY=""
        zle -M ""
        return
    fi

    zle -M ""

    local selected="${__tab_candidates[$(( __tab_selected + 1 ))]}"
    local ghost=""
    [[ "$selected" == "$BUFFER"* ]] && ghost="${selected#$BUFFER}"

    local buf_len=${#BUFFER}
    local post="$ghost"
    local -a _tab_rh=()

    if [[ -n "$ghost" ]]; then
        _tab_rh+=("$buf_len $(( buf_len + ${#ghost} )) $TAB_GHOST_STYLE memo=tab")
    fi

    local i _cand icon prefix_str line
    for (( i = 1; i <= n; i++ )); do
        _cand="${__tab_candidates[$i]}"

        if [[ "$TAB_SHOW_ICONS" != "0" && -n "$TAB_SHOW_ICONS" ]]; then
            case "${__tab_sources[$i]}" in
                H) icon="H " ;; S|B) icon="S " ;; *) icon="P " ;;
            esac
        else
            icon=""
        fi

        if (( i - 1 == __tab_selected )); then
            prefix_str=$'\n'" > ${icon}"
        else
            prefix_str=$'\n'"   ${icon}"
        fi
        line="${prefix_str}${_cand}"

        local line_start=$(( buf_len + ${#post} ))
        post+="$line"

        if [[ -n "$BUFFER" && "$_cand" == "$BUFFER"* ]]; then
            local input_start=$(( line_start + ${#prefix_str} ))
            local input_end=$(( input_start + ${#BUFFER} ))
            _tab_rh+=("$input_start $input_end $TAB_INPUT_STYLE memo=tab")
            local gray_end=$(( line_start + ${#line} ))
            if (( input_end < gray_end )); then
                _tab_rh+=("$input_end $gray_end $TAB_GHOST_STYLE memo=tab")
            fi
        fi
    done

    POSTDISPLAY="$post"
    __tab_clear_highlight
    region_highlight+=("${_tab_rh[@]}")
}

__tab_update_async() {
    if [[ -z "$BUFFER" ]]; then
        __tab_active=0
        __tab_candidates=()
        __tab_clear_highlight
        POSTDISPLAY=""
        zle -M ""
        return
    fi
    __tab_dismissed=0
    # region_highlight indices break as soon as BUFFER changes by even one
    # char; drop stale state immediately and let the response repaint.
    __tab_clear_highlight
    POSTDISPLAY=""
    # Sanitize control bytes so bracketed-paste with embedded tabs/newlines
    # still forms valid JSON and the daemon's buffer-echo correlation matches.
    local buf="${BUFFER//$'\t'/ }"
    buf="${buf//$'\n'/ }"
    buf="${buf//$'\r'/ }"
    buf="${buf//$'\x1f'/ }"
    local json_buf="${buf//\\/\\\\}"
    json_buf="${json_buf//\"/\\\"}"
    local cwd="${PWD//\\/\\\\}"
    cwd="${cwd//\"/\\\"}"
    __tab_send_async "{\"buffer\":\"$json_buf\",\"cwd\":\"$cwd\"}"
}

__tab_accept() {
    (( __tab_active )) || { zle expand-or-complete; return; }
    local text="${__tab_candidates[$(( __tab_selected + 1 ))]}"
    if [[ -n "$text" ]]; then
        __tab_clear_highlight
        POSTDISPLAY=""
        BUFFER="$text"
        CURSOR=${#BUFFER}
    fi
    __tab_active=0
    __tab_candidates=()
    zle -M ""
}

__tab_wrap_widget() {
    local widget="$1"
    eval "
        __tab_orig_${widget}() { zle .${widget}; }
        __tab_wrapped_${widget}() {
            __tab_orig_${widget}
            __tab_update_async
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
__tab_wrap_widget bracketed-paste
__tab_wrap_widget yank

__tab_accept_widget() { __tab_accept; }
zle -N __tab_accept_widget
bindkey '^I' __tab_accept_widget

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

# Enter must always run accept-line; accepting candidates is Tab / Right's job.
# Gating it on "selection equals buffer" silently swallowed typos: the fuzzy
# matcher returns near-misses, __tab_active flips to 1, and the command never
# ran while $? stuck to its previous value.
__tab_enter() {
    if (( __tab_active )); then
        __tab_active=0
        __tab_candidates=()
        __tab_clear_highlight
        POSTDISPLAY=""
        zle -M ""
    fi
    zle accept-line
}
zle -N __tab_enter
bindkey '^M' __tab_enter

__tab_dismiss() {
    if (( __tab_active )); then
        __tab_active=0
        __tab_dismissed=1
        __tab_candidates=()
        __tab_clear_highlight
        POSTDISPLAY=""
        zle -M ""
    fi
}
zle -N __tab_dismiss
bindkey '\e' __tab_dismiss

__tab_reset_state() {
    __tab_active=0
    __tab_dismissed=0
    __tab_candidates=()
    __tab_sources=()
    __tab_selected=0
    __tab_clear_highlight
    POSTDISPLAY=""
}

__tab_preexec() { __tab_reset_state; }

# Without this, Ctrl-C mid-edit leaves __tab_active=1 and the next Tab inserts
# a stale candidate from the prior line.
__tab_line_init_widget() {
    __tab_reset_state
    zle -M ""
}
zle -N zle-line-init __tab_line_init_widget

autoload -Uz add-zsh-hook
add-zsh-hook preexec __tab_preexec

__tab_cleanup() {
    __tab_close_coproc
}
trap __tab_cleanup EXIT

# zsh-autosuggestions also wraps self-insert and writes POSTDISPLAY; whichever
# plugin sources last wins and the other is silently broken. Disable it.
if typeset -f _zsh_autosuggest_disable &>/dev/null; then
    _zsh_autosuggest_disable 2>/dev/null || true
fi
"#;
