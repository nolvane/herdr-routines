//! The system clipboard for the prompt editor, through the usual command
//! line tools (`pbcopy` on macOS, `wl-copy` or `xclip` on Linux). Without
//! one, copy and paste still work inside the list.

use std::io::Write;
use std::process::{Command, Stdio};

const COPY: &[&[&str]] = &[
    &["pbcopy"],
    &["wl-copy"],
    &["xclip", "-selection", "clipboard"],
];
const PASTE: &[&[&str]] = &[
    &["pbpaste"],
    &["wl-paste", "--no-newline"],
    &["xclip", "-selection", "clipboard", "-o"],
];

/// Puts text on the system clipboard. False when no tool worked.
pub fn set(text: &str) -> bool {
    COPY.iter().any(|cmd| {
        let Ok(mut child) = Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|s| s.success()) && wrote
    })
}

/// The text on the system clipboard, None when no tool worked.
pub fn get() -> Option<String> {
    PASTE.iter().find_map(|cmd| {
        let out = Command::new(cmd[0])
            .args(&cmd[1..])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}
