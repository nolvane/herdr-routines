//! Routines and settings, stored as small TOML files a person or an agent
//! can edit by hand.

use crate::approval::{self, Approvals, Snapshot};
use crate::paths::{self, Paths};
use crate::schedule::Schedule;
use serde::{Deserialize, Serialize};

/// Not an agent: the routine runs a shell command or script, no AI.
pub const SHELL: &str = "shell";

/// What a routine can run, in the order the edit form lists them: the agent
/// kinds Herdr can start (`herdr agent start --kind`) and [`SHELL`].
pub const AGENTS: &[&str] = &[
    "claude",
    SHELL,
    "codex",
    "gemini",
    "cursor",
    "opencode",
    "copilot",
    "amp",
    "cline",
    "droid",
    "kiro",
    "qwen",
    "kimi",
    "grok",
    "devin",
    "pi",
    "agy",
    "omp",
    "mastracode",
    "hermes",
    "kilo",
    "qodercli",
    "letta",
    "maki",
    "muse",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Missed {
    /// After a time Herdr was not running, run once for all missed times.
    #[default]
    RunOnce,
    /// Skip missed times; wait for the next one.
    Skip,
}

impl Missed {
    pub fn label(self) -> &'static str {
        match self {
            Missed::RunOnce => "Run once",
            Missed::Skip => "Skip",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Missed::RunOnce => "One catch-up run once Herdr runs again",
            Missed::Skip => "Wait for the next time",
        }
    }
}

/// How a routine tells its owner about a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Alert {
    /// A Herdr notification when a run fails, runs too long or needs you
    /// (and when it finishes, if the settings say so).
    #[default]
    Notification,
    /// A popup in Herdr that stays until it is answered, for a run that
    /// fails, runs too long or needs you; a notification otherwise.
    PopupOnProblems,
    /// A popup for every ending run, also one that finished.
    Popup,
}

impl Alert {
    pub const ALL: [Alert; 3] = [Alert::Notification, Alert::PopupOnProblems, Alert::Popup];

    pub fn label(self) -> &'static str {
        match self {
            Alert::Notification => "Notification",
            Alert::PopupOnProblems => "Popup on problems",
            Alert::Popup => "Popup always",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Alert::Notification => "A Herdr notification when it fails or needs you",
            Alert::PopupOnProblems => "A popup that stays until OK when it fails or needs you",
            Alert::Popup => "A popup that stays until OK, also when it finishes",
        }
    }

    pub fn popup(self, problem: bool) -> bool {
        match self {
            Alert::Notification => false,
            Alert::PopupOnProblems => problem,
            Alert::Popup => true,
        }
    }
}

/// Where a shell routine runs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RunsIn {
    /// Herdr's scheduler starts it in a tab, while Herdr runs.
    #[default]
    Herdr,
    /// A line in the user's crontab starts it, through the plugin's
    /// wrapper (`crontab.rs`), also when Herdr does not run. Shell commands
    /// only, one line.
    Cron,
}

impl RunsIn {
    pub const ALL: [RunsIn; 2] = [RunsIn::Herdr, RunsIn::Cron];

    pub fn label(self) -> &'static str {
        match self {
            RunsIn::Herdr => "Herdr",
            RunsIn::Cron => "Cron",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            RunsIn::Herdr => "In a tab in Herdr, while Herdr runs",
            RunsIn::Cron => "A classic cron job, also when Herdr is closed",
        }
    }
}

fn is_herdr(r: &RunsIn) -> bool {
    *r == RunsIn::Herdr
}

fn is_notification(a: &Alert) -> bool {
    *a == Alert::Notification
}

fn yes() -> bool {
    true
}
fn default_agent() -> String {
    "claude".into()
}
fn default_folder() -> String {
    "~".into()
}
fn default_limit() -> u32 {
    120
}

