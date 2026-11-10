//! Run records (one JSON file per run) and when each routine was last
//! checked for due times.

use crate::paths::{self, Paths};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// On time.
    Scheduled,
    /// Catching up on a time Herdr was not running.
    Missed,
    /// Started by hand.
    Manual,
    /// Trying again after a failed run.
    Retry,
    /// Started because Herdr started (`@start`).
    Start,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Scheduled => "Scheduled",
            Trigger::Missed => "Catch-up",
            Trigger::Manual => "By hand",
            Trigger::Retry => "Retry",
            Trigger::Start => "Herdr start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// The tab and agent are being set up.
    Starting,
    /// The agent works on the prompt.
    Running,
    /// The agent asks a question or waits for an approval.
    NeedsYou,
    /// The agent finished its turn.
    Finished,
    /// Still not finished after the time limit; the tab stays open.
    TimedOut,
    /// The agent could not be started.
    Failed,
    /// Not started because the previous run was still going.
    Skipped,
    /// Someone closed the tab or the agent quit before it finished.
    Closed,
    /// The check command found nothing to do, so the agent did not start.
    NothingToDo,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Starting => "Starting",
            Status::Running => "Running",
            Status::NeedsYou => "Needs you",
            Status::Finished => "Finished",
            Status::TimedOut => "Timed out",
            Status::Failed => "Failed",
            Status::Skipped => "Skipped",
            Status::Closed => "Closed early",
            Status::NothingToDo => "Nothing to do",
        }
    }

    /// Whether the run still has a live agent the scheduler watches.
    pub fn active(self) -> bool {
        matches!(self, Status::Starting | Status::Running | Status::NeedsYou)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub routine_id: String,
    pub routine_name: String,
    pub agent: String,
    pub trigger: Trigger,
    /// The scheduled time it ran for (none when started by hand).
    pub slot: Option<DateTime<Local>>,
    pub started: DateTime<Local>,
    pub ended: Option<DateTime<Local>>,
    pub status: Status,
    /// What went wrong, or the last line of the agent's answer.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub agent_name: String,
    /// The agent was seen working since the prompt went in.
    #[serde(default)]
    pub seen_working: bool,
    /// When the prompt went in.
    #[serde(default)]
    pub prompted: Option<DateTime<Local>>,
    /// The agent's completed turns just before the prompt went in; one more
    /// means it finished the prompt, even if it was too quick to be seen
    /// working.
    #[serde(default)]
    pub turns_before: Option<u64>,
    /// A "needs you" notification went out for the current wait.
    #[serde(default)]
    pub notified: bool,
    /// The plugin closed the run's tab.
    #[serde(default)]
    pub tab_closed: bool,
    /// The Herdr server the run's tab is on (`paths::Paths::server_id`).
    /// Pane ids only mean something on that server.
    #[serde(default)]
    pub server: String,
    /// The agent's completed turns when the run finished; more later means
    /// someone picked up the conversation.
    #[serde(default)]
    pub turns_at_end: Option<u64>,
    /// What the run came to: the agent's summary (between the result
    /// markers in its last answer) or a shell command's last output line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    /// 0 for the first try, 1 for the first retry and so on.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Run {
    pub fn new(
        routine_id: &str,
        routine_name: &str,
        agent: &str,
        trigger: Trigger,
        slot: Option<DateTime<Local>>,
    ) -> Run {
        let started = Local::now();
        Run {
            id: format!(
                "{}-{:03}",
                started.format("%Y%m%d-%H%M%S"),
                started.timestamp_subsec_millis()
            ),
            routine_id: routine_id.into(),
            routine_name: routine_name.into(),
            agent: agent.into(),
            trigger,
            slot,
            started,
            ended: None,
            status: Status::Starting,
            message: String::new(),
            workspace_id: String::new(),
            tab_id: String::new(),
            pane_id: String::new(),
            agent_name: String::new(),
            seen_working: false,
            prompted: None,
            turns_before: None,
            notified: false,
            tab_closed: false,
            server: String::new(),
            turns_at_end: None,
            result: String::new(),
            attempt: 0,
        }
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        let path = paths
            .runs_dir(&self.routine_id)
            .join(format!("{}.json", self.id));
        let text = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        paths::write_atomic(&path, &text)
    }

    pub fn log_path(&self, paths: &Paths) -> std::path::PathBuf {
        self.file(paths, "log")
    }

    /// A file of this run next to its record, such as its log or, for a
    /// shell command, its script (`sh`), wrapper (`run`) and exit code
    /// (`exit`).
    pub fn file(&self, paths: &Paths, ext: &str) -> std::path::PathBuf {
        paths
            .runs_dir(&self.routine_id)
            .join(format!("{}.{ext}", self.id))
    }

    pub fn finish(&mut self, status: Status, message: impl Into<String>) {
        self.status = status;
        self.message = message.into();
        if self.ended.is_none() {
            self.ended = Some(Local::now());
        }
    }
}

/// The runs of one routine, newest first.
pub fn runs_of(paths: &Paths, routine_id: &str) -> Vec<Run> {
    let mut out: Vec<Run> = std::fs::read_dir(paths.runs_dir(routine_id))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a: &Run, b: &Run| b.id.cmp(&a.id));
    out
}

/// Every run of every routine, newest first.
pub fn all_runs(paths: &Paths) -> Vec<Run> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(paths.all_runs_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        if entry.path().is_dir() {
            out.extend(runs_of(paths, &entry.file_name().to_string_lossy()));
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.started));
    out
}

/// Deletes the oldest finished runs beyond `keep`.
pub fn prune(paths: &Paths, routine_id: &str, keep: usize) {
    let runs = runs_of(paths, routine_id);
    for run in runs.iter().skip(keep).filter(|r| !r.status.active()) {
        for ext in ["json", "log", "sh", "run", "exit", "out"] {
            let _ = std::fs::remove_file(run.file(paths, ext));
        }
    }
}

/// When each routine was last checked for due times.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Checked(pub BTreeMap<String, DateTime<Local>>);

impl Checked {
    pub fn load(paths: &Paths) -> Checked {
        std::fs::read(paths.checked_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        if let Ok(text) = serde_json::to_vec_pretty(self) {
            let _ = paths::write_atomic(&paths.checked_file(), &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let dir =
            std::env::temp_dir().join(format!("herdr-routines-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        }
    }

    #[test]
    fn saves_lists_and_prunes() {
        let p = temp_paths("runs");
        for i in 0..5 {
            let mut r = Run::new("a", "A", "claude", Trigger::Manual, None);
            r.id = format!("2026111{i}-080000-000");
            r.finish(Status::Finished, "ok");
            r.save(&p).unwrap();
        }
        let mut active = Run::new("a", "A", "claude", Trigger::Scheduled, None);
        active.id = "20261101-080000-000".into();
        active.status = Status::Running;
        active.save(&p).unwrap();
        assert_eq!(runs_of(&p, "a").len(), 6);
        assert_eq!(runs_of(&p, "a")[0].id, "20261114-080000-000");
        prune(&p, "a", 2);
        let left = runs_of(&p, "a");
        // Two newest plus the active one, which is never pruned.
        assert_eq!(left.len(), 3);
        assert!(left.iter().any(|r| r.status == Status::Running));
        assert_eq!(all_runs(&p).len(), 3);
    }
}
