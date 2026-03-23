use std::path::PathBuf;

/// Returns the directory for tab's Unix sockets: `$TMPDIR/tab-$UID/`
pub fn socket_dir() -> PathBuf {
    let tmpdir = std::env::var("TMPDIR")
        .unwrap_or_else(|_| "/tmp".into());
    let uid = unsafe { libc::getuid() };
    PathBuf::from(tmpdir).join(format!("tab-{uid}"))
}

/// Path to the shell↔daemon socket
pub fn shell_socket_path() -> PathBuf {
    socket_dir().join("shell.sock")
}

/// Path to the daemon↔overlay socket
pub fn overlay_socket_path() -> PathBuf {
    socket_dir().join("overlay.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_are_under_tmpdir() {
        let dir = socket_dir();
        assert!(dir.to_str().unwrap().contains("tab-"));

        let shell = shell_socket_path();
        assert!(shell.ends_with("shell.sock"));

        let overlay = overlay_socket_path();
        assert!(overlay.ends_with("overlay.sock"));
    }
}