/// The file `routine.toml`. The prompt sits next to it in `prompt.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutineFile {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Five-field cron in local time, such as `15 8-18 * * 1-5`.
    pub schedule: String,
    /// A Herdr agent kind, such as `claude` or `codex`.
    #[serde(default = "default_agent")]
    pub agent: String,
    /// The agent's permission mode, such as `auto` (see `agent::modes`);
    /// empty for the agent's own default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permission_mode: String,
    /// The model, such as `opus`; empty for the agent's own default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// How hard the agent thinks, such as `high`; empty for its default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    /// Extra arguments for the agent, passed after the ones above.
    #[serde(default)]
    pub args: Vec<String>,
    /// The folder the agent starts in.
    #[serde(default = "default_folder")]
    pub folder: String,
    #[serde(default)]
    pub missed: Missed,
    /// After this many minutes a run counts as timed out (its tab stays).
    #[serde(default = "default_limit")]
    pub time_limit_minutes: u32,
    /// At the time limit, stop the run (close its tab) instead of only
    /// marking it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stop_at_limit: bool,
    /// A failed run is tried again this many times...
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u32,
    /// ...this many minutes after it failed.
    #[serde(default = "default_retry_delay")]
    pub retry_delay_minutes: u32,
    /// A shell command run first, in the folder. Exit code 0: the agent
    /// runs and gets its output; anything else: nothing to do this time.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub check_command: String,
    /// Give the agent the results of earlier runs (kept in `memory.md` in
    /// the state folder, updated after each run).
    #[serde(default, skip_serializing_if = "is_false")]
    pub remember: bool,
    /// A notification, or a popup that stays until it is answered.
    #[serde(default, skip_serializing_if = "is_notification")]
    pub alert: Alert,
    /// Herdr's scheduler, or a classic cron job (shell commands only).
    #[serde(default, skip_serializing_if = "is_herdr")]
    pub runs_in: RunsIn,
    /// Runs kept: at most this many (0: no limit)...
    #[serde(default = "default_keep_runs")]
    pub keep_runs: u32,
    /// ...and none older than this many days (0: no limit).
    #[serde(default = "default_keep_days")]
    pub keep_days: u32,
}

pub const KEEP_RUNS: u32 = 30;
pub const KEEP_DAYS: u32 = 90;

fn default_keep_runs() -> u32 {
    KEEP_RUNS
}
fn default_keep_days() -> u32 {
    KEEP_DAYS
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}
fn default_retry_delay() -> u32 {
    10
}

#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    pub id: String,
    pub file: RoutineFile,
    pub prompt: String,
    /// Why the routine cannot run, if it cannot.
    pub problem: Option<String>,
    /// `routine.toml` could not be read: the other fields are stand-ins.
    pub unreadable: bool,
    /// The files are the ones the owner saved in the list or approved
    /// there; anything else waits for approval and does not run (see
    /// `approval`).
    pub approved: bool,
}

/// Whether the text holds characters that do not show on screen or show as
/// something else: control characters (line breaks and tabs are fine in a
/// multi-line text), Unicode format characters (zero-width, direction
/// overrides, soft hyphen, tag characters), variation selectors beyond
/// the two emoji ones, and private-use characters.
pub fn has_hidden(text: &str, multi_line: bool) -> bool {
    text.chars().any(|c| {
        if c.is_control() {
            return !(multi_line && matches!(c, '\n' | '\t' | '\r'));
        }
        matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200C}'
                | '\u{200E}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0D}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{E000}'..='\u{F8FF}'
                | '\u{E0000}'..='\u{E0FFF}'
                | '\u{F0000}'..='\u{10FFFF}'
        )
    })
}

