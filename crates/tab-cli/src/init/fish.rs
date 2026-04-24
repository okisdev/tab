pub const SCRIPT: &str = r#"
# tab - terminal autocomplete plugin (fish)
# Install:
#   tab init fish | source
# Persist:
#   tab init fish > ~/.config/fish/conf.d/tab.fish

set -g __tab_bin (command -v tab)
if test -z "$__tab_bin"
    set -g __tab_bin tab
end

function __tab_complete
    set -l line (commandline)
    set -l selected ($__tab_bin complete --buffer "$line" --cwd (pwd) 2>/dev/null | string join \n)
    if test $status -eq 0 -a -n "$selected"
        commandline -r -- "$selected"
        commandline -f end-of-line
    end
end

# Bind Tab in both default and vi-insert maps unconditionally. Fish preserves
# both so the binding still fires after the user switches keymaps later.
bind \t __tab_complete 2>/dev/null; or true
bind -M insert \t __tab_complete 2>/dev/null; or true
"#;
