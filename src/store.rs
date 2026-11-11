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

    /// A record of a time at which nothing ran (the previous run was still
    /// going, or the check found nothing to do). These are kept apart from
    /// the runs when the history is pruned, so a row of them never pushes
    /// out the runs that have output.
    pub fn note(self) -> bool {
        matches!(self, Status::Skipped | Status::NothingToDo)
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
    /// The terminal Herdr made for the run's pane. Unlike pane and tab ids,
    /// which a restarted Herdr hands out again, it is random, so it tells
    /// whether a pane id still means the run's own pane.
    #[serde(default)]
    pub terminal_id: String,
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
    /// A run of a cron job: the pid of its wrapper process, which owns the
    /// record (it has no tab, and Herdr's scheduler leaves it alone).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cron_pid: u32,
    /// When that wrapper process started, as `ps` prints it. A pid alone
    /// is handed out again; together with its start time it names one
    /// process (`scheduler::alive_since`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cron_started: String,
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
            terminal_id: String::new(),
            seen_working: false,
            prompted: None,
            turns_before: None,
            notified: false,
            tab_closed: false,
            server: String::new(),
            turns_at_end: None,
            result: String::new(),
            attempt: 0,
            cron_pid: 0,
            cron_started: String::new(),
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

    /// How Herdr calls reach the run's agent: by its unique name, so a
    /// prompt or a state check never lands on another agent that now sits
    /// in a pane with the same id. Older records have only the pane.
    pub fn agent_target(&self) -> &str {
        if self.agent_name.is_empty() {
            &self.pane_id
        } else {
            &self.agent_name
        }
    }

    pub fn finish(&mut self, status: Status, message: impl Into<String>) {
        self.status = status;
        self.message = message.into();
        if self.ended.is_none() {
            self.ended = Some(Local::now());
        }
    }
}

/// Whether a run id has the form `Run::new` gives it (digits and dashes).
fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// The runs of one routine, newest first. A record counts only when it
/// names the folder and file it is in, since its routine and run ids
/// become the paths of its log, scripts and later writes and deletes.
pub fn runs_of(paths: &Paths, routine_id: &str) -> Vec<Run> {
    if !crate::routine::valid_id(routine_id) {
        return Vec::new();
    }
    let mut out: Vec<Run> = std::fs::read_dir(paths.runs_dir(routine_id))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let bytes = paths::read_limited(&e.path(), paths::FILE_LIMIT).ok()?;
            let run: Run = serde_json::from_slice(&bytes).ok()?;
            let stem = e.path().file_stem()?.to_string_lossy().to_string();
            (run.routine_id == routine_id && run.id == stem && valid_run_id(&run.id)).then_some(run)
        })
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

/// Keeps a routine's newest `keep_runs` runs, none older than `keep_days`
/// days (0: no limit for either). Runs still going always stay. Notes of
/// times at which nothing ran (`Status::note`) are counted on their own,
/// with the same limits, so they never push a real run out.
pub fn prune(paths: &Paths, routine_id: &str, keep_runs: u32, keep_days: u32) {
    let runs = runs_of(paths, routine_id);
    // A number of days too large for the calendar means no age limit.
    let oldest = (keep_days > 0)
        .then(|| chrono::Duration::try_days(i64::from(keep_days)))
        .flatten()
        .and_then(|d| chrono::Local::now().checked_sub_signed(d));
    let (mut real, mut notes) = (0usize, 0usize);
    for run in &runs {
        let seen = if run.status.note() {
            &mut notes
        } else {
            &mut real
        };
        *seen += 1;
        let too_many = keep_runs > 0 && *seen > keep_runs as usize;
        let too_old = oldest.is_some_and(|t| run.started < t);
        if (too_many || too_old) && !run.status.active() {
            remove(paths, run);
        }
    }
}

/// Deletes a run's record and files; the folder goes with its last run.
pub fn remove(paths: &Paths, run: &Run) {
    for ext in ["json", "log", "sh", "run", "exit", "out", "prompt"] {
        let _ = std::fs::remove_file(run.file(paths, ext));
    }
    // Only empties: `remove_dir` refuses a folder with files left.
    let _ = std::fs::remove_dir(paths.runs_dir(&run.routine_id));
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
        prune(&p, "a", 2, 0);
        let left = runs_of(&p, "a");
        // Two newest plus the active one, which is never pruned.
        assert_eq!(left.len(), 3);
        assert!(left.iter().any(|r| r.status == Status::Running));
        assert_eq!(all_runs(&p).len(), 3);
        // By age: a run older than the days kept goes, whatever the count.
        let mut old = Run::new("b", "B", "shell", Trigger::Scheduled, None);
        old.id = "20200101-080000-000".into();
        old.started = chrono::Local::now() - chrono::Duration::days(100);
        old.finish(Status::Finished, "");
        old.save(&p).unwrap();
        let mut new = Run::new("b", "B", "shell", Trigger::Scheduled, None);
        new.id = "20261101-090000-000".into();
        new.finish(Status::Finished, "");
        new.save(&p).unwrap();
        prune(&p, "b", 0, 90);
        let left = runs_of(&p, "b");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "20261101-090000-000");
    }

    #[test]
    fn notes_do_not_push_runs_out_and_huge_limits_do_not_panic() {
        let p = temp_paths("notes");
        let mk = |id: &str, status: Status| {
            let mut run = Run::new("a", "A", "claude", Trigger::Scheduled, None);
            run.id = id.into();
            run.finish(status, "");
            run.save(&p).unwrap();
        };
        mk("20261101-080000-000", Status::Finished);
        mk("20261101-081000-000", Status::Failed);
        for i in 0..5 {
            mk(&format!("20261101-09{i:02}00-000"), Status::Skipped);
            mk(&format!("20261101-10{i:02}00-000"), Status::NothingToDo);
        }
        prune(&p, "a", 2, 90);
        let left = runs_of(&p, "a");
        let real = left.iter().filter(|r| !r.status.note()).count();
        assert_eq!(real, 2, "both real runs stay");
        assert_eq!(left.len(), 4, "and the two newest notes");
        // Limits far outside the calendar: no panic, nothing deleted by age.
        for days in [100_000_000, u32::MAX] {
            prune(&p, "a", 0, days);
        }
        assert_eq!(runs_of(&p, "a").len(), 4);
    }

    #[test]
    fn records_that_point_elsewhere_are_ignored() {
        let p = temp_paths("forged");
        let mut ok = Run::new("a", "A", "claude", Trigger::Manual, None);
        ok.id = "20261110-080000-000".into();
        ok.save(&p).unwrap();
        // A record whose ids would lead its writes and deletes elsewhere.
        let mut forged = ok.clone();
        forged.routine_id = "../../config".into();
        forged.id = "../x".into();
        let text = serde_json::to_vec(&forged).unwrap();
        paths::write_atomic(&p.runs_dir("a").join("20261110-090000-000.json"), &text).unwrap();
        // A record copied under another name.
        paths::write_atomic(
            &p.runs_dir("a").join("20261110-100000-000.json"),
            &serde_json::to_vec(&ok).unwrap(),
        )
        .unwrap();
        let runs = runs_of(&p, "a");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, ok.id);
        assert!(runs_of(&p, "../a").is_empty());
    }
}