/// Whether a folder name can be a routine id: lower case letters, digits
/// and dashes, as `new_id` makes them. Ids become parts of file paths and
/// of the line a shell routine's tab runs, so nothing else is allowed.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Routine {
    pub fn schedule(&self) -> Option<Schedule> {
        Schedule::parse(&self.file.schedule).ok()
    }

    pub fn schedule_text(&self) -> String {
        match Schedule::parse(&self.file.schedule) {
            Ok(s) => s.describe(),
            Err(_) => format!("Invalid: {}", self.file.schedule),
        }
    }

    /// Checks what would stop a run.
    pub fn check(file: &RoutineFile, prompt: &str) -> Option<String> {
        if file.name.trim().is_empty() {
            return Some("the name is empty".into());
        }
        // Text the owner cannot see on screen but an agent or a shell reads
        // (such as instructions hidden in Unicode tag characters) makes what
        // is shown for approval differ from what runs.
        let fields = [
            ("name", file.name.as_str()),
            ("folder", file.folder.as_str()),
            ("model", file.model.as_str()),
            ("check command", file.check_command.as_str()),
        ];
        for (what, text) in fields {
            if has_hidden(text, false) {
                return Some(format!("the {what} has hidden characters"));
            }
        }
        if file.args.iter().any(|a| has_hidden(a, false)) {
            return Some("the extra arguments have hidden characters".into());
        }
        if has_hidden(prompt, true) {
            return Some(if file.agent == SHELL {
                "the command has hidden characters".into()
            } else {
                "the prompt has hidden characters".into()
            });
        }
        if let Err(e) = Schedule::parse(&file.schedule) {
            return Some(format!("schedule: {e}"));
        }
        if file.agent == SHELL && (!file.check_command.is_empty() || file.remember) {
            return Some("a shell command takes no check command and no memory".into());
        }
        if !AGENTS.contains(&file.agent.as_str()) {
            return Some(format!("unknown agent \"{}\"", file.agent));
        }
        if let Some(problem) = crate::agent::check(file) {
            return Some(problem);
        }
        if !paths::expand(&file.folder).is_dir() {
            return Some(format!("folder {} does not exist", file.folder));
        }
        if prompt.trim().is_empty() {
            return Some(if file.agent == SHELL {
                "the command is empty".into()
            } else {
                "the prompt is empty".into()
            });
        }
        if file.runs_in == RunsIn::Cron && file.agent != SHELL {
            return Some("only a shell command can run as a cron job".into());
        }
        if file.runs_in == RunsIn::Cron && prompt.trim().contains('\n') {
            return Some("a cron job's command is one line (for more, call a script)".into());
        }
        if file.agent == SHELL && !file.args.is_empty() {
            return Some("a shell command takes no extra arguments".into());
        }
        if file.time_limit_minutes == 0 {
            return Some("the time limit is 0 minutes".into());
        }
        None
    }

    pub fn is_shell(&self) -> bool {
        self.file.agent == SHELL
    }

    /// A classic cron job: the crontab starts it, not Herdr's scheduler.
    pub fn is_cron(&self) -> bool {
        self.file.runs_in == RunsIn::Cron
    }

    /// The file next to `routine.toml` with what the routine runs: the
    /// prompt, or for a shell command the command.
    pub fn text_file(agent: &str) -> &'static str {
        if agent == SHELL {
            "command.sh"
        } else {
            "prompt.md"
        }
    }

    /// Whether the scheduler, `fire` and the list may start it.
    pub fn can_run(&self) -> bool {
        self.problem.is_none() && self.approved
    }

    pub fn load(paths: &Paths, id: &str) -> Result<Routine, String> {
        Routine::load_with(paths, id, &Approvals::load(paths))
    }

    fn load_with(paths: &Paths, id: &str, approvals: &Approvals) -> Result<Routine, String> {
        if !valid_id(id) {
            return Err(format!(
                "the folder name \"{id}\" may only have lower case letters, digits and dashes"
            ));
        }
        Routine::from_files(id, &approval::snapshot(paths, id), approvals)
    }

    /// The routine from one read of its files. Its approval is judged
    /// against exactly these bytes, so a file swapped between the check and
    /// the run cannot slip through.
    pub fn from_files(
        id: &str,
        files: &Snapshot,
        approvals: &Approvals,
    ) -> Result<Routine, String> {
        let text = files
            .get("routine.toml")
            .ok_or_else(|| "cannot read routine.toml".to_string())?;
        let file: RoutineFile =
            toml::from_str(text).map_err(|e| format!("routine.toml: {}", e.message()))?;
        let prompt = files
            .get(Routine::text_file(&file.agent))
            .cloned()
            .unwrap_or_default();
        let problem = Routine::check(&file, &prompt);
        let approved = approvals.approves(id, &file.agent, files);
        Ok(Routine {
            id: id.to_string(),
            file,
            prompt,
            problem,
            unreadable: false,
            approved,
        })
    }

    /// Writes the routine's files and returns what it wrote, so the list
    /// can approve exactly that.
    pub fn save(&self, paths: &Paths) -> Result<Snapshot, String> {
        if !valid_id(&self.id) {
            return Err(format!("invalid routine id \"{}\"", self.id));
        }
        let dir = paths.routine_dir(&self.id);
        let text = toml::to_string(&self.file).map_err(|e| e.to_string())?;
        let name = Routine::text_file(&self.file.agent);
        let what = if self.is_shell() {
            "The command"
        } else {
            "The prompt"
        };
        let header = format!("# A Herdr routine. {what} is in {name} next to this file.\n");
        let toml_text = format!("{header}{text}");
        paths::write_atomic(&dir.join("routine.toml"), toml_text.as_bytes())
            .map_err(|e| format!("cannot save: {e}"))?;
        paths::write_atomic(&dir.join(name), self.prompt.as_bytes())
            .map_err(|e| format!("cannot save {}: {e}", what.to_lowercase()))?;
        // A routine switched between agent and shell keeps one text file.
        for other in ["prompt.md", "command.sh"] {
            if other != name {
                let _ = std::fs::remove_file(dir.join(other));
            }
        }
        Ok(Snapshot::from([
            ("routine.toml".to_string(), toml_text),
            (name.to_string(), self.prompt.clone()),
        ]))
    }

    /// Saves the routine as the owner wrote it in the list and approves
    /// exactly those files.
    pub fn save_approved(&self, paths: &Paths) -> Result<(), String> {
        let written = self.save(paths)?;
        approval::approve(paths, &self.id, written)
    }

    /// Deletes a routine folder. A folder with an invalid name (shown in the
    /// list with its problem) can be deleted too, as long as the name is one
    /// plain folder inside the routines folder.
    /// `with_runs`: its run history goes too; otherwise the runs stay in
    /// Runs (marked as of a deleted routine) until deleted with another
    /// routine's history or by hand.
    pub fn delete(paths: &Paths, id: &str, with_runs: bool) -> Result<(), String> {
        use std::path::Component;
        let mut parts = std::path::Path::new(id).components();
        if !matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(_)), None)
        ) {
            return Err(format!("invalid routine id \"{id}\""));
        }
        std::fs::remove_dir_all(paths.routine_dir(id)).map_err(|e| e.to_string())?;
        if valid_id(id) {
            if with_runs {
                let _ = std::fs::remove_dir_all(paths.runs_dir(id));
            }
            let _ = std::fs::remove_file(crate::runner::memory_file(paths, id));
        }
        approval::forget(paths, id);
        Ok(())
    }
}

