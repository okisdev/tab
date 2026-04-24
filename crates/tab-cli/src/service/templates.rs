//! Pure content generators for the per-OS install artifacts.
//!
//! These functions are not `cfg`-gated — they compile on every platform so
//! their output is unit-testable from any host. On a given target only one
//! of them ends up being *called* by the corresponding `service/*.rs` module;
//! the unused ones are kept live for cross-platform testing.
#![allow(dead_code)]

use std::path::Path;

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn launchd_plist(label: &str, daemon: &Path, log_dir: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon}</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>{log}/daemon-crash.log</string>
    <key>StandardErrorPath</key><string>{log}/daemon-crash.log</string>
    <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
        label = xml_escape(label),
        daemon = xml_escape(&daemon.display().to_string()),
        log = xml_escape(&log_dir.display().to_string()),
    )
}

pub fn systemd_unit(daemon: &Path) -> String {
    format!(
        r#"[Unit]
Description=tab autocomplete daemon
After=default.target

[Service]
Type=simple
ExecStart={daemon}
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target
"#,
        daemon = daemon.display()
    )
}

pub fn windows_vbs(daemon: &Path) -> String {
    // VBS strings quote " as "". Use CRLF line endings for Windows.
    let path = daemon.display().to_string().replace('"', "\"\"");
    format!(
        "Set WshShell = CreateObject(\"WScript.Shell\")\r\nWshShell.Run \"\"\"{path}\"\"\", 0, False\r\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn xml_escape_every_special() {
        assert_eq!(xml_escape("&"), "&amp;");
        assert_eq!(xml_escape("<"), "&lt;");
        assert_eq!(xml_escape(">"), "&gt;");
        assert_eq!(xml_escape("\""), "&quot;");
        assert_eq!(xml_escape("'"), "&apos;");
        assert_eq!(xml_escape("A & B < C > D"), "A &amp; B &lt; C &gt; D");
    }

    #[test]
    fn xml_escape_preserves_safe_chars() {
        assert_eq!(xml_escape("hello world"), "hello world");
        assert_eq!(xml_escape("/path/with/slashes"), "/path/with/slashes");
        assert_eq!(xml_escape("日本語"), "日本語");
    }

    #[test]
    fn launchd_plist_well_formed_xml() {
        let plist = launchd_plist(
            "com.tab.daemon",
            Path::new("/usr/local/bin/tab-daemon"),
            Path::new("/var/log"),
        );
        assert!(plist.contains("<?xml version=\"1.0\""));
        assert!(plist.contains("<plist"));
        assert!(plist.contains("</plist>"));
        assert!(plist.contains("<string>com.tab.daemon</string>"));
        assert!(plist.contains("<string>/usr/local/bin/tab-daemon</string>"));
        assert!(plist.contains("/var/log/daemon-crash.log"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        assert!(plist.contains("<key>KeepAlive</key><true/>"));
    }

    #[test]
    fn launchd_plist_escapes_ampersand_in_path() {
        // Rare but legal on macOS.
        let daemon = PathBuf::from("/Users/me & you/tab-daemon");
        let plist = launchd_plist("com.tab.daemon", &daemon, Path::new("/tmp"));
        assert!(
            plist.contains("/Users/me &amp; you/tab-daemon"),
            "plist should XML-escape `&`"
        );
        assert!(!plist.contains("/Users/me & you/"));
    }

    #[test]
    fn launchd_plist_escapes_angle_brackets() {
        let daemon = PathBuf::from("/tmp/<weird>/tab-daemon");
        let plist = launchd_plist("com.tab.daemon", &daemon, Path::new("/tmp"));
        assert!(plist.contains("/tmp/&lt;weird&gt;/tab-daemon"));
    }

    #[test]
    fn systemd_unit_includes_required_sections() {
        let unit = systemd_unit(Path::new("/home/u/.local/bin/tab-daemon"));
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("ExecStart=/home/u/.local/bin/tab-daemon"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn windows_vbs_uses_crlf() {
        let vbs = windows_vbs(Path::new(r"C:\Users\me\tab-daemon.exe"));
        assert!(vbs.contains("\r\n"), "VBS conventionally uses CRLF");
        assert!(vbs.contains("WScript.Shell"));
        assert!(vbs.contains("WshShell.Run"));
        // Third arg `False` means don't wait for completion.
        assert!(vbs.contains("False"));
        // Path embedded with VBS double-quote escaping.
        assert!(vbs.contains(r#""""C:\Users\me\tab-daemon.exe"""#));
    }

    #[test]
    fn windows_vbs_escapes_embedded_quote() {
        let vbs = windows_vbs(Path::new(r#"C:\weird"path\tab.exe"#));
        // Every " becomes "" within the string.
        assert!(vbs.contains(r#"""""#), "double-quote must be escaped");
    }
}
