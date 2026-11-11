//! Running other programs with a time limit, outside any tab: a command
//! line in the person's login shell (where their tools are on the PATH), a
//! cron job's command as cron runs it, and the tools the plugin calls
//! (`herdr`, `crontab`).
//!
//! A command runs in its own process group, so the time limit stops the
//! command and everything it started, not only its shell. Standard output
//! and errors share one pipe, joined by this process, so no shell has to
//! understand `2>&1`.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of a command's output is kept in memory from its start.
pub const HEAD_LIMIT: usize = 256 * 1024;
/// How much is kept from its end (for the last line).
const TAIL_LIMIT: usize = 16 * 1024;
/// The most a called tool may print.
const TOOL_LIMIT: u64 = 16 * 1024 * 1024;

/// Runs `cmd` in `cwd` and returns its exit code and output (standard
/// output and errors together). None when it could not start or took longer
/// than `limit` (it is stopped then).
pub fn run(cmd: &str, cwd: Option<&Path>, limit: Duration) -> Option<(i32, String)> {
    match run_full(cmd, cwd, limit) {
        Some(Ended {
            code: Some(c),
            output,
            ..
        }) => Some((c, output)),
        _ => None,
    }
}

/// How a command ended: its exit code (`None` when it was stopped at the
/// time limit) and its output so far.
pub struct Ended {
    pub code: Option<i32>,
    /// The start of the output, at most [`HEAD_LIMIT`] bytes.
    pub output: String,
    /// The end of the output (the same text when it is short).
    pub tail: String,
}

/// Like `run`, but keeps the output of a command stopped at the limit.
/// None only when it could not start.
pub fn run_full(cmd: &str, cwd: Option<&Path>, limit: Duration) -> Option<Ended> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    run_program(
        &shell,
        &[OsStr::new("-lc"), OsStr::new(cmd)],
        cwd,
        limit,
        Stdio::null(),
        None,
    )
}

/// A cron job's command as cron runs it: `shell -c`, in the current
/// folder, with the environment cron gave. `stdin` is what cron feeds the
/// command (the text after a `%` in the crontab line). All output also
/// goes to `sink` as it comes, so nothing has to be held in memory.
pub fn run_as_cron(
    shell: &str,
    cmd: &OsStr,
    limit: Duration,
    stdin: Stdio,
    sink: Option<Box<dyn Write + Send>>,
) -> Option<Ended> {
    run_program(shell, &[OsStr::new("-c"), cmd], None, limit, stdin, sink)
}

/// The start and the end of what a command printed.
#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: Vec<u8>,
    cut: bool,
}

impl Capture {
    fn add(&mut self, bytes: &[u8]) {
        let room = HEAD_LIMIT.saturating_sub(self.head.len());
        self.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
        if bytes.len() > room {
            self.cut = true;
        }
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > 2 * TAIL_LIMIT {
            let drop = self.tail.len() - TAIL_LIMIT;
            self.tail.drain(..drop);
        }
    }

    fn texts(&self) -> (String, String) {
        let head = String::from_utf8_lossy(&self.head).into_owned();
        if !self.cut {
            return (head.clone(), head);
        }
        let from = self.tail.len().saturating_sub(TAIL_LIMIT);
        (
            head,
            String::from_utf8_lossy(&self.tail[from..]).into_owned(),
        )
    }
}

/// Stops a process group for good.
fn kill_group(pid: u32) {
    // The child was started as the leader of its own group (`process_group(0)`),
    // so its pid is the group's id and no other process is in it.
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        if pid > 1 {
            // SAFETY: `killpg` takes plain integers and touches no memory.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }
}

fn run_program(
    program: &str,
    args: &[&OsStr],
    cwd: Option<&Path>,
    limit: Duration,
    stdin: Stdio,
    mut sink: Option<Box<dyn Write + Send>>,
) -> Option<Ended> {
    let (mut reader, writer) = std::io::pipe().ok()?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(stdin)
        .stdout(writer.try_clone().ok()?)
        .stderr(writer)
        .process_group(0);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().ok()?;
    // This process's copies of the pipe's write end: the pipe ends when the
    // command (and what it started) is done.
    drop(command);
    // Read in a thread, so a long output cannot fill the pipe and stall.
    let capture = Arc::new(Mutex::new(Capture::default()));
    let shared = Arc::clone(&capture);
    let collector = std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(s) = sink.as_mut() {
                        // A sink that fails (a full disk) is given up; the
                        // command still runs to its end.
                        if s.write_all(&buf[..n]).and_then(|_| s.flush()).is_err() {
                            sink = None;
                        }
                    }
                    shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&buf[..n]);
                }
            }
        }
    });
    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.code().unwrap_or(-1)),
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Something the command left running may still hold the pipe open:
    // wait for the rest of the output only briefly.
    let waited = Instant::now();
    while !collector.is_finished() && waited.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let (output, tail) = capture.lock().unwrap_or_else(|e| e.into_inner()).texts();
    Some(Ended { code, output, tail })
}