/// Every routine, sorted by name. Folders that do not load show up with
/// their problem, so a typo in a file is visible instead of silently ignored.
pub fn load_all(paths: &Paths) -> Vec<Routine> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(paths.routines_dir()) else {
        return out;
    };
    let approvals = Approvals::load(paths);
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if id.starts_with('.') {
            continue;
        }
        match Routine::load_with(paths, &id, &approvals) {
            Ok(r) => out.push(r),
            Err(e) => out.push(Routine {
                id: id.clone(),
                file: RoutineFile {
                    name: id.clone(),
                    enabled: false,
                    schedule: String::new(),
                    agent: default_agent(),
                    permission_mode: String::new(),
                    model: String::new(),
                    effort: String::new(),
                    args: Vec::new(),
                    folder: default_folder(),
                    missed: Missed::default(),
                    time_limit_minutes: default_limit(),
                    stop_at_limit: false,
                    retries: 0,
                    retry_delay_minutes: default_retry_delay(),
                    check_command: String::new(),
                    remember: false,
                    alert: Alert::default(),
                    runs_in: RunsIn::default(),
                    keep_runs: KEEP_RUNS,
                    keep_days: KEEP_DAYS,
                },
                prompt: String::new(),
                problem: Some(e),
                unreadable: true,
                approved: false,
            }),
        }
    }
    out.sort_by_key(|r| r.file.name.to_lowercase());
    out
}

