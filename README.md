# tab

cross-platform terminal autocomplete with fuzzy history matching. three-process split:

- `tab-daemon` — long-lived background process. watches `~/.zsh_history` / `~/.bash_history` / fish history / PSReadLine history, indexes commands with a nucleo fuzzy matcher, and serves completion queries over a local socket (unix socket on macOS/linux, named pipe on windows).
- `tab hook` — per-shell coprocess that bridges stdin/stdout line protocol to the daemon.
- `tab complete` — an interactive picker (invoked from a shell widget) for shells that can't render ghost text.

zsh gets the full ghost-text experience. bash / fish / pwsh get an in-terminal picker bound to the Tab key.

## quick start

```sh
# install via Homebrew (macOS)
brew install okisdev/tap/tab
# or build from source:
cargo install --path crates/tab-cli --path crates/tab-daemon
# or: cargo build --release   # then put ./target/release/tab{,-daemon} on PATH

# install the background service and print shell hints
tab install
# (Homebrew users can instead run: brew services start okisdev/tap/tab)

# add the shell integration (see per-shell section below)
```

`tab install` auto-detects your OS and registers:

- macOS: launchd agent `~/Library/LaunchAgents/com.tab.daemon.plist`
- Linux: systemd user unit `~/.config/systemd/user/tab.service`
- Windows: startup shortcut `…\Start Menu\Programs\Startup\tab-daemon.vbs` (no console window)

## shell integration

### zsh (full ghost-text UX)

```sh
# persist — add to ~/.zshrc
eval "$(tab init zsh)"
```

widgets wrapped: `self-insert`, `backward-delete-char`, `bracketed-paste`, `yank`, etc.
key bindings:
- Tab / → — accept highlighted candidate
- ↑ / ↓ — navigate candidates
- Enter — execute the buffer as typed (use Tab / → first to accept a candidate)
- Esc — dismiss

### bash (picker)

```sh
# persist — add to ~/.bashrc
eval "$(tab init bash)"
```

Tab opens a picker. requires readline (enabled by default in interactive bash ≥ 4).

### fish (picker)

one-shot:

```fish
tab init fish | source
```

persistent:

```fish
tab init fish > ~/.config/fish/conf.d/tab.fish
```

### PowerShell / pwsh (picker)

add to `$PROFILE` (find its path with `echo $PROFILE`):

```powershell
tab init pwsh | Out-String | Invoke-Expression
```

requires the `PSReadLine` module (bundled with PowerShell 5.1+ and all pwsh 7.x).

## platform details

| OS | IPC | service manager | history sources auto-detected |
|---|---|---|---|
| macOS   | unix socket under `$XDG_RUNTIME_DIR/tab-<uid>/` if set, else `$TMPDIR/tab-<uid>/` | launchd user agent | `~/.zsh_history`, `~/.bash_history`, `~/Library/Application Support/fish/fish_history`, `~/.local/share/powershell/PSReadLine/ConsoleHost_history.txt` |
| Linux   | unix socket under `$XDG_RUNTIME_DIR/tab-<uid>/`, falls back to `$TMPDIR/tab-<uid>/` | systemd user unit | `~/.zsh_history`, `~/.bash_history`, `~/.local/share/fish/fish_history`, `~/.local/share/powershell/PSReadLine/ConsoleHost_history.txt` |
| Windows | named pipe `\\.\pipe\tab-<user>.sock` | startup shortcut (no admin) | `%APPDATA%\Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt` plus any unix-style history files you have in `%USERPROFILE%` |

the daemon loads every history file that exists; you can override the set via `config.toml` (see below).

## configuration

config lives at:

- Linux:   `~/.config/tab/config.toml`
- macOS:   `~/Library/Application Support/tab/config.toml`
- Windows: `%APPDATA%\tab\config.toml`

logs live at:

- Linux:   `~/.local/share/tab/logs/`
- macOS:   `~/Library/Application Support/tab/logs/`
- Windows: `%APPDATA%\tab\logs\`

defaults are written on first `tab install`. run `tab settings` for an interactive editor, or edit the file directly:

```toml
[completion]
max_results = 8
match_mode = "fuzzy"   # or "prefix"

[log]
level = ""             # "" = component default; or one of error/warn/info/debug/trace

[history]
sources = ["auto"]     # or any subset of ["zsh","bash","fish","pwsh"]
# optional explicit paths:
# zsh_path  = "/path/to/zsh_history"
# bash_path = "/path/to/bash_history"
# fish_path = "/path/to/fish_history"
# pwsh_path = "/path/to/ConsoleHost_history.txt"
```

override the log level without editing config via the `TAB_LOG` env var (accepts `tracing` EnvFilter syntax, e.g. `TAB_LOG=debug`).

## CLI reference

```
tab init <shell>         emit shell integration script (zsh | bash | fish | pwsh)
tab hook                 coprocess bridge; called by the shell
tab complete --buffer … --cwd …
                         interactive picker; prints selection on stdout
tab start                run the daemon in the foreground
tab status               daemon + service-manager status
tab install              register auto-start and launch the daemon
tab uninstall            stop + deregister
tab settings             interactive config editor
tab doctor               environment diagnostic
tab logs [component] [-f] [-n N]
                         components: daemon | hook | all
```

## troubleshooting

- **zsh ghost text lagging / duplicated** — `tab logs daemon -f` while typing; each keystroke should log a query. stale responses are filtered by buffer-echo correlation.
- **bash picker does nothing** — ensure `bind -x` is available (`set +o vi` / not in POSIX mode).
- **fish picker conflicts with native autosuggest** — fish runs both concurrently; tab's picker takes Tab, fish keeps → for its own ghost text.
- **pwsh handler overrides PSReadLine's completion menu** — the script falls back to `TabCompleteNext` when the daemon returns nothing.
- **daemon not running** — `tab status`. if the socket file exists but connections fail, the daemon probably crashed: `tab logs daemon -n 200`.
- **permission denied on unix socket** — the runtime dir is created with mode `0700`; if you run across multiple users, set `$TMPDIR` per user.

## build from source

```sh
cargo build --release
# produces target/release/tab and target/release/tab-daemon
```

requires rust 1.75+.

### minimum shell versions

| shell | minimum | notes |
|---|---|---|
| zsh  | 5.3 | needs `POSTDISPLAY` for ghost text |
| bash | 4.0 | `bind -x` / `READLINE_LINE` — macOS default is 3.2; `brew install bash` |
| fish | 3.0 | any modern fish |
| pwsh | 5.1 | PSReadLine 2.0+ bundled with PowerShell 5.1 and pwsh 7.x |

cross-compilation:

```sh
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin x86_64-unknown-linux-gnu
cargo build --release --target <triple>
```

## architecture

```
       keystroke                    JSONL over local socket
  shell ────────► tab hook ────────────────────────────────► tab-daemon
     ▲  (zsh ZLE)  (coprocess)                                 │
     │                                                         │
     │             \x1f-separated display lines                ▼
     └─────────────────────────────── candidates ◄─── history + scripts + paths
```

- protocol: newline-delimited JSON. request = `{"buffer","cwd","match_mode"}`, response = `{"candidates":[{"text","score","match_positions","source"}]}`.
- correlation: hook echoes the request buffer as the first `\x1f`-separated field; zsh drops mismatches to avoid rendering stale results after a burst of keystrokes.
- history indexing: `nucleo-matcher` (Rust port of FZF's scorer) with a weighted score of fuzzy quality, frequency, recency (14d half-life), and exact-prefix bonus.

## license

MIT
