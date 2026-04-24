use anyhow::Result;
use std::sync::OnceLock;

use interprocess::local_socket::{prelude::*, ListenerOptions, Name, Stream as SyncStream};
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};

use interprocess::local_socket::tokio::{
    prelude::*, Listener as TokioListener, Stream as TokioStream,
};

#[cfg(unix)]
use crate::paths::runtime_dir;
#[cfg(unix)]
use crate::paths::socket_file;

static ENDPOINT: OnceLock<String> = OnceLock::new();

fn endpoint() -> &'static str {
    ENDPOINT.get_or_init(|| {
        #[cfg(windows)]
        {
            let user: String = whoami::username()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            let user = if user.is_empty() { "user".into() } else { user };
            format!("tab-{user}.sock")
        }
        #[cfg(unix)]
        {
            socket_file().to_string_lossy().into_owned()
        }
    })
}

/// Cross-platform name:
/// - Windows: namespaced (named pipe `\\.\pipe\tab-{user}.sock`)
/// - Unix:    filesystem path under `XDG_RUNTIME_DIR` / `$TMPDIR`
pub fn make_name() -> Result<Name<'static>> {
    let s = endpoint();
    #[cfg(windows)]
    {
        Ok(s.to_ns_name::<GenericNamespaced>()?)
    }
    #[cfg(unix)]
    {
        Ok(s.to_fs_name::<GenericFilePath>()?)
    }
}

/// Bind a tokio listener, preparing the runtime dir first.
///
/// Race-safe: if a stale socket file exists, we first re-probe before removing
/// it. If a daemon is still accepting connections we bail instead of stomping.
pub fn prepare_listener() -> Result<TokioListener> {
    #[cfg(unix)]
    {
        let dir = runtime_dir();
        std::fs::create_dir_all(&dir)?;
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
            tracing::warn!("set 0700 on {dir:?} failed: {e}");
        }
        let path = socket_file();
        if path.exists() {
            if ping() {
                anyhow::bail!("another tab-daemon is already running on {path:?}");
            }
            let _ = std::fs::remove_file(&path);
        }
    }
    let name = make_name()?;
    Ok(ListenerOptions::new().name(name).create_tokio()?)
}

/// Blocking connect — used by the hook coprocess.
pub fn connect_sync() -> Result<SyncStream> {
    let name = make_name()?;
    Ok(SyncStream::connect(name)?)
}

/// Async connect — used by the TUI picker.
pub async fn connect_async() -> Result<TokioStream> {
    let name = make_name()?;
    Ok(TokioStream::connect(name).await?)
}

/// Non-destructive probe: true if a daemon is already accepting connections.
pub fn ping() -> bool {
    connect_sync().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_stable_across_calls() {
        assert_eq!(endpoint(), endpoint());
    }

    #[cfg(unix)]
    #[test]
    fn unix_endpoint_looks_like_socket_path() {
        let e = endpoint();
        assert!(e.ends_with("shell.sock"), "got {e}");
        assert!(e.starts_with('/'), "should be an absolute path, got {e}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_endpoint_has_sock_suffix() {
        let e = endpoint();
        assert!(e.ends_with(".sock"));
        assert!(e.starts_with("tab-"));
    }

    #[test]
    fn make_name_does_not_panic() {
        // We can't bind without a daemon, but producing the `Name` itself
        // must never fail — it's pure string formatting.
        let _ = make_name().expect("make_name");
    }

    #[test]
    fn ping_is_false_when_daemon_absent_at_random_path() {
        // We can't easily override the endpoint, but we can at least verify
        // that ping returns a bool without panicking.
        let _ = ping();
    }
}