/// A folder name for a new routine: lower case letters, digits and dashes,
/// not yet taken.
pub fn new_id(paths: &Paths, name: &str) -> String {
    let mut base = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-');
    let base: String = if base.is_empty() {
        "routine".into()
    } else {
        base.chars().take(40).collect()
    };
    let mut id = base.clone();
    let mut n = 2;
    // Kept runs of a deleted routine keep its id: a new routine never
    // takes it over.
    while paths.routine_dir(&id).exists() || paths.runs_dir(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// The settings in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The workspace the runs open in; created when missing.
    pub workspace: String,
    /// Close the tab of a finished run after this many minutes (0 = never).
    pub close_finished_after_minutes: u32,
    /// Also notify when a run finishes, not only when it needs you.
    pub notify_finished: bool,
    /// `nerd` for Nerd Font icons, `plain` for plain symbols.
    pub icons: String,
    /// Where `open` shows the list: `overlay`, `split` or `tab`.
    pub placement: String,
    /// The side of a split: `down`, `up`, `left` or `right`.
    pub split_direction: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            workspace: "Routines".into(),
            close_finished_after_minutes: 60,
            notify_finished: false,
            icons: "nerd".into(),
            placement: "overlay".into(),
            split_direction: "down".into(),
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Settings {
        std::fs::read_to_string(paths.settings_file())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> Paths {
        let dir =
            std::env::temp_dir().join(format!("herdr-routines-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        }
    }

    fn sample() -> RoutineFile {
        RoutineFile {
            name: "Morning report".into(),
            enabled: true,
            schedule: "0 8 * * 1-5".into(),
            agent: "claude".into(),
            permission_mode: "auto".into(),
            model: "opus".into(),
            effort: String::new(),
            args: vec![],
            folder: "~".into(),
            missed: Missed::RunOnce,
            time_limit_minutes: 60,
            stop_at_limit: false,
            retries: 0,
            retry_delay_minutes: 10,
            check_command: String::new(),
            remember: false,
            alert: Alert::Notification,
            runs_in: RunsIn::Herdr,
            keep_runs: KEEP_RUNS,
            keep_days: KEEP_DAYS,
        }
    }

    #[test]
    fn ids_are_unique_slugs() {
        let p = temp_paths("ids");
        assert_eq!(new_id(&p, "Morning Report!"), "morning-report");
        std::fs::create_dir_all(p.routine_dir("morning-report")).unwrap();
        assert_eq!(new_id(&p, "Morning Report"), "morning-report-2");
        assert_eq!(new_id(&p, "!!!"), "routine");
    }

    #[test]
    fn save_and_load_round_trip() {
        let p = temp_paths("roundtrip");
        let r = Routine {
            id: "morning-report".into(),
            file: sample(),
            prompt: "Write the report.".into(),
            problem: None,
            unreadable: false,
            approved: false,
        };
        r.save(&p).unwrap();
        let back = Routine::load(&p, "morning-report").unwrap();
        assert_eq!(back.file, r.file);
        assert_eq!(back.prompt, r.prompt);
        assert_eq!(back.problem, None);
        assert_eq!(load_all(&p).len(), 1);
        Routine::delete(&p, "morning-report", true).unwrap();
        assert!(load_all(&p).is_empty());
    }

    #[test]
    fn shell_routines_keep_their_command_in_command_sh() {
        let p = temp_paths("shell");
        let mut r = Routine {
            id: "backup".into(),
            file: sample(),
            prompt: "Write the report.".into(),
            problem: None,
            unreadable: false,
            approved: false,
        };
        r.save(&p).unwrap();
        r.file.agent = SHELL.into();
        r.file.permission_mode.clear();
        r.file.model.clear();
        r.prompt = "./backup.sh --all".into();
        r.save(&p).unwrap();
        let dir = p.routine_dir("backup");
        assert!(dir.join("command.sh").exists());
        assert!(!dir.join("prompt.md").exists(), "the prompt file goes");
        let back = Routine::load(&p, "backup").unwrap();
        assert_eq!(back.prompt, "./backup.sh --all");
        assert_eq!(back.problem, None);
    }

    #[test]
    fn only_routines_saved_in_the_list_or_approved_run() {
        let p = temp_paths("approval");
        let r = Routine {
            id: "morning-report".into(),
            file: sample(),
            prompt: "Write the report.".into(),
            problem: None,
            unreadable: false,
            approved: false,
        };
        // Written by something else (an agent): waits for approval.
        r.save(&p).unwrap();
        let back = Routine::load(&p, "morning-report").unwrap();
        assert!(!back.approved && !back.can_run());
        // Saved in the list: approved.
        r.save_approved(&p).unwrap();
        assert!(Routine::load(&p, "morning-report").unwrap().can_run());
        // An agent raises the permission mode: waits again.
        let mut raised = r.clone();
        raised.file.permission_mode = "bypassPermissions".into();
        raised.save(&p).unwrap();
        assert!(!load_all(&p)[0].approved);
        // Deleting forgets the approval.
        Routine::delete(&p, "morning-report", true).unwrap();
        assert!(Approvals::load(&p).0.is_empty());
    }

    #[test]
    fn hidden_characters_are_a_problem() {
        let mut f = sample();
        assert_eq!(Routine::check(&f, "Line one\n\tLine two 👍🏽 ❤️"), None);
        // Instructions hidden in Unicode tag characters.
        let tags: String = "ignore"
            .chars()
            .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
            .collect();
        assert!(Routine::check(&f, &format!("Summarise{tags}")).is_some());
        assert!(Routine::check(&f, "Run \u{202E}txt.sh").is_some());
        assert!(Routine::check(&f, "a\u{200B}b").is_some());
        assert!(Routine::check(&f, "a\u{1b}[2Jb").is_some());
        f.args = vec!["--x\u{200F}".into()];
        assert!(Routine::check(&f, "go").is_some());
        f = sample();
        f.name = "Report\u{FE01}".into();
        assert!(Routine::check(&f, "go").is_some());
    }

    #[test]
    fn folder_names_must_be_plain_ids() {
        assert!(valid_id("morning-report-2"));
        for bad in ["", "..", "a/b", "Big", "a b", "a'b", "-x", "a\\b", "ä"] {
            assert!(!valid_id(bad), "{bad}");
        }
        let p = temp_paths("names");
        std::fs::create_dir_all(p.routine_dir("My_Routine")).unwrap();
        let all = load_all(&p);
        assert!(all[0].unreadable && all[0].problem.is_some());
        assert!(Routine::load(&p, "../../etc").is_err());
        assert!(Routine::delete(&p, "..", true).is_err());
        assert!(Routine::delete(&p, "/tmp", true).is_err());
        Routine::delete(&p, "My_Routine", true).unwrap();
    }

    #[test]
    fn minimal_file_gets_defaults() {
        let f: RoutineFile = toml::from_str("name = \"x\"\nschedule = \"0 9 * * *\"").unwrap();
        assert!(f.enabled);
        assert_eq!(f.agent, "claude");
        assert_eq!(f.missed, Missed::RunOnce);
        let f: RoutineFile =
            toml::from_str("name = \"x\"\nschedule = \"0 9 * * *\"\nmissed = \"skip\"").unwrap();
        assert_eq!(f.missed, Missed::Skip);
    }

    #[test]
    fn checks_find_problems() {
        let mut f = sample();
        assert_eq!(Routine::check(&f, "go"), None);
        assert!(Routine::check(&f, " ").is_some());
        f.schedule = "nope".into();
        assert!(Routine::check(&f, "go").unwrap().starts_with("schedule"));
        f = sample();
        f.agent = "robot".into();
        assert!(Routine::check(&f, "go").is_some());
        f = sample();
        f.folder = "/does/not/exist/here".into();
        assert!(Routine::check(&f, "go").is_some());
    }
}
