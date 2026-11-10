//! Running a command line in the person's login shell (where their tools
//! are on the PATH), with a time limit, outside any tab.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs `cmd` in `cwd` and returns its exit code and output (standard
/// output and errors together). None when it could not start or took longer
/// than `limit` (it is stopped then).
pub fn run(cmd: &str, cwd: Option<&Path>, limit: Duration) -> Option<(i32, String)> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut command = Command::new(shell);
    command
        .args(["-lc", &format!("exec 2>&1\n{cmd}")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().ok()?;
    // Read in a thread, so a long output cannot fill the pipe and stall.
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = reader.join().unwrap_or_default();
                return Some((status.code().unwrap_or(-1), text));
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_output_and_limit() {
        let tmp = std::env::temp_dir();
        let (code, out) = run(
            "echo out; echo err >&2; exit 3",
            Some(&tmp),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(code, 3);
        assert!(out.contains("out") && out.contains("err"));
        assert!(run("sleep 5", None, Duration::from_millis(300)).is_none());
    }
}
