//! Starting the Herdr server at login, so routines run after a restart
//! without anyone opening Herdr. The plugin's startup hook then starts the
//! scheduler with the server.
//!
//! macOS: a launch agent in `~/Library/LaunchAgents`; Linux: a systemd user
//! service. Turning it on only writes (and on Linux enables) the entry: it
//! takes effect at the next login, so the server running now is left alone.
//! Turning it off removes the entry and never stops a running server.

use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "herdr-routines.herdr-server";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
}

/// Where the entry lives, None where autostart is not supported.
pub fn entry(home: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(
            home.join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        )
    } else if cfg!(target_os = "linux") {
        Some(
            home.join(".config/systemd/user")
                .join(format!("{LABEL}.service")),
        )
    } else {
        None
    }
}

pub fn supported() -> bool {
    entry(&home()).is_some()
}

pub fn is_on() -> bool {
    entry(&home()).is_some_and(|p| p.is_file())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launch agent: `herdr server` at login, with the PATH and shell of
/// now (launchd's own PATH lacks Homebrew and the like).
fn plist(herdr: &str, path: &str, shell: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by herdr-routines: starts the Herdr server at login. Remove it, or turn it off in the list, to stop. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>server</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{}</string>
    <key>SHELL</key>
    <string>{}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#,
        xml(herdr),
        xml(path),
        xml(shell)
    )
}

fn unit(herdr: &str, path: &str, shell: &str) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    format!(
        "# Written by herdr-routines: starts the Herdr server at login.\n[Unit]\nDescription=Herdr server (for herdr-routines)\n\n[Service]\nExecStart={} server\nEnvironment={}\nEnvironment={}\nRestart=no\n\n[Install]\nWantedBy=default.target\n",
        q(herdr),
        q(&format!("PATH={path}")),
        q(&format!("SHELL={shell}")),
    )
}

/// Writes the entry for `herdr` (an absolute path).
pub fn turn_on(herdr: &str) -> Result<(), String> {
    if !Path::new(herdr).is_absolute() {
        return Err("cannot find the herdr program".into());
    }
    let file = entry(&home()).ok_or("not supported on this system")?;
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let text = if cfg!(target_os = "macos") {
        plist(herdr, &path, &shell)
    } else {
        unit(herdr, &path, &shell)
    };
    std::fs::create_dir_all(file.parent().expect("has a parent")).map_err(|e| e.to_string())?;
    crate::paths::write_atomic(&file, text.as_bytes()).map_err(|e| e.to_string())?;
    if cfg!(target_os = "linux") {
        let ok = Command::new("systemctl")
            .args(["--user", "enable", &format!("{LABEL}.service")])
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            let _ = std::fs::remove_file(&file);
            return Err("systemctl --user enable failed".into());
        }
    }
    Ok(())
}

/// Removes the entry. A server started by it keeps running.
pub fn turn_off() -> Result<(), String> {
    let file = entry(&home()).ok_or("not supported on this system")?;
    if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", &format!("{LABEL}.service")])
            .output();
    }
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_name_herdr_server() {
        let p = plist("/opt/herdr & co/bin/herdr", "/usr/bin:/bin", "/bin/zsh");
        assert!(p.contains(
            "<string>/opt/herdr &amp; co/bin/herdr</string>\n    <string>server</string>"
        ));
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(p.contains("<key>KeepAlive</key>\n  <false/>"));
        let u = unit("/usr/bin/herdr", "/usr/bin", "/bin/bash");
        assert!(u.contains("ExecStart=\"/usr/bin/herdr\" server"));
        assert!(entry(Path::new("/h")).is_some());
    }
}
