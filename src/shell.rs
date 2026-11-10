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
    match run_full(cmd, cwd, limit) {
        Some(Ended {
            code: Some(c),
            output,
        }) => Some((c, output)),
        _ => None,
    }
}

/// How a command ended: its exit code (`None` when it was stopped at the
/// time limit) and its output so far.
pub struct Ended {
    pub code: Option<i32>,
    pub output: String,
}

/// Like `run`, but keeps the output of a command stopped at the limit.
/// None only when it could not start.
pub fn run_full(cmd: &str, cwd: Option<&Path>, limit: Duration) -> Option<Ended> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    run_program(&shell, &["-lc", &format!("exec 2>&1\n{cmd}")], cwd, limit)
}

/// A cron job's command as cron runs it: `shell -c`, in the current
/// folder, with the environment cron gave.
pub fn run_as_cron(shell: &str, cmd: &str, limit: Duration) -> Option<Ended> {
    run_program(shell, &["-c", &format!("exec 2>&1\n{cmd}")], None, limit)
}

fn run_program(program: &str, args: &[&str], cwd: Option<&Path>, limit: Duration) -> Option<Ended> {
    let mut command = Command::new(program);
    command
        .args(args)
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
                let output = reader.join().unwrap_or_default();
                return Some(Ended {
                    code: Some(status.code().unwrap_or(-1)),
                    output,
                });
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                // Children of the command may still hold the pipe open:
                // wait for the output only briefly.
                let output = wait_briefly(reader);
                return Some(Ended { code: None, output });
            }
        }
    }
}

fn wait_briefly(reader: std::thread::JoinHandle<String>) -> String {
    let start = Instant::now();
    while !reader.is_finished() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(50));
    }
    if reader.is_finished() {
        reader.join().unwrap_or_default()
    } else {
        String::new()
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