/// What a called tool answered.
#[derive(Debug)]
pub struct ToolOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs a tool the plugin depends on (`herdr`, `crontab`) and waits at most
/// `limit` for it, so a tool that hangs cannot hang the scheduler or the
/// list with it. An error when it could not start or took too long (it is
/// stopped then).
pub fn tool(mut command: Command, limit: Duration) -> std::io::Result<ToolOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    drop(command);
    // Read in threads into shared buffers: what came is there even when
    // something the tool started keeps a pipe open after it.
    type Bytes = Arc<Mutex<Vec<u8>>>;
    let read = |pipe: Option<Box<dyn Read + Send>>| -> (Bytes, std::thread::JoinHandle<()>) {
        let bytes = Bytes::default();
        let shared = Arc::clone(&bytes);
        let reader = std::thread::spawn(move || {
            let Some(mut pipe) = pipe else { return };
            let mut buf = vec![0u8; 64 * 1024];
            let mut left = TOOL_LIMIT;
            while left > 0 {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        left = left.saturating_sub(n as u64);
                        shared
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .extend_from_slice(&buf[..n]);
                    }
                }
            }
        });
        (bytes, reader)
    };
    let (out, out_reader) = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let (err, err_reader) = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(10)),
            None => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("no answer within {} seconds", limit.as_secs()),
                ));
            }
        }
    };
    let waited = Instant::now();
    while !(out_reader.is_finished() && err_reader.is_finished())
        && waited.elapsed() < Duration::from_secs(2)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let taken =
        |bytes: Bytes| std::mem::take(&mut *bytes.lock().unwrap_or_else(|e| e.into_inner()));
    Ok(ToolOutput {
        success: status.success(),
        stdout: taken(out),
        stderr: taken(err),
    })
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

    #[test]
    fn the_limit_stops_what_the_command_started_too() {
        let dir = std::env::temp_dir().join(format!("rt-shell-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker");
        let script = dir.join("job.sh");
        std::fs::write(&script, format!("sleep 1\ntouch '{}'\n", marker.display())).unwrap();
        // The shell stays (something follows the job), so the job is a
        // process of its own.
        let cmd = format!("/bin/sh '{}'; echo done", script.display());
        let ended = run_as_cron(
            "/bin/sh",
            OsStr::new(&cmd),
            Duration::from_millis(200),
            Stdio::null(),
            None,
        )
        .unwrap();
        assert!(ended.code.is_none(), "stopped at the limit");
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "the job itself was stopped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_are_joined_without_shell_text_and_input_is_passed_on() {
        let dir = std::env::temp_dir().join(format!("rt-shell-csh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // csh reads `2>&1` as "into a file named 1": nothing like it is
        // sent to the shell any more.
        if Path::new("/bin/csh").exists() {
            let cmd = format!("cd '{}'; echo out; ls /does/not/exist/here", dir.display());
            let ended = run_as_cron(
                "/bin/csh",
                OsStr::new(&cmd),
                Duration::from_secs(10),
                Stdio::null(),
                None,
            )
            .unwrap();
            assert!(ended.output.contains("out"));
            assert!(ended.output.contains("does/not/exist"), "{}", ended.output);
            assert!(!dir.join("1").exists());
        }
        // What cron feeds a command (the text after `%`) reaches it, and
        // the output goes to the sink as it comes.
        let input = dir.join("in.txt");
        std::fs::write(&input, "fed by cron\n").unwrap();
        let out = dir.join("out.txt");
        let sink = std::fs::File::create(&out).unwrap();
        let ended = run_as_cron(
            "/bin/sh",
            OsStr::new("cat; echo last"),
            Duration::from_secs(10),
            Stdio::from(std::fs::File::open(&input).unwrap()),
            Some(Box::new(sink)),
        )
        .unwrap();
        assert_eq!(ended.code, Some(0));
        assert_eq!(ended.output, "fed by cron\nlast\n");
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "fed by cron\nlast\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_output_is_kept_at_both_ends_only() {
        let ended = run_as_cron(
            "/bin/sh",
            OsStr::new("i=0; while [ $i -lt 20000 ]; do echo \"line $i of many many many many\"; i=$((i+1)); done; echo the end"),
            Duration::from_secs(60),
            Stdio::null(),
            None,
        )
        .unwrap();
        assert_eq!(ended.code, Some(0));
        assert_eq!(ended.output.len(), HEAD_LIMIT);
        assert!(ended.output.starts_with("line 0 of"));
        assert!(ended.tail.len() <= TAIL_LIMIT);
        assert!(ended.tail.ends_with("the end\n"));
    }

    #[test]
    fn a_tool_that_hangs_is_given_up() {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "sleep 5"]);
        let e = tool(c, Duration::from_millis(200)).err().unwrap();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "echo a; echo b >&2; exit 2"]);
        let o = tool(c, Duration::from_secs(10)).unwrap();
        assert!(!o.success);
        assert_eq!(
            (o.stdout.as_slice(), o.stderr.as_slice()),
            (&b"a\n"[..], &b"b\n"[..])
        );
    }
}
