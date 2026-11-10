//! Model names an agent prints itself (`agent::list_command`), kept in the
//! state folder so the form never waits for them. A refresh runs in the
//! background: daily, or hourly while the agent printed nothing (not
//! installed, not logged in).

use crate::agent;
use crate::paths::{self, Paths};
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// How long a list command may take.
const LIMIT: Duration = Duration::from_secs(30);

fn file(paths: &Paths, agent: &str) -> PathBuf {
    paths.state.join("models").join(format!("{agent}.txt"))
}

/// The names last printed by the agent, empty when none are known.
pub fn cached(paths: &Paths, agent: &str) -> Vec<String> {
    std::fs::read_to_string(file(paths, agent))
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Starts a refresh in the background when the list is missing or old.
pub fn refresh(paths: &Paths, agent: &str) {
    static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Some(cmd) = agent::list_command(agent) else {
        return;
    };
    let path = file(paths, agent);
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());
    let empty = cached(paths, agent).is_empty();
    let max_age = Duration::from_secs(if empty { 3600 } else { 86_400 });
    if age.is_some_and(|a| a < max_age) {
        return;
    }
    {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if !running
            .get_or_insert_with(HashSet::new)
            .insert(agent.to_string())
        {
            return;
        }
    }
    let agent = agent.to_string();
    std::thread::spawn(move || {
        let names = run(cmd)
            .map(|out| agent::parse_models(&out))
            .unwrap_or_default();
        // Keep a good list when the agent printed nothing this time.
        if !names.is_empty() || std::fs::metadata(&path).is_err() || empty {
            let _ = std::fs::create_dir_all(path.parent().expect("has a parent"));
            let _ = paths::write_atomic(&path, names.join("\n").as_bytes());
        } else {
            // Touch it, so the next try waits.
            let _ = paths::write_atomic(&path, std::fs::read(&path).unwrap_or_default().as_slice());
        }
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = running.as_mut() {
            r.remove(&agent);
        }
    });
}

/// Runs a command line in the person's login shell (where the agents are
/// on the PATH) and returns what it printed, or None on failure or when it
/// took too long.
fn run(cmd: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(shell)
        .args(["-lc", cmd])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read in a thread, so a long list cannot fill the pipe and stall.
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = reader.join().unwrap_or_default();
                return status.success().then_some(text);
            }
            Ok(None) if start.elapsed() < LIMIT => std::thread::sleep(Duration::from_millis(100)),
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
    fn reads_the_cached_list() {
        let dir = std::env::temp_dir().join(format!("rt-models-{}", std::process::id()));
        let paths = Paths {
            config: dir.join("c"),
            state: dir.join("s"),
            socket: dir.join("sock"),
        };
        assert!(cached(&paths, "opencode").is_empty());
        std::fs::create_dir_all(paths.state.join("models")).unwrap();
        std::fs::write(file(&paths, "opencode"), "a/one\n\nb/two\n").unwrap();
        assert_eq!(cached(&paths, "opencode"), vec!["a/one", "b/two"]);
        assert_eq!(run("printf 'x-1\\ny-2\\n'").as_deref(), Some("x-1\ny-2\n"));
        assert_eq!(run("exit 3"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
