//! The list's state and what the keys and the mouse do. Drawing is in
//! `ui.rs`.

use crate::agent;
use crate::clipboard;
use crate::editor::TextArea;
use crate::form::{Field, Form, OTHER_MODEL};
use crate::models;
use crate::paths::{self, Paths};
use crate::routine::{self, Routine, Settings};
use crate::runner;
use crate::scheduler;
use crate::store::{self, Run};
use crate::theme::{Icons, ACCENT, BLUE, GREEN, MUTED, RED, YELLOW};
use crate::ui;
use chrono::{DateTime, Local};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::DefaultTerminal;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RELOAD: Duration = Duration::from_secs(2);
const FLASH: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Routines Herdr runs.
    Routines,
    /// Classic cron jobs: this plugin's and the rest of the crontab.
    Cron,
    Runs,
}

impl View {
    pub const ORDER: [View; 3] = [View::Routines, View::Cron, View::Runs];

    pub fn name(self) -> &'static str {
        match self {
            View::Routines => "Routines",
            View::Cron => "Cron jobs",
            View::Runs => "Runs",
        }
    }

    /// The next (or previous) tab.
    fn step(self, by: i32) -> View {
        let i = View::ORDER.iter().position(|v| *v == self).unwrap_or(0) as i32;
        View::ORDER[(i + by).rem_euclid(View::ORDER.len() as i32) as usize]
    }

    /// A list of routines (with or without cron jobs), not of runs.
    pub fn lists_routines(self) -> bool {
        self != View::Runs
    }
}

/// A clickable area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    View(View),
    Row(usize),
    /// The runs link of a routine row: shows that routine's runs.
    RunsOf(usize),
    PickRow(usize),
    /// A field of the form, by its place in `Form::fields`.
    Field(usize),
    /// A key, as if pressed.
    Key(char),
    /// A key with Ctrl, as if pressed.
    Ctrl(char),
    /// The text of the prompt editor.
    Text,
    /// The repository link in the keys overlay's bottom border.
    Link,
    /// An entry of the right-click menu.
    MenuRow(usize),
    /// The ‹ or › beside a choice in the form: the field and the step.
    Cycle(usize, i8),
    /// A way to take over cron jobs, in the dialog that offers them.
    Take(usize),
    /// A weekday switch in the form (Monday is 0).
    Day(usize),
    /// The ✕ of a filter of the Runs tab.
    Unfilter(RunFilter),
    /// A part of the line above the routines, such as "2 went wrong".
    Attention(Attention),
}

/// The filters of the Runs tab, which narrow it together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFilter {
    /// Only the runs of one routine.
    Routine,
    /// Only runs that went wrong or need the owner.
    Problems,
    /// Only runs with the searched text.
    Search,
}

/// What a routine wants from its owner, most pressing first. The list is
/// sorted by it, and the line above the routines counts each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Attention {
    /// Its run waits for an answer.
    NeedsYou,
    /// Its last run went wrong.
    WentWrong,
    /// New or changed outside the list: it waits for approval.
    ToApprove,
    /// Something in its files keeps it from running.
    CannotRun,
    /// A run of it is going.
    Running,
    /// On, with a next time (or none, such as "when Herdr starts").
    Waiting,
    Off,
}

/// How the routines are sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// What needs the owner first, then by next time, off last.
    #[default]
    Attention,
    Name,
}

impl Sort {
    fn word(self) -> &'static str {
        match self {
            Sort::Attention => "attention",
            Sort::Name => "name",
        }
    }
}

/// What a list to choose from is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickFor {
    /// `n`: how to create a routine.
    New,
    /// A field of the form.
    Field(Field),
    /// When the computer is kept awake.
    Awake,
}

/// Runs the system's opener (first argument) on a web link, true when it
/// could be started. Replaceable, so tests open nothing.
pub type Opener = fn(&str, &str) -> bool;

const OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub title: String,
    pub items: Vec<crate::form::Item>,
    pub sel: usize,
    pub purpose: PickFor,
    /// A note under the entries, about all of them.
    pub hint: String,
    /// Typed to narrow a long list (such as the models an agent lists).
    pub filter: String,
    /// Every entry; `items` is what the filter leaves.
    pub all: Vec<crate::form::Item>,
    /// The first entry shown, kept so the list only scrolls when the
    /// marked entry would leave the window.
    pub top: usize,
}

impl Picker {
    pub fn new(
        title: String,
        items: Vec<crate::form::Item>,
        sel: usize,
        purpose: PickFor,
    ) -> Picker {
        Picker {
            title,
            all: items.clone(),
            items,
            sel,
            purpose,
            hint: String::new(),
            filter: String::new(),
            top: 0,
        }
    }

    /// Long lists are narrowed by typing instead of picked by number.
    pub fn filters(&self) -> bool {
        self.all.len() > 12
    }

    fn refilter(&mut self) {
        let f = self.filter.to_lowercase();
        let keep = |i: &crate::form::Item| {
            i.value == OTHER_MODEL
                || i.value.to_lowercase().contains(&f)
                || i.label.to_lowercase().contains(&f)
        };
        self.items = self.all.iter().filter(|i| keep(i)).cloned().collect();
        self.sel = 0;
        self.top = 0;
    }
}

/// The choices behind `n`.
const BY_FORM: &str = "form";
const BY_AGENT: &str = "agent";
const BY_CRON: &str = "cron";

#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub title: String,
    pub hint: String,
    pub text: String,
    pub purpose: InputFor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputFor {
    /// A model name for the form.
    Model,
    /// The text the Runs tab is searched for.
    Search,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmFor {
    Run(String),
    /// A routine; true when it has runs (then `r` deletes them too).
    Delete(String, bool),
    /// One run: routine id and run id.
    DeleteRun(String, String),
    /// Ending a run that is going: routine id and run id.
    StopRun(String, String),
    DiscardForm,
    /// What was written in the prompt editor since it opened.
    DiscardEditor,
    /// Taking over cron jobs in a way that changes the crontab.
    Import(Vec<crate::crontab::CronJob>, crate::crontab::Take),
    Autostart,
    AutostartOff,
    RoutinesOn,
    RoutinesOff,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub question: String,
    pub lines: Vec<String>,
    pub purpose: ConfirmFor,
}

impl Confirm {
    /// What `y` does, for its button.
    pub fn yes(&self) -> &'static str {
        match &self.purpose {
            ConfirmFor::Run(_) => "run",
            ConfirmFor::Delete(_, true) => "delete, keep runs",
            ConfirmFor::Delete(_, false) | ConfirmFor::DeleteRun(..) => "delete",
            ConfirmFor::StopRun(..) => "stop",
            ConfirmFor::DiscardForm | ConfirmFor::DiscardEditor => "discard",
            ConfirmFor::Import(_, crate::crontab::Take::ToHerdr) => "move",
            ConfirmFor::Import(..) => "take over",
            ConfirmFor::Autostart | ConfirmFor::RoutinesOn => "turn on",
            ConfirmFor::AutostartOff | ConfirmFor::RoutinesOff => "turn off",
        }
    }

    /// What any other key does, for its button.
    pub fn no(&self) -> &'static str {
        match &self.purpose {
            ConfirmFor::DiscardForm | ConfirmFor::DiscardEditor => "keep editing",
            _ => "no",
        }
    }

    /// Something is lost for good: its button is red, and ⏎ does not
    /// confirm it.
    pub fn danger(&self) -> bool {
        matches!(
            self.purpose,
            ConfirmFor::Delete(..)
                | ConfirmFor::DeleteRun(..)
                | ConfirmFor::StopRun(..)
                | ConfirmFor::DiscardForm
                | ConfirmFor::DiscardEditor
        )
    }

    /// Whether ⏎ confirms like `y`. Deleting, discarding, stopping a run
    /// and running by hand ask for `y` itself.
    pub fn enter_confirms(&self) -> bool {
        !self.danger() && !matches!(self.purpose, ConfirmFor::Run(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogView {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: u16,
    /// How far it can scroll, set when drawn.
    pub max: u16,
    /// The file the output comes from, which `f` opens.
    pub file: Option<std::path::PathBuf>,
    /// The approval view (`a`): the routine and exactly the files shown,
    /// approved with `y` once read to the end.
    pub approve: Option<(String, crate::approval::Snapshot)>,
    /// A line that stays above the scrolled text: how a run went and why.
    pub note: String,
}

/// One run on a page of its own: how it went, its result and the end of
/// its output. The run itself is read from the list each time it is drawn,
/// so a run that is going shows how it goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPage {
    pub routine_id: String,
    pub run_id: String,
    /// The end of the run's output, read when the page opened and again
    /// while the run is going.
    pub tail: Vec<String>,
    /// The output is longer than `tail`.
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Help,
    /// A run's page (⏎ on a run).
    RunPage(RunPage),
    Pick(Picker),
    Input(Input),
    Confirm(Confirm),
    /// A note to read, closed with Enter or Esc.
    Message {
        title: String,
        lines: Vec<String>,
    },
    Log(LogView),
    /// The jobs in the person's crontab, to move or copy here.
    Cron(Vec<crate::crontab::CronJob>),
    /// The right-click menu of a row.
    Menu(Menu),
    AskQuit,
}

/// A right-click menu: what can be done with the row it was opened on,
/// with the key that does the same.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Where it was opened (its top left corner, moved to fit).
    pub x: u16,
    pub y: u16,
    pub items: Vec<MenuItem>,
    pub sel: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// The key shown beside it, such as "⏎" or "space".
    pub key: &'static str,
    pub action: MenuAction,
    /// Shown in red: deletes something (it still asks first).
    pub danger: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAction {
    /// The same as pressing this key in the list.
    Key(char),
    /// Takes over the person's own cron job under the cursor.
    Take(crate::crontab::Take),
}

/// What the prompt editor writes to when saved.
#[derive(Debug, Clone, PartialEq)]
pub enum EditTarget {
    /// A routine's prompt or command: its id and the file (`prompt.md` or
    /// `command.sh`).
    Prompt(String, &'static str),
    /// A routine's `routine.toml`, when it cannot be read.
    File(String),
    /// The prompt of the open form.
    Form,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub text: TextArea,
    pub target: EditTarget,
    /// What is edited, for the header.
    pub title: String,
    /// The text when the editor opened, which discarding goes back to.
    pub original: String,
}

/// A message in the footer.
#[derive(Debug, Clone, PartialEq)]
pub struct Flash {
    pub text: String,
    pub color: Color,
    /// When it goes by itself; a problem (red or yellow) stays until the
    /// next key or click instead.
    until: Instant,
    pub sticky: bool,
}

/// The ways to take over cron jobs, in the order the dialog lists them:
/// its key, what it is called and what it means.
pub const TAKES: [(char, crate::crontab::Take, &str, &str); 3] = [
    (
        'k',
        crate::crontab::Take::KeepCron,
        "Keep as a cron job",
        "Cron still runs it, also when Herdr is closed; you see, edit and switch it here, with its output in Runs",
    ),
    (
        'm',
        crate::crontab::Take::ToHerdr,
        "Move to Herdr",
        "Herdr runs it in a tab instead; its crontab line is commented out",
    ),
    (
        'c',
        crate::crontab::Take::Copy,
        "Copy to Herdr, switched off",
        "A routine with the same command and schedule; your crontab stays as it is",
    ),
];

/// How the list works, shown on first open and under `?`.
pub const INTRO: [&str; 5] = [
    "A routine runs on a schedule, in its own tab in the workspace Routines: a coding agent that gets your prompt, or a shell command without AI, like a cron job. You can watch each run and step in when it asks something.",
    "Routines run while Herdr runs, also with no window open. When Herdr or the computer was off at the time, a routine runs once afterwards or skips that time, as you choose.",
    "The permission mode decides what the agent may do without asking. Auto suits most routines; with Ask every time, a run waits for you.",
    "The tab of a finished run closes after an hour. Runs lists every run, and l shows what it wrote.",
    "? shows all keys and this note again.",
];

pub fn ago(t: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let m = (*now - *t).num_minutes();
    match m {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{m} min ago"),
        60..=1439 => format!("{} h ago", m / 60),
        _ => format!("{} d ago", m / 1440),
    }
}

pub fn until(t: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let m = (*t - *now).num_minutes();
    if m < 60 {
        format!("in {} min", m.max(1))
    } else {
        crate::cron::short_time(t, now)
    }
}

/// How long a run took; for one that is going, how long it has been at
/// work (without the time the computer slept).
pub fn took(run: &Run) -> String {
    span_words(match run.ended {
        Some(end) => (end - run.started).num_seconds(),
        None => run.awake_for(Local::now()).num_seconds(),
    })
}

/// A length of time in words: "40 s", "12 min", "2 h 5 min".
pub fn span_words(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

pub struct App {
    pub paths: Paths,
    pub icons: Icons,
    pub view: View,
    pub routines: Vec<Routine>,
    /// The runs the Runs view shows: all, or those of `run_filter`.
    pub runs: Vec<Run>,
    /// Every run of every routine, newest first.
    pub all_runs: Vec<Run>,
    /// The routine whose runs the Runs view shows; None for all.
    pub run_filter: Option<String>,
    /// The Runs view shows only runs that went wrong or need the owner.
    pub runs_problems: bool,
    /// The Runs view shows only runs with this text (in the routine's
    /// name, the message or the result); empty for all.
    pub runs_search: String,
    /// How the routines are sorted, kept across restarts.
    pub sort: Sort,
    /// When the computer is kept awake.
    pub awake: crate::awake::Mode,
    /// Routines that wait for a free place, with the time they run for.
    pub waiting: BTreeMap<String, DateTime<Local>>,
    /// Routines whose runs went wrong in a row, with how many.
    pub failing: HashMap<String, u32>,
    pub sel_routine: usize,
    /// The cursor in the Cron jobs view.
    pub sel_cron: usize,
    pub sel_run: usize,
    pub scheduler: Option<(u32, String)>,
    pub mode: Mode,
    pub form: Option<Box<Form>>,
    pub editor: Option<Editor>,
    pub help_scroll: u16,
    pub help_max: u16,
    pub flash: Option<Flash>,
    /// The first row each list shows (routines, cron jobs, runs). The
    /// wheel changes it; the cursor moves it only to stay in view.
    pub tops: [usize; 3],
    /// The cursor moved by key: the list scrolls to show it when drawn.
    pub follow: bool,
    /// Where the mouse is, for marking the row under it.
    pub mouse: Option<(u16, u16)>,
    /// The open dialog's place: a click beside it closes it.
    pub dialog: Option<Rect>,
    /// The tab the runs filter was set from; Esc goes back there.
    pub filter_from: Option<View>,
    /// Routines that were approved at least once: one that waits again
    /// was changed, any other is new.
    pub known: HashSet<String>,
    /// The marked way in the dialog that takes over cron jobs.
    pub take_sel: usize,
    /// The size the list was last drawn at.
    pub size: (u16, u16),
    pub quit: bool,
    pub hits: Vec<(Rect, Hit)>,
    pub opener: Opener,
    /// The editor's text should go to the person's own editor.
    external: bool,
    /// The person's own cron jobs (not this plugin's), shown under the
    /// routines, and when the crontab was read last.
    pub crontab_jobs: Vec<crate::crontab::CronJob>,
    crontab_read: Option<Instant>,
    last_load: Instant,
    last_click: Option<(Instant, Hit)>,
    /// What was copied last, for pasting when there is no system clipboard.
    clip: String,
    /// Ask about starting at login once the intro is closed.
    autostart_after_intro: bool,
    /// The routine the form or editor was opened on: its files as read
    /// then, and whether they were the approved ones. Saving approves what
    /// it writes only when both still hold (`opened_approved`).
    opened: Option<(String, crate::approval::Snapshot, bool)>,
    /// The accounts of the Multiaccount plugin, when it is installed.
    pub accounts: Option<Vec<crate::accounts::Account>>,
}

impl App {
    pub fn new(paths: Paths) -> App {
        let settings = Settings::load(&paths);
        let mut app = App {
            paths,
            icons: Icons {
                nerd: settings.icons != "plain",
            },
            view: View::Routines,
            routines: Vec::new(),
            runs: Vec::new(),
            all_runs: Vec::new(),
            run_filter: None,
            runs_problems: false,
            runs_search: String::new(),
            sort: Sort::default(),
            awake: crate::awake::Mode::default(),
            waiting: BTreeMap::new(),
            failing: HashMap::new(),
            sel_routine: 0,
            sel_cron: 0,
            sel_run: 0,
            scheduler: None,
            mode: Mode::Normal,
            form: None,
            editor: None,
            help_scroll: 0,
            help_max: 0,
            flash: None,
            tops: [0; 3],
            follow: true,
            mouse: None,
            dialog: None,
            filter_from: None,
            known: HashSet::new(),
            take_sel: 0,
            size: (0, 0),
            quit: false,
            hits: Vec::new(),
            opener: open_in_browser,
            external: false,
            crontab_jobs: Vec::new(),
            crontab_read: None,
            last_load: Instant::now(),
            last_click: None,
            clip: String::new(),
            autostart_after_intro: false,
            opened: None,
            accounts: None,
        };
        app.sort = match std::fs::read_to_string(app.sort_file()) {
            Ok(text) if text.trim() == Sort::Name.word() => Sort::Name,
            _ => Sort::Attention,
        };
        app.reload();
        app
    }

    pub fn reload(&mut self) {
        // The cursor stays on its routine, cron job or run, wherever a
        // reload puts it: new runs and renamed routines move the rows.
        let on = self.cursor_keys();
        // The crontab changes rarely: read every 15 seconds, or after a
        // change made here (which clears `crontab_read`). Its managed jobs
        // come first, as hand edits there win.
        let crontab_due = self
            .crontab_read
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(15));
        if crontab_due {
            self.cron_pull();
        }
        self.routines = routine::load_all(&self.paths);
        self.known = crate::approval::Approvals::load(&self.paths)
            .0
            .into_keys()
            .collect();
        self.all_runs = store::all_runs(&self.paths);
        // A filter stays while its routine or any of its runs is there
        // (runs can outlive a deleted routine).
        if let Some(id) = &self.run_filter {
            if !self.routines.iter().any(|r| &r.id == id)
                && !self.all_runs.iter().any(|r| &r.routine_id == id)
            {
                self.run_filter = None;
                self.filter_from = None;
            }
        }
        self.runs = self.shown_runs();
        self.waiting = scheduler::waiting_for_a_place(&self.paths);
        // How many runs went wrong in a row, for those whose last one did.
        self.failing = self
            .routines
            .iter()
            .filter(|r| {
                self.latest_run(&r.id)
                    .is_some_and(|run| run.status.problem())
            })
            .map(|r| (r.id.clone(), runner::failures_in_a_row(&self.paths, &r.id)))
            .filter(|(_, n)| *n >= 2)
            .collect();
        if self.sort == Sort::Attention {
            // What needs the owner first; the names keep the rest in order.
            let now = Local::now();
            let mut keyed: Vec<((Attention, i64), Routine)> = std::mem::take(&mut self.routines)
                .into_iter()
                .map(|r| (self.attention_key(&r, &now), r))
                .collect();
            keyed.sort_by_key(|(key, _)| *key);
            self.routines = keyed.into_iter().map(|(_, r)| r).collect();
        }
        self.awake = crate::awake::mode(&self.paths);
        self.refresh_run_page();
        self.scheduler = scheduler::running(&self.paths);
        self.accounts = crate::accounts::list(&self.paths);
        if crontab_due {
            self.crontab_jobs = crate::crontab::read()
                .map(|t| crate::crontab::parse(&t))
                .unwrap_or_default();
            self.crontab_read = Some(Instant::now());
        }
        self.restore_cursors(on);
        let herdr = self.routines.iter().filter(|r| !r.is_cron()).count();
        let cron = self.routines.len() - herdr + self.crontab_jobs.len();
        self.sel_routine = self.sel_routine.min(herdr.saturating_sub(1));
        self.sel_cron = self.sel_cron.min(cron.saturating_sub(1));
        self.sel_run = self.sel_run.min(self.runs.len().saturating_sub(1));
        self.last_load = Instant::now();
        self.fill_models();
    }

    /// The runs the Runs view shows: all, narrowed by its filters.
    fn shown_runs(&self) -> Vec<Run> {
        let text = self.runs_search.trim().to_lowercase();
        self.all_runs
            .iter()
            .filter(|r| {
                self.run_filter
                    .as_ref()
                    .is_none_or(|id| &r.routine_id == id)
            })
            .filter(|r| {
                !self.runs_problems || r.status.problem() || r.status == store::Status::NeedsYou
            })
            .filter(|r| {
                text.is_empty()
                    || [&r.routine_name, &r.message, &r.result]
                        .iter()
                        .any(|part| part.to_lowercase().contains(&text))
            })
            .cloned()
            .collect()
    }

    /// Whether the Runs view is narrowed by any filter.
    pub fn runs_filtered(&self) -> bool {
        self.run_filter.is_some() || self.runs_problems || !self.runs_search.trim().is_empty()
    }

    /// What a routine wants from its owner (see [`Attention`]).
    pub fn attention(&self, r: &Routine) -> Attention {
        let status = self.latest_run(&r.id).map(|run| run.status);
        if status == Some(store::Status::NeedsYou) {
            Attention::NeedsYou
        } else if status.is_some_and(|s| s.problem()) {
            Attention::WentWrong
        } else if r.problem.is_some() {
            Attention::CannotRun
        } else if !r.approved {
            Attention::ToApprove
        } else if status.is_some_and(|s| s.active()) {
            Attention::Running
        } else if r.file.enabled {
            Attention::Waiting
        } else {
            Attention::Off
        }
    }

    /// When a routine that is on and can run comes next; none for one
    /// without a next time (such as "when Herdr starts").
    pub fn next_time(&self, r: &Routine, now: &DateTime<Local>) -> Option<DateTime<Local>> {
        if !r.file.enabled || !r.can_run() {
            return None;
        }
        match r.schedule() {
            Some(crate::schedule::Schedule::AtStart) | None => None,
            Some(s) => s.next_after(now),
        }
    }

    /// The place of a routine in the list sorted by what needs the owner:
    /// the kind, then the next time (those without one come after).
    fn attention_key(&self, r: &Routine, now: &DateTime<Local>) -> (Attention, i64) {
        let kind = self.attention(r);
        let next = match kind {
            Attention::Waiting => self.next_time(r, now).map_or(i64::MAX, |t| t.timestamp()),
            _ => 0,
        };
        (kind, next)
    }

    /// The line above the routines: what needs the owner, what is going
    /// and what comes next. Only the parts that are not zero.
    pub fn attention_parts(&self) -> Vec<(Attention, String, Color)> {
        let count = |kind: Attention| {
            self.routines
                .iter()
                .filter(|r| self.attention(r) == kind)
                .count()
        };
        let mut v = Vec::new();
        let mut part = |kind: Attention, one: &str, many: &str, color: Color| {
            let n = count(kind);
            if n > 0 {
                v.push((
                    kind,
                    format!("{n} {}", if n == 1 { one } else { many }),
                    color,
                ));
            }
        };
        part(Attention::NeedsYou, "needs you", "need you", YELLOW);
        part(Attention::WentWrong, "went wrong", "went wrong", RED);
        part(Attention::ToApprove, "to approve", "to approve", YELLOW);
        part(Attention::CannotRun, "cannot run", "cannot run", RED);
        part(Attention::Running, "running", "running", BLUE);
        if let Some((r, when)) = self.next_up() {
            v.push((
                Attention::Waiting,
                format!("next: {} {when}", r.file.name),
                MUTED,
            ));
        }
        v
    }

    /// The routine that runs next, and when ("in 9 min", "Thu 02:30").
    pub fn next_up(&self) -> Option<(&Routine, String)> {
        let now = Local::now();
        self.routines
            .iter()
            .filter(|r| !self.waiting.contains_key(&r.id))
            .filter_map(|r| self.next_time(r, &now).map(|t| (t, r)))
            .min_by_key(|(t, _)| *t)
            .map(|(t, r)| (r, until(&t, &now)))
    }

    /// A click on a part of that line: the runs that went wrong, or the
    /// first routine the part counts.
    fn attention_click(&mut self, kind: Attention) {
        if kind == Attention::WentWrong {
            self.run_filter = None;
            self.filter_from = None;
            self.runs_search.clear();
            self.runs_problems = true;
            self.view = View::Runs;
            self.sel_run = 0;
            self.tops[2] = 0;
            self.follow = true;
            return self.reload();
        }
        let id = if kind == Attention::Waiting {
            self.next_up().map(|(r, _)| r.id.clone())
        } else {
            self.routines
                .iter()
                .find(|r| self.attention(r) == kind)
                .map(|r| r.id.clone())
        };
        if let Some(id) = id {
            self.select_id(&id);
        }
    }

    fn sort_file(&self) -> std::path::PathBuf {
        self.paths.state.join("list-sort")
    }

    /// `s`: the routines by what needs the owner first, or by name. The
    /// choice is kept for the next time the list opens.
    fn toggle_sort(&mut self) {
        self.sort = match self.sort {
            Sort::Attention => Sort::Name,
            Sort::Name => Sort::Attention,
        };
        let saved = paths::write_atomic(&self.sort_file(), self.sort.word().as_bytes());
        self.reload();
        self.follow = true;
        match (self.sort, saved) {
            (_, Err(e)) => self.flash(format!("Sorted, but the choice was not saved: {e}"), YELLOW),
            (Sort::Attention, _) => self.flash("Sorted by what needs you first", ACCENT),
            (Sort::Name, _) => self.flash("Sorted by name", ACCENT),
        }
    }

    /// What each list's cursor is on: the routine's id, the cron job's id
    /// (or, for one only in the crontab, its line) and the run.
    fn cursor_keys(&self) -> [Option<String>; 3] {
        let routine = self
            .routines
            .iter()
            .filter(|r| !r.is_cron())
            .nth(self.sel_routine)
            .map(|r| r.id.clone());
        let managed: Vec<&Routine> = self.routines.iter().filter(|r| r.is_cron()).collect();
        let cron = match managed.get(self.sel_cron) {
            Some(r) => Some(format!("id:{}", r.id)),
            None => self
                .crontab_jobs
                .get(self.sel_cron - managed.len())
                .map(|j| format!("line:{}", j.line)),
        };
        let run = self
            .runs
            .get(self.sel_run)
            .map(|r| format!("{}/{}", r.routine_id, r.id));
        [routine, cron, run]
    }

    /// Puts each cursor back on what it was on, where that is still
    /// there. A list that is scrolled keeps its picture: it moves along
    /// with rows that came in above.
    fn restore_cursors(&mut self, on: [Option<String>; 3]) {
        let [routine, cron, run] = on;
        let mut moved = [None; 3];
        if let Some(id) = routine {
            let now = self
                .routines
                .iter()
                .filter(|r| !r.is_cron())
                .position(|r| r.id == id);
            moved[0] = now.map(|i| (self.sel_routine, i));
        }
        if let Some(key) = cron {
            let managed: Vec<&Routine> = self.routines.iter().filter(|r| r.is_cron()).collect();
            let now = managed
                .iter()
                .position(|r| format!("id:{}", r.id) == key)
                .or_else(|| {
                    self.crontab_jobs
                        .iter()
                        .position(|j| format!("line:{}", j.line) == key)
                        .map(|i| i + managed.len())
                });
            moved[1] = now.map(|i| (self.sel_cron, i));
        }
        if let Some(key) = run {
            let now = self
                .runs
                .iter()
                .position(|r| format!("{}/{}", r.routine_id, r.id) == key);
            moved[2] = now.map(|i| (self.sel_run, i));
        }
        for (list, change) in moved.into_iter().enumerate() {
            let Some((was, now)) = change else { continue };
            match list {
                0 => self.sel_routine = now,
                1 => self.sel_cron = now,
                _ => self.sel_run = now,
            }
            if self.tops[list] > 0 {
                self.tops[list] = (self.tops[list] + now).saturating_sub(was);
            }
        }
    }

    /// The models the form offers besides the agent's suggestions: what
    /// the agent lists itself (refreshed in the background) and what other
    /// routines of the same agent use.
    fn fill_models(&mut self) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let agent = form.agent();
        if agent::models(agent).is_empty() {
            form.more_models.clear();
            return;
        }
        models::refresh(&self.paths, agent);
        let mut more: Vec<(String, String)> = models::cached(&self.paths, agent)
            .into_iter()
            .map(|m| (m, format!("Listed by {agent}")))
            .collect();
        for r in &self.routines {
            let m = &r.file.model;
            if r.file.agent == agent && !m.is_empty() && !more.iter().any(|(x, _)| x == m) {
                more.push((m.clone(), format!("Used in {}", r.file.name)));
            }
        }
        form.more_models = more;
    }

    /// An account of the Multiaccount plugin as the list names it.
    pub fn account_name(&self, id: &str) -> String {
        self.accounts
            .iter()
            .flatten()
            .find(|a| a.id == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("{id} (not in Multiaccount any more)"))
    }

    /// Says something in the footer. What went wrong (red or yellow)
    /// stays until the next key or click; the rest goes after a moment.
    pub fn flash(&mut self, text: impl Into<String>, color: Color) {
        self.flash = Some(Flash {
            text: text.into(),
            color,
            until: Instant::now() + FLASH,
            sticky: color == RED || color == YELLOW,
        });
    }

    /// A key or a click: the problem it was shown before has been seen.
    fn seen_flash(&mut self) {
        if self.flash.as_ref().is_some_and(|f| f.sticky) {
            self.flash = None;
        }
    }

    /// The list a tab shows, as a place in `tops`.
    pub fn list_index(view: View) -> usize {
        match view {
            View::Routines => 0,
            View::Cron => 1,
            View::Runs => 2,
        }
    }

    /// The routines the current tab lists: those Herdr runs, or the cron
    /// jobs (which the person's own crontab jobs follow).
    pub fn listed(&self) -> Vec<&Routine> {
        let cron = self.view == View::Cron;
        self.routines
            .iter()
            .filter(|r| r.is_cron() == cron)
            .collect()
    }

    /// The cursor of the current list of routines.
    pub fn list_sel(&self) -> usize {
        if self.view == View::Cron {
            self.sel_cron
        } else {
            self.sel_routine
        }
    }

    pub fn selected_routine(&self) -> Option<&Routine> {
        if !self.view.lists_routines() {
            return None;
        }
        self.listed().get(self.list_sel()).copied()
    }

    /// The person's own cron job under the cursor (in the Cron jobs tab,
    /// after this plugin's cron jobs).
    pub fn selected_cron_job(&self) -> Option<&crate::crontab::CronJob> {
        if self.view != View::Cron {
            return None;
        }
        self.sel_cron
            .checked_sub(self.listed().len())
            .and_then(|i| self.crontab_jobs.get(i))
    }

    /// Shows a routine in its tab, under the cursor.
    pub fn select_id(&mut self, id: &str) {
        let Some(cron) = self
            .routines
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.is_cron())
        else {
            return;
        };
        self.view = if cron { View::Cron } else { View::Routines };
        self.follow = true;
        if let Some(i) = self.listed().iter().position(|r| r.id == id) {
            if cron {
                self.sel_cron = i;
            } else {
                self.sel_routine = i;
            }
        }
    }

    pub fn latest_run(&self, id: &str) -> Option<&Run> {
        self.all_runs.iter().find(|r| r.routine_id == id)
    }

    /// How many runs of a routine are kept.
    pub fn run_count(&self, id: &str) -> usize {
        self.all_runs.iter().filter(|r| r.routine_id == id).count()
    }

    /// Shows the runs of one routine (or all with None). The cursor stays
    /// on its run where the new list still has it.
    pub fn filter_runs(&mut self, routine_id: Option<String>) {
        let on = self
            .runs
            .get(self.sel_run)
            .map(|r| (r.routine_id.clone(), r.id.clone()));
        let from_runs = self.view == View::Runs;
        self.run_filter = routine_id;
        self.filter_from = None;
        self.view = View::Runs;
        self.reload();
        self.sel_run = 0;
        self.tops[2] = 0;
        if from_runs {
            if let Some((routine, run)) = on {
                if let Some(i) = self
                    .runs
                    .iter()
                    .position(|r| r.routine_id == routine && r.id == run)
                {
                    self.sel_run = i;
                }
            }
        }
        self.follow = true;
    }

    /// `h` or a click on "n runs" on a routine: its runs, and Esc comes
    /// back to it. One without runs says so instead of an empty list.
    fn runs_of_selected(&mut self) {
        let Some(id) = self.selected_routine().map(|r| r.id.clone()) else {
            return;
        };
        if self.run_count(&id) == 0 {
            return self.flash("No run yet", MUTED);
        }
        let from = self.view;
        // All of its runs: what narrowed the Runs tab before does not
        // come along.
        self.runs_problems = false;
        self.runs_search.clear();
        self.filter_runs(Some(id));
        self.filter_from = Some(from);
    }

    /// Esc in filtered runs: every filter goes. Back to the routine where
    /// "only its runs" came from its tab, else all runs again.
    fn leave_filter(&mut self) {
        self.runs_problems = false;
        self.runs_search.clear();
        match self.filter_from.take() {
            Some(view) => {
                self.run_filter = None;
                self.view = view;
                self.reload();
                self.follow = true;
            }
            None => self.filter_runs(None),
        }
    }

    /// Applies a changed filter of the Runs view. The cursor stays on its
    /// run where the new list still has it.
    fn refilter_runs(&mut self) {
        let on = self
            .runs
            .get(self.sel_run)
            .map(|r| (r.routine_id.clone(), r.id.clone()));
        self.runs = self.shown_runs();
        self.sel_run = on
            .and_then(|(routine, run)| {
                self.runs
                    .iter()
                    .position(|r| r.routine_id == routine && r.id == run)
            })
            .unwrap_or(0);
        self.tops[2] = 0;
        self.follow = true;
    }

    /// `p` in Runs: only the runs that went wrong or need the owner.
    fn toggle_problems(&mut self) {
        self.runs_problems = !self.runs_problems;
        self.refilter_runs();
    }

    /// `/` in Runs: asks for the text to search the runs for.
    fn ask_search(&mut self) {
        self.mode = Mode::Input(Input {
            title: "Search the runs".into(),
            hint: "Shows the runs with this text in the routine's name, the message or the result. Empty shows all again.".into(),
            text: self.runs_search.clone(),
            purpose: InputFor::Search,
        });
    }

    /// The ✕ of a filter: only that one goes.
    fn unfilter(&mut self, which: RunFilter) {
        match which {
            RunFilter::Routine => return self.filter_runs(None),
            RunFilter::Problems => self.runs_problems = false,
            RunFilter::Search => self.runs_search.clear(),
        }
        self.refilter_runs();
    }

    /// The name of the routine the runs are filtered to.
    pub fn filter_name(&self) -> Option<&str> {
        let id = self.run_filter.as_ref()?;
        self.routines
            .iter()
            .find(|r| &r.id == id)
            .map(|r| r.file.name.as_str())
            .or_else(|| {
                self.all_runs
                    .iter()
                    .find(|r| &r.routine_id == id)
                    .map(|r| r.routine_name.as_str())
            })
    }

    // ---- actions ----------------------------------------------------------

    fn fire(&mut self, id: &str) {
        // The `y` was for the routine as the list showed it: its files are
        // read again, and only one that can still run starts.
        if !Routine::load(&self.paths, id).is_ok_and(|r| r.can_run()) {
            self.reload();
            return self.flash("Not started: it changed meanwhile (a shows it)", YELLOW);
        }
        if let Some(r) = self.routines.iter().find(|r| r.id == id && r.is_cron()) {
            let started = crate::crontab::run_now(&self.paths, r);
            match started {
                Ok(()) => self.flash("Started in the background; its run shows in Runs", GREEN),
                Err(e) => self.flash(format!("Cannot start: {e}"), RED),
            }
            return self.reload();
        }
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return self.flash(format!("Cannot start: {e}"), RED),
        };
        use std::os::unix::process::CommandExt;
        let spawned = Command::new(exe)
            .args(["fire", id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        match spawned {
            Ok(_) if self.scheduler.is_none() => self.flash(
                "Starting; routines are off, so the run is not followed (S switches them on)",
                YELLOW,
            ),
            Ok(_) => self.flash("Starting, its tab opens in the background", GREEN),
            Err(e) => self.flash(format!("Cannot start: {e}"), RED),
        }
        self.reload();
    }

    fn toggle(&mut self) {
        let Some(id) = self.selected_routine().map(|r| r.id.clone()) else {
            return;
        };
        // From disk, not from the list: the files may have changed since
        // the list was read, and what is saved must be what is approved.
        let mut r = match Routine::load(&self.paths, &id) {
            Ok(r) if r.problem.is_none() || r.file.enabled => r,
            _ => return self.flash("Fix the routine first (⏎ edits it)", YELLOW),
        };
        r.file.enabled = !r.file.enabled;
        // Switching keeps a routine that waits for approval waiting: only
        // `a` approves what something else wrote.
        let saved = if r.approved {
            r.save_approved(&self.paths)
        } else {
            r.save(&self.paths).map(|_| ())
        };
        match saved {
            Ok(()) => {
                let state = if r.file.enabled { "on" } else { "off" };
                let color = if r.file.enabled { GREEN } else { RED };
                self.flash(format!("{} is {state}", r.file.name), color);
            }
            Err(e) => self.flash(e, RED),
        }
        if r.is_cron() && !r.approved {
            // It waits for approval, so its crontab line stays the approved
            // one (`write_job` writes nothing); only on or off follows the key.
            if let Err(e) = crate::crontab::switch(&self.paths, &id, r.file.enabled) {
                self.flash(format!("Crontab not updated: {e}"), RED);
            }
            self.crontab_read = None;
        }
        self.cron_write(&id);
        self.reload();
    }

    /// Writes a cron job's schedule, command and on or off into the
    /// crontab (nothing for a routine Herdr runs, and nothing for one that
    /// waits for approval: see `crontab::write_job`); says so when it fails.
    fn cron_write(&mut self, id: &str) {
        let Ok(r) = Routine::load(&self.paths, id) else {
            return;
        };
        if !r.is_cron() {
            return;
        }
        if let Err(e) = crate::crontab::write_job(&self.paths, &r) {
            self.flash(format!("Crontab not updated: {e}"), RED);
        }
        self.crontab_read = None;
    }

    /// Reads the crontab back into the managed cron jobs (edits made by
    /// hand win).
    fn cron_pull(&mut self) {
        if let Err(e) = crate::crontab::pull(&self.paths) {
            self.flash(format!("Crontab not read: {e}"), RED);
        }
    }

    /// `c`: a copy of the selected routine, switched off, without its runs
    /// or memory. A copy of a routine that waits for approval waits too.
    fn duplicate(&mut self) {
        let Some(id) = self.selected_routine().map(|r| r.id.clone()) else {
            return;
        };
        // From disk, as for switching: what is copied is what is approved.
        let source = match Routine::load(&self.paths, &id) {
            Ok(r) => r,
            Err(e) => return self.flash(format!("Cannot duplicate: {e}"), RED),
        };
        let mut name = format!("{} copy", source.file.name);
        let mut n = 2;
        while self.routines.iter().any(|r| r.file.name == name) {
            name = format!("{} copy {n}", source.file.name);
            n += 1;
        }
        let mut copy = source.clone();
        copy.id = routine::new_id(&self.paths, &name);
        copy.file.name = name;
        copy.file.enabled = false;
        let saved = if source.approved {
            copy.save_approved(&self.paths)
        } else {
            copy.save(&self.paths).map(|_| ())
        };
        match saved {
            Ok(()) => {
                self.flash(
                    format!("Duplicated as {}, switched off", copy.file.name),
                    GREEN,
                );
                // A copied cron job gets its own (switched off) crontab line.
                self.cron_write(&copy.id);
                self.reload();
                self.select_id(&copy.id);
            }
            Err(e) => self.flash(format!("Cannot duplicate: {e}"), RED),
        }
    }

    fn open_run(&mut self, run: Option<Run>) {
        let Some(run) = run else {
            return self.flash("No run yet", MUTED);
        };
        match runner::show(&self.paths, &run) {
            Ok(()) => self.flash("Showing its tab", GREEN),
            Err(_) => self.show_log(&run),
        }
    }

    /// The run a key is about: the one under the cursor in Runs, else the
    /// last run of the routine under the cursor.
    fn run_at_cursor(&self) -> Option<Run> {
        match self.view {
            View::Runs => self.runs.get(self.sel_run).cloned(),
            _ => self
                .selected_routine()
                .and_then(|r| self.latest_run(&r.id))
                .cloned(),
        }
    }

    /// `x`: ends a run that is going, after asking.
    fn ask_stop(&mut self, run: Option<Run>) {
        let Some(run) = run.filter(|r| r.status.active()) else {
            return self.flash("No run is going here", MUTED);
        };
        let what = if run.cron_pid != 0 {
            "Its command is stopped."
        } else {
            "Its tab is closed; what it wrote so far is kept."
        };
        self.mode = Mode::Confirm(Confirm {
            question: "Stop this run?".into(),
            lines: vec![
                format!(
                    "{}, started {}.",
                    run.routine_name,
                    ago(&run.started, &Local::now())
                ),
                what.into(),
            ],
            purpose: ConfirmFor::StopRun(run.routine_id, run.id),
        });
    }

    fn stop_run(&mut self, routine_id: &str, run_id: &str) {
        // The record as it is now: the run may have ended meanwhile.
        let Some(mut run) = store::load(&self.paths, routine_id, run_id) else {
            self.reload();
            return self.flash("Not stopped: the run is not there any more", RED);
        };
        let cron = run.cron_pid != 0;
        match runner::stop(&self.paths, &mut run) {
            // A cron job's own process writes down how it ended.
            Ok(()) if cron => self.flash("Stopping; its run shows it in a moment", ACCENT),
            Ok(()) => self.flash("Run stopped", ACCENT),
            Err(e) => self.flash(format!("Not stopped: {e}"), RED),
        }
        self.reload();
    }

    /// `d` on a run: deletes it, after asking.
    fn ask_delete_run(&mut self, run: Run) {
        if run.status.active() {
            return self.flash("It is still running (x stops it)", MUTED);
        }
        self.mode = Mode::Confirm(Confirm {
            question: format!("Delete this run of {}?", run.routine_name),
            lines: vec!["Its record, output and result are deleted. An open tab stays.".into()],
            purpose: ConfirmFor::DeleteRun(run.routine_id, run.id),
        });
    }

    /// The end of a run's output, for its page: `(lines, there is more)`.
    fn output_tail(&self, run: &Run) -> (Vec<String>, bool) {
        let (text, cut) = self
            .output_file(run)
            .and_then(|f| paths::read_tail_lossy(&f, 64 * 1024))
            .unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(visible_text).collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        (lines, cut)
    }

    /// ⏎ on a run: its page.
    fn open_run_page(&mut self, run: &Run) {
        let (tail, cut) = self.output_tail(run);
        self.mode = Mode::RunPage(RunPage {
            routine_id: run.routine_id.clone(),
            run_id: run.id.clone(),
            tail,
            cut,
        });
    }

    /// The run a page shows, as the list has it now.
    pub fn page_run(&self, page: &RunPage) -> Option<&Run> {
        self.all_runs
            .iter()
            .find(|r| r.routine_id == page.routine_id && r.id == page.run_id)
    }

    /// After a reload: a page whose run is gone closes, and one whose run
    /// is going reads the output again.
    fn refresh_run_page(&mut self) {
        let Mode::RunPage(page) = &self.mode else {
            return;
        };
        match self.page_run(page).cloned() {
            None => self.mode = Mode::Normal,
            Some(run) if run.status.active() || page.tail.is_empty() => {
                let (tail, cut) = self.output_tail(&run);
                if let Mode::RunPage(page) = &mut self.mode {
                    page.tail = tail;
                    page.cut = cut;
                }
            }
            Some(_) => {}
        }
    }

    /// A key on a run's page: the same keys as on the run in the list.
    fn run_page_key(&mut self, page: RunPage, code: KeyCode) {
        let Some(run) = self.page_run(&page).cloned() else {
            self.mode = Mode::Normal;
            return;
        };
        match code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.mode = Mode::Normal,
            KeyCode::Char('o') => {
                self.mode = Mode::Normal;
                self.open_run(Some(run));
            }
            KeyCode::Char('l') => self.show_log(&run),
            KeyCode::Char('f') => self.open_output_file(&run),
            KeyCode::Char('r') => {
                self.mode = Mode::Normal;
                self.ask_run_of(&run.routine_id);
            }
            KeyCode::Char('x') => self.ask_stop(Some(run)),
            KeyCode::Char('d') | KeyCode::Delete => self.ask_delete_run(run),
            _ => {}
        }
    }

    /// `W` or a click on "awake": when the computer is kept awake.
    fn ask_awake(&mut self) {
        if !crate::awake::available() {
            return self.flash("Keeping the computer awake is not supported here", MUTED);
        }
        let items: Vec<crate::form::Item> = crate::awake::Mode::ALL
            .iter()
            .map(|m| crate::form::Item {
                value: m.title().into(),
                label: m.title().into(),
                detail: m.detail().into(),
            })
            .collect();
        let now = crate::awake::mode(&self.paths);
        let sel = crate::awake::Mode::ALL
            .iter()
            .position(|m| *m == now)
            .unwrap_or(0);
        let mut picker = Picker::new("Keep the computer awake".into(), items, sel, PickFor::Awake);
        picker.hint =
            "A laptop with its lid closed still goes to sleep, whatever is chosen here.".into();
        self.mode = Mode::Pick(picker);
    }

    fn set_awake(&mut self, title: &str) {
        let Some(mode) = crate::awake::Mode::ALL
            .into_iter()
            .find(|m| m.title() == title)
        else {
            return;
        };
        match crate::awake::set_mode(&self.paths, mode) {
            Ok(()) => {
                self.awake = mode;
                use crate::awake::Mode as Awake;
                let (text, color) = match mode {
                    Awake::Runs => ("The computer now stays awake while a run is going", GREEN),
                    Awake::Always => ("The computer now stays awake while Herdr runs", GREEN),
                    Awake::Off => (
                        "The computer now sleeps as its own settings say, also in the middle of a run",
                        YELLOW,
                    ),
                };
                self.flash(text, color);
            }
            Err(e) => self.flash(format!("Not changed: {e}"), RED),
        }
    }

    /// The file with a run's output: a shell command's (or check's) whole
    /// output, else what its tab showed.
    pub fn output_file(&self, run: &Run) -> Option<std::path::PathBuf> {
        [run.file(&self.paths, "out"), run.log_path(&self.paths)]
            .into_iter()
            .find(|p| p.is_file())
    }

    fn open_output_file(&mut self, run: &Run) {
        match self.output_file(run) {
            Some(path) => {
                if (self.opener)(OPENER, &path.to_string_lossy()) {
                    self.flash("Opened the output file", GREEN);
                } else {
                    self.flash(format!("Could not run {OPENER}"), RED);
                }
            }
            None => self.flash("No output saved for this run", MUTED),
        }
    }

    fn show_log(&mut self, run: &Run) {
        let file = self.output_file(run);
        // The end of the file, whatever its size; bytes that are not text
        // do not hide the rest.
        let (text, cut) = file
            .as_ref()
            .and_then(|f| paths::read_tail_lossy(f, 2 * 1024 * 1024))
            .unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(visible_text).collect();
        if cut {
            lines.insert(
                0,
                "(the output is long: this is its end, f opens the whole file)".into(),
            );
        }
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        // The result first, when there is one; then the whole output.
        let has_result = !run.result.trim().is_empty();
        if has_result {
            // A command has no summary, only its last line.
            let heading = if run.agent == routine::SHELL {
                "Last line"
            } else {
                "Result"
            };
            let mut top = vec![heading.to_string()];
            top.extend(run.result.lines().map(|l| format!("  {l}")));
            top.push(String::new());
            if !lines.is_empty() {
                top.push("Output".to_string());
            }
            top.append(&mut lines);
            lines = top;
        }
        if lines.is_empty() {
            let why = if run.status.active() {
                "The run is still going: o goes to its tab."
            } else {
                "No output was saved for this run."
            };
            lines.push(why.into());
        }
        // How it went and why, always in sight above the output.
        let how = if run.attempt > 0 {
            format!("{} {}", run.trigger.label(), run.attempt)
        } else {
            run.trigger.label().to_string()
        };
        let mut note = format!(
            "{how} · {} {}",
            if run.status.active() { "for" } else { "took" },
            took(run)
        );
        if !run.message.trim().is_empty() {
            note = format!("{} · {note}", visible_text(run.message.trim()));
        }
        // Open at the result, or else at the end, where the answer is.
        self.mode = Mode::Log(LogView {
            title: format!(
                "{} · {} · {}",
                run.routine_name,
                run.started.format("%a %-d %b %H:%M"),
                run.status.label()
            ),
            lines,
            scroll: if has_result { 0 } else { u16::MAX },
            max: 0,
            file,
            approve: None,
            note,
        });
    }

    /// `n`: fill in the form, or describe the routine to Claude.
    fn new_routine(&mut self) {
        let item = |value: &str, label: &str, detail: &str| crate::form::Item {
            value: value.into(),
            label: label.into(),
            detail: detail.into(),
        };
        self.mode = Mode::Pick(Picker::new(
            "New".into(),
            vec![
                item(
                    BY_FORM,
                    "Herdr routine",
                    "An agent or a command, run by Herdr in a tab",
                ),
                item(
                    BY_CRON,
                    "Cron job",
                    "A command in your crontab, runs also when Herdr is closed",
                ),
                item(
                    BY_AGENT,
                    "Describe it to Claude",
                    "Claude asks what you need in a new tab and creates it",
                ),
            ],
            // In the Cron jobs tab, a cron job is the likely choice.
            usize::from(self.view == View::Cron),
            PickFor::New,
        ));
    }

    /// Opens a tab next to the list with Claude, briefed to create a
    /// routine with the person. Claude writes the files; the scheduler and
    /// the list pick them up.
    fn create_with_agent(&mut self) {
        let config = self.paths.config.clone();
        let brief = agent_brief(&self.paths);
        // So `herdr-routines routines` in that tab reads these routines.
        let env = vec![
            format!("HERDR_PLUGIN_CONFIG_DIR={}", config.display()),
            format!("HERDR_PLUGIN_STATE_DIR={}", self.paths.state.display()),
        ];
        if let Err(e) = paths::create_private_dir(&self.paths.routines_dir()) {
            self.flash(format!("Could not prepare the folder: {e}"), RED);
            return;
        }
        self.flash("Opening a tab with Claude…", ACCENT);
        std::thread::spawn(move || {
            let result =
                crate::herdr::open_focused_tab(&config.to_string_lossy(), "New routine", &env)
                    .and_then(|place| {
                        // Auto: it writes the files and runs the check without
                        // asking, and a safety check still stops risky steps.
                        let args = ["--permission-mode".into(), "auto".into()];
                        // The new tab's shell needs a moment before it takes
                        // the agent, as for runs.
                        let mut started = Ok(());
                        for attempt in 0..6 {
                            if attempt > 0 {
                                std::thread::sleep(Duration::from_secs(2));
                            }
                            started = crate::herdr::agent_start(
                                "new-routine",
                                "claude",
                                &place.pane_id,
                                &args,
                            );
                            match &started {
                                Err(e) if e.code != "agent_not_ready" && e.code != "timeout" => {}
                                _ => break,
                            }
                        }
                        match started {
                            Err(e) if e.code != "agent_not_ready" && e.code != "timeout" => {
                                return Err(e);
                            }
                            _ => {}
                        }
                        // The brief goes in as the first message once Claude
                        // takes it, also after it asked something first (such
                        // as whether to trust the folder). Up to ten minutes.
                        let mut last = Ok(());
                        for _ in 0..300 {
                            last = crate::herdr::agent_prompt(&place.pane_id, &brief);
                            match &last {
                                Err(e)
                                    if e.code == "agent_not_ready" || e.code == "agent_blocked" =>
                                {
                                    std::thread::sleep(Duration::from_secs(2));
                                }
                                _ => break,
                            }
                        }
                        last
                    });
            if let Err(e) = result {
                crate::herdr::notify("Could not start Claude", &e.to_string(), "none");
            }
        });
    }

    fn new_form(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_default();
        let folder = if cwd == std::path::Path::new("/") {
            std::env::var_os("HOME")
                .map(|h| paths::shorten(std::path::Path::new(&h)))
                .unwrap_or_else(|| "~".into())
        } else {
            paths::shorten(&cwd)
        };
        let mut form = Form::new(&folder);
        form.accounts = self.accounts.clone();
        self.form = Some(Box::new(form));
    }

    /// The form for a classic cron job: a shell command run by cron.
    fn new_cron_form(&mut self) {
        self.new_form();
        if let Some(form) = self.form.as_mut() {
            form.choose(Field::Agent, routine::SHELL);
            form.choose(Field::RunsIn, crate::routine::RunsIn::Cron.label());
            form.dirty = false;
        }
    }

    fn edit_selected(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        // From the files as they are now, which are remembered: what the
        // form saves is approved only if they were and still are.
        let r = self.note_opened(&r.id).unwrap_or(r);
        if r.unreadable {
            // The form would overwrite the file with stand-ins.
            let text = paths::read_text_limited(
                &self.paths.routine_dir(&r.id).join("routine.toml"),
                paths::FILE_LIMIT,
            )
            .unwrap_or_default();
            self.open_editor(&text, EditTarget::File(r.id.clone()), "routine.toml");
            self.flash(
                r.problem
                    .unwrap_or_else(|| "routine.toml cannot be read".into()),
                YELLOW,
            );
        } else {
            let mut form = Form::edit(&r);
            form.accounts = self.accounts.clone();
            self.form = Some(Box::new(form));
        }
    }

    fn open_editor(&mut self, text: &str, target: EditTarget, title: &str) {
        let mut t = TextArea::new(text);
        t.go_to_end();
        self.editor = Some(Editor {
            original: t.text(),
            text: t,
            target,
            title: title.into(),
        });
    }

    fn save_form(&mut self, form: &Form) -> Result<String, String> {
        let (file, prompt) = form.build()?;
        let id = match &form.id {
            Some(id) => id.clone(),
            None => routine::new_id(&self.paths, &file.name),
        };
        let name = file.name.clone();
        // A routine that waits for approval keeps waiting: approving is
        // what `a` does, after showing everything it would run.
        let waiting = form.id.as_ref().is_some_and(|id| !self.opened_approved(id));
        let r = Routine {
            id: id.clone(),
            file,
            prompt,
            problem: None,
            unreadable: false,
            approved: !waiting,
        };
        let was_cron = Routine::load(&self.paths, &id).is_ok_and(|old| old.is_cron());
        if waiting {
            r.save(&self.paths)?;
        } else {
            // The owner wrote it in the form: what it saves is approved.
            r.save_approved(&self.paths)?;
        }
        // A cron job's schedule, command and on or off live in the crontab.
        let crontab = if r.is_cron() {
            crate::crontab::write_job(&self.paths, &r)
        } else if was_cron && !waiting {
            // Now run by Herdr: its crontab line goes (once approved).
            crate::crontab::remove_job(&self.paths, &id, false)
        } else {
            Ok(())
        };
        let cron = crontab
            .err()
            .map(|e| format!("; but {e}"))
            .unwrap_or_default();
        self.crontab_read = None;
        self.reload();
        self.select_id(&id);
        Ok(if waiting {
            format!("Saved {name}; it still waits for approval (a){cron}")
        } else {
            format!("Saved {name}{cron}")
        })
    }

    fn try_save(&mut self) {
        let Some(form) = self.form.take() else {
            return;
        };
        match self.save_form(&form) {
            Ok(msg) => self.flash(msg, GREEN),
            Err(e) => {
                self.flash(format!("Not saved: {e}"), RED);
                self.form = Some(form);
            }
        }
    }

    fn save_editor(&mut self) {
        let Some(ed) = self.editor.take() else {
            return;
        };
        let text = ed.text.text();
        let result = match &ed.target {
            EditTarget::Form => {
                if let Some(form) = self.form.as_mut() {
                    if form.prompt != text {
                        form.prompt = text;
                        form.dirty = true;
                    }
                }
                return;
            }
            EditTarget::Prompt(id, name) => self.write_file(id, name, &text).map(|_| "Saved"),
            EditTarget::File(id) => self
                .write_file(id, "routine.toml", &text)
                .map(|_| "routine.toml saved"),
        };
        match result {
            Ok(msg) => {
                self.flash(msg, GREEN);
                if let EditTarget::Prompt(id, _) | EditTarget::File(id) = &ed.target {
                    self.cron_write(id);
                }
            }
            Err(e) => {
                self.flash(format!("Not saved: {e}"), RED);
                self.editor = Some(ed);
            }
        }
        self.reload();
    }

    /// Writes one file of a routine from the editor. It stays approved when
    /// it was; one that waits for approval keeps waiting.
    fn write_file(&self, id: &str, name: &str, text: &str) -> Result<(), String> {
        let was = self.opened_approved(id);
        paths::write_atomic(&self.paths.routine_dir(id).join(name), text.as_bytes())
            .map_err(|e| e.to_string())?;
        crate::approval::approve_file(&self.paths, id, was, name, text)
    }

    fn close_form(&mut self) {
        match &self.form {
            Some(f) if f.dirty => {
                self.mode = Mode::Confirm(Confirm {
                    question: "Discard the changes?".into(),
                    lines: vec![if f.id.is_some() {
                        "The routine stays as it was.".into()
                    } else {
                        "The new routine is not created.".into()
                    }],
                    purpose: ConfirmFor::DiscardForm,
                })
            }
            _ => self.form = None,
        }
    }

    fn ask_run(&mut self) {
        if let Some(id) = self.selected_routine().map(|r| r.id.clone()) {
            self.ask_run_of(&id);
        }
    }

    /// Asks before running a routine by hand.
    fn ask_run_of(&mut self, id: &str) {
        let Some(r) = self.routines.iter().find(|r| r.id == id).cloned() else {
            return self.flash("Its routine was deleted", MUTED);
        };
        if let Some(p) = &r.problem {
            return self.flash(format!("Cannot run: {p}"), RED);
        }
        if !r.approved {
            return self.flash(
                "It changed outside the list: a shows it for approval",
                YELLOW,
            );
        }
        let mut lines = vec![if r.is_cron() {
            "The command runs now in the background, as cron would run it; its run shows in Runs."
                .to_string()
        } else if r.is_shell() {
            format!(
                "The command runs in a new tab in the workspace {}.",
                Settings::load(&self.paths).workspace
            )
        } else {
            format!(
                "{} starts in a new tab in the workspace {} and gets the prompt.",
                r.file.agent,
                Settings::load(&self.paths).workspace
            )
        }];
        let flags = crate::agent::flags(&r.file);
        if !r.is_shell() && !flags.is_empty() {
            lines.push(format!("Options: {}", flags.join(" ")));
        }
        self.mode = Mode::Confirm(Confirm {
            question: format!("Run {} now?", r.file.name),
            lines,
            purpose: ConfirmFor::Run(r.id),
        });
    }

    /// Reads the routine's files once, remembers them for the form or
    /// editor that opens on them, and returns the routine they make.
    fn note_opened(&mut self, id: &str) -> Option<Routine> {
        let files = crate::approval::snapshot(&self.paths, id);
        let approvals = crate::approval::Approvals::load(&self.paths);
        let now = Routine::from_files(id, &files, &approvals).ok();
        let approved = now.as_ref().is_some_and(|r| r.approved);
        self.opened = Some((id.to_string(), files, approved));
        now
    }

    /// Whether what the form or editor saves for this routine is the
    /// owner's edit of approved files: they were approved when it opened
    /// and are unchanged since. Judged against what was opened, not
    /// against the files at the moment of saving, so text that came from
    /// a change made elsewhere is never approved by saving it.
    fn opened_approved(&self, id: &str) -> bool {
        self.opened
            .as_ref()
            .is_some_and(|(opened, files, approved)| {
                opened == id && *approved && crate::approval::snapshot(&self.paths, id) == *files
            })
    }

    /// `a`: shows everything a routine changed outside the list would run,
    /// from one read of its files, and approves exactly those files with
    /// `y` once the owner has scrolled to the end.
    fn ask_approve(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        if r.approved {
            return self.flash("It is approved: it runs as saved", MUTED);
        }
        if r.unreadable {
            return self.flash("Fix the routine first (⏎ edits it)", YELLOW);
        }
        let files = crate::approval::snapshot(&self.paths, &r.id);
        let approvals = crate::approval::Approvals::load(&self.paths);
        let now = match Routine::from_files(&r.id, &files, &approvals) {
            Ok(now) => now,
            Err(e) => return self.flash(format!("Cannot read it: {e}"), RED),
        };
        if now.approved {
            self.reload();
            return self.flash("It is approved: it runs as saved", MUTED);
        }
        self.mode = Mode::Log(LogView {
            title: visible_text(&format!("Approve {}?", now.file.name)),
            lines: approval_lines(&self.paths, &now),
            scroll: 0,
            // Unknown until drawn, so `y` cannot approve unseen text.
            max: u16::MAX,
            file: None,
            approve: Some((r.id, files)),
            note: String::new(),
        });
    }

    /// `y` in the approval view: approves the files it showed, unless they
    /// changed while it was open.
    fn approve_shown(&mut self, id: String, files: crate::approval::Snapshot) {
        self.mode = Mode::Normal;
        if crate::approval::snapshot(&self.paths, &id) != files {
            self.reload();
            return self.flash("It changed meanwhile; a shows it again", YELLOW);
        }
        match crate::approval::approve(&self.paths, &id, files) {
            Ok(()) => self.flash("Approved: it runs as shown", GREEN),
            Err(e) => self.flash(e, RED),
        }
        self.cron_write(&id);
        self.reload();
    }

    fn delete_routine(&mut self, id: &str, with_runs: bool) {
        let had_runs = self.run_count(id) > 0;
        let was_cron = Routine::load(&self.paths, id).is_ok_and(|r| r.is_cron());
        if was_cron {
            // Its crontab line goes first, so cron never starts a job whose
            // routine is gone.
            if let Err(e) = crate::crontab::remove_job(&self.paths, id, false) {
                return self.flash(format!("Not deleted: the crontab {e}"), RED);
            }
            self.crontab_read = None;
        }
        match Routine::delete(&self.paths, id, with_runs) {
            Ok(()) if had_runs && !with_runs => {
                self.flash("Deleted; its runs stay in Runs", ACCENT)
            }
            Ok(()) => self.flash("Deleted", ACCENT),
            Err(e) => self.flash(e, RED),
        }
        self.reload();
    }

    fn ask_delete(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        let runs = self.run_count(&r.id);
        let mut lines = vec![if r.is_cron() {
            "The cron job is deleted, and its line in your crontab with it.".to_string()
        } else if r.is_shell() {
            "The routine and its command are deleted. Tabs of runs that are still open stay."
                .to_string()
        } else {
            "The routine and its prompt are deleted. Tabs of runs that are still open stay."
                .to_string()
        }];
        if runs > 0 {
            lines.push(format!(
                "Its {} stay in Runs (y), or are deleted too (r).",
                if runs == 1 {
                    "run".to_string()
                } else {
                    format!("{runs} runs")
                }
            ));
        }
        self.mode = Mode::Confirm(Confirm {
            question: format!("Delete {}?", r.file.name),
            lines,
            purpose: ConfirmFor::Delete(r.id, runs > 0),
        });
    }

    /// `C`: the jobs that are only in the crontab, to take them over.
    fn show_cron(&mut self) {
        match crate::crontab::read() {
            Ok(text) => {
                let jobs = crate::crontab::parse(&text);
                if jobs.is_empty() {
                    self.flash("No other cron jobs in your crontab", MUTED);
                } else {
                    self.take_sel = 0;
                    self.mode = Mode::Cron(jobs);
                }
            }
            Err(e) => self.flash(format!("Could not read the crontab: {e}"), RED),
        }
    }

    /// Takes cron jobs over. Keeping and moving change the crontab, so
    /// they ask first; a copy leaves it alone.
    fn ask_import(&mut self, jobs: Vec<crate::crontab::CronJob>, how: crate::crontab::Take) {
        use crate::crontab::Take;
        let one = jobs.len() == 1;
        let what = if one {
            "1 cron job".to_string()
        } else {
            format!("{} cron jobs", jobs.len())
        };
        let (question, text) = match how {
            Take::Copy => return self.import_cron(jobs, how),
            Take::ToHerdr => (
                format!("Move {what} to Herdr?"),
                if one {
                    "Herdr runs it in a tab from now on. Its line in your crontab is commented out, so it does not run twice; a backup of the crontab is kept."
                } else {
                    "Herdr runs them in tabs from now on. Their lines in your crontab are commented out, so nothing runs twice; a backup of the crontab is kept."
                },
            ),
            Take::KeepCron => (
                format!("Take over {what}?"),
                if one {
                    "Cron still runs it, also when Herdr is closed. The list puts a marker around its line in your crontab to follow its runs; a backup of the crontab is kept."
                } else {
                    "Cron still runs them, also when Herdr is closed. The list puts a marker around their lines in your crontab to follow their runs; a backup of the crontab is kept."
                },
            ),
        };
        self.mode = Mode::Confirm(Confirm {
            question,
            lines: vec![text.to_string()],
            purpose: ConfirmFor::Import(jobs, how),
        });
    }

    fn import_cron(&mut self, jobs: Vec<crate::crontab::CronJob>, how: crate::crontab::Take) {
        use crate::crontab::Take;
        self.mode = Mode::Normal;
        match crate::crontab::import(&self.paths, &jobs, how) {
            Ok(n) => {
                let what = if n == 1 {
                    "1 cron job".to_string()
                } else {
                    format!("{n} cron jobs")
                };
                let text = match how {
                    Take::ToHerdr => format!("Moved {what} to Herdr"),
                    Take::KeepCron => format!("Took over {what}; cron still runs them"),
                    Take::Copy => format!("Copied {what}, switched off"),
                };
                self.flash(text, GREEN);
            }
            Err(e) => self.flash(e, RED),
        }
        self.view = if how == Take::KeepCron {
            View::Cron
        } else {
            View::Routines
        };
        self.crontab_read = None;
        self.reload();
    }

    /// `A`: start Herdr at login, or not any more (asks before turning on).
    fn toggle_autostart(&mut self) {
        if !crate::autostart::supported() {
            self.flash("Starting at login is not supported here", MUTED);
        } else if crate::autostart::is_on() {
            self.mode = Mode::Confirm(Confirm {
                question: "Turn autostart off?".into(),
                lines: vec![
                    "Herdr then no longer starts when you log in: after a restart, routines run only once you open Herdr. Herdr running now keeps running.".into(),
                ],
                purpose: ConfirmFor::AutostartOff,
            });
        } else {
            self.ask_autostart();
        }
    }

    fn ask_autostart(&mut self) {
        self.mode = Mode::Confirm(Confirm {
            question: "Turn autostart on?".into(),
            lines: vec![
                "Herdr then starts in the background when you log in, so routines also run after a restart without opening it. Opening Herdr connects to it as usual.".into(),
                "Takes effect at the next login. Change it any time with a click on \"autostart\" at the top.".into(),
            ],
            purpose: ConfirmFor::Autostart,
        });
    }

    fn confirmed(&mut self, c: Confirm) {
        self.mode = Mode::Normal;
        match c.purpose {
            ConfirmFor::AutostartOff => match crate::autostart::turn_off() {
                Ok(()) => self.flash("Autostart is off", RED),
                Err(e) => self.flash(format!("Could not turn it off: {e}"), RED),
            },
            ConfirmFor::Autostart => {
                let mut bin = crate::herdr::bin();
                if !bin.is_absolute() {
                    if let Some((0, out)) =
                        crate::shell::run("command -v herdr", None, Duration::from_secs(10))
                    {
                        bin = std::path::PathBuf::from(out.trim());
                    }
                }
                match crate::autostart::turn_on(&bin.to_string_lossy()) {
                    Ok(()) => self.flash("Autostart is on: Herdr starts at login", GREEN),
                    Err(e) => self.flash(format!("Could not turn it on: {e}"), RED),
                }
            }
            ConfirmFor::Run(id) => self.fire(&id),
            ConfirmFor::Delete(id, _) => self.delete_routine(&id, false),
            ConfirmFor::DeleteRun(routine_id, run_id) => {
                if let Some(run) = self
                    .all_runs
                    .iter()
                    .find(|r| r.routine_id == routine_id && r.id == run_id)
                    .cloned()
                {
                    store::remove(&self.paths, &run);
                    self.flash("Run deleted", ACCENT);
                }
                self.reload();
            }
            ConfirmFor::StopRun(routine_id, run_id) => self.stop_run(&routine_id, &run_id),
            ConfirmFor::DiscardForm => self.form = None,
            ConfirmFor::DiscardEditor => self.editor = None,
            ConfirmFor::Import(jobs, how) => self.import_cron(jobs, how),
            ConfirmFor::RoutinesOn => self.routines_on(),
            ConfirmFor::RoutinesOff => self.routines_off(),
        }
    }

    /// `S` or a click on "routines on/off": all routines on or off, after
    /// asking.
    fn toggle_routines(&mut self) {
        if self.scheduler.is_some() {
            self.mode = Mode::Confirm(Confirm {
                question: "Switch all routines off?".into(),
                lines: vec![
                    "No routine runs until you switch them on again here, also not after Herdr restarts. Cron jobs skip their times too. Runs going on now carry on.".into(),
                    "Each routine keeps its own on or off.".into(),
                ],
                purpose: ConfirmFor::RoutinesOff,
            });
        } else {
            self.mode = Mode::Confirm(Confirm {
                question: "Switch routines on?".into(),
                lines: vec![
                    "Routines that are on run on their schedule again, and cron jobs at their times.".into(),
                ],
                purpose: ConfirmFor::RoutinesOn,
            });
        }
    }

    fn routines_on(&mut self) {
        if let Err(e) = scheduler::set_paused(&self.paths, false) {
            return self.flash(format!("Could not switch them on: {e}"), RED);
        }
        if self.scheduler.is_some() {
            return self.flash("Routines are on", GREEN);
        }
        match scheduler::spawn(&self.paths) {
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(300));
                self.reload();
                self.flash("Routines are on", GREEN);
            }
            Err(e) => self.flash(e, RED),
        }
    }

    fn routines_off(&mut self) {
        if let Err(e) = scheduler::set_paused(&self.paths, true) {
            return self.flash(format!("Could not switch them off: {e}"), RED);
        }
        scheduler::stop(&self.paths);
        std::thread::sleep(Duration::from_millis(300));
        self.reload();
        self.flash("All routines are off", RED);
    }

    fn toggle_icons(&mut self) {
        self.icons.nerd = !self.icons.nerd;
        let value = if self.icons.nerd {
            "\"nerd\""
        } else {
            "\"plain\""
        };
        let file = self.paths.settings_file();
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        if paths::write_atomic(&file, with_value(&text, "icons", value).as_bytes()).is_err() {
            self.flash("Could not save the icon choice", RED);
        }
    }

    fn open_picker(&mut self) {
        self.fill_models();
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let field = form.current();
        let items = form.items(field);
        if items.is_empty() {
            return;
        }
        let now = form.value(field);
        let sel = items.iter().position(|i| i.value == now).unwrap_or(0);
        self.mode = Mode::Pick(Picker::new(
            form.label(field).into(),
            items,
            sel,
            PickFor::Field(field),
        ));
    }

    fn picked(&mut self, p: Picker) {
        self.mode = Mode::Normal;
        let Some(item) = p.items.get(p.sel) else {
            return;
        };
        let field = match p.purpose {
            PickFor::New => {
                match item.value.as_str() {
                    BY_AGENT => self.create_with_agent(),
                    BY_CRON => self.new_cron_form(),
                    _ => self.new_form(),
                }
                return;
            }
            PickFor::Awake => return self.set_awake(&item.value),
            PickFor::Field(field) => field,
        };
        if item.value == OTHER_MODEL {
            let text = self
                .form
                .as_ref()
                .map(|f| f.model.clone())
                .unwrap_or_default();
            self.mode = Mode::Input(Input {
                title: "Model name".into(),
                hint: "The model's full name or an alias the agent knows. Empty means the agent's default.".into(),
                text,
                purpose: InputFor::Model,
            });
            return;
        }
        if let Some(form) = self.form.as_mut() {
            form.choose(field, &item.value);
        }
    }

    fn entered(&mut self, input: Input) {
        self.mode = Mode::Normal;
        match input.purpose {
            InputFor::Model => {
                if let Some(form) = self.form.as_mut() {
                    form.choose(Field::Model, input.text.trim());
                }
            }
            InputFor::Search => {
                self.runs_search = input.text.trim().to_string();
                self.refilter_runs();
            }
        }
    }

    /// ⏎ on a form field.
    fn form_enter(&mut self) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match form.current() {
            Field::Prompt => {
                let text = form.prompt.clone();
                let title = form.label(Field::Prompt);
                self.open_editor(&text, EditTarget::Form, title);
            }
            Field::Save => self.try_save(),
            Field::More => form.toggle_more(),
            Field::Days => form.toggle_day(form.day_cursor),
            f if f.is_choice() => self.open_picker(),
            _ => form.move_by(1),
        }
    }

    // ---- keys -------------------------------------------------------------

    fn rows_len(&self) -> usize {
        match self.view {
            View::Routines => self.listed().len(),
            View::Cron => self.listed().len() + self.crontab_jobs.len(),
            View::Runs => self.runs.len(),
        }
    }

    fn cursor(&mut self) -> &mut usize {
        match self.view {
            View::Routines => &mut self.sel_routine,
            View::Cron => &mut self.sel_cron,
            View::Runs => &mut self.sel_run,
        }
    }

    fn step(&mut self, by: i32) {
        let len = self.rows_len();
        if len == 0 {
            return;
        }
        let c = self.cursor();
        *c = (*c as i64 + by as i64).clamp(0, len as i64 - 1) as usize;
        self.follow = true;
    }

    /// Goes to another tab.
    fn show(&mut self, view: View) {
        self.view = view;
        self.follow = true;
    }

    /// A key that needs a routine, on a job that is only in the crontab.
    fn only_in_crontab(&mut self) {
        self.flash(
            "This job is only in your crontab: ⏎ takes it over first",
            MUTED,
        );
    }

    /// `Ctrl+D` in the editor: leaves without keeping what was written,
    /// after asking when something changed.
    fn discard_editor(&mut self) {
        let Some(ed) = &self.editor else {
            return;
        };
        if ed.text.text() == ed.original {
            self.editor = None;
            return;
        }
        self.mode = Mode::Confirm(Confirm {
            question: "Discard what you wrote?".into(),
            lines: vec!["The text stays as it was when the editor opened.".into()],
            purpose: ConfirmFor::DiscardEditor,
        });
    }

    fn enter(&mut self) {
        match self.view {
            View::Cron if self.selected_cron_job().is_some() => {
                let job = self.selected_cron_job().cloned().unwrap();
                self.take_sel = 0;
                self.mode = Mode::Cron(vec![job]);
            }
            View::Routines | View::Cron => self.edit_selected(),
            View::Runs => {
                if let Some(run) = self.runs.get(self.sel_run).cloned() {
                    self.open_run_page(&run);
                }
            }
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        self.seen_flash();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let writing = self.editor.is_some() && matches!(self.mode, Mode::Normal);
        if ctrl && k.code == KeyCode::Char('c') && !writing {
            self.mode = Mode::AskQuit;
            return;
        }
        match self.mode.clone() {
            Mode::AskQuit => {
                self.mode = Mode::Normal;
                if k.code == KeyCode::Char('y') {
                    self.quit = true;
                }
            }
            Mode::Help => match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.mode = Mode::Normal;
                }
                KeyCode::Up | KeyCode::Char('k') => self.scroll_help(-1),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_help(1),
                KeyCode::PageUp => self.scroll_help(-10),
                KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_help(10),
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => self.help_scroll = self.help_max,
                _ => {}
            },
            Mode::RunPage(page) => self.run_page_key(page, k.code),
            Mode::Menu(mut m) => match k.code {
                KeyCode::Up | KeyCode::BackTab if !m.items.is_empty() => {
                    m.sel = (m.sel + m.items.len() - 1) % m.items.len();
                    self.mode = Mode::Menu(m);
                }
                KeyCode::Down | KeyCode::Tab if !m.items.is_empty() => {
                    m.sel = (m.sel + 1) % m.items.len();
                    self.mode = Mode::Menu(m);
                }
                KeyCode::Enter | KeyCode::Char(' ') => match m.items.get(m.sel) {
                    Some(item) => self.menu_pick(item.action),
                    None => self.mode = Mode::Normal,
                },
                // An entry's own key picks it.
                KeyCode::Char(c) if m.items.iter().any(|i| i.key == c.to_string()) => {
                    let item = m.items.iter().find(|i| i.key == c.to_string()).unwrap();
                    self.menu_pick(item.action);
                }
                _ => self.mode = Mode::Normal,
            },
            Mode::Confirm(c) => match (k.code, &c.purpose) {
                (KeyCode::Char('y'), _) => self.confirmed(c),
                (KeyCode::Enter, _) if c.enter_confirms() => self.confirmed(c),
                // Deleting a routine with runs: `r` takes its runs along.
                (KeyCode::Char('r'), ConfirmFor::Delete(id, true)) => {
                    let id = id.clone();
                    self.mode = Mode::Normal;
                    self.delete_routine(&id, true);
                }
                _ => self.mode = Mode::Normal,
            },
            // A long list: typing narrows it, Backspace widens it again.
            Mode::Pick(mut p)
                if p.filters()
                    && !ctrl
                    && matches!(k.code, KeyCode::Char(_) | KeyCode::Backspace) =>
            {
                match k.code {
                    KeyCode::Char(c) => p.filter.push(c),
                    _ => {
                        p.filter.pop();
                    }
                }
                p.refilter();
                self.mode = Mode::Pick(p);
            }
            Mode::Pick(mut p) => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                    p.sel = p.sel.saturating_sub(1);
                    self.mode = Mode::Pick(p);
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    p.sel = (p.sel + 1).min(p.items.len().saturating_sub(1));
                    self.mode = Mode::Pick(p);
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    p.sel = 0;
                    self.mode = Mode::Pick(p);
                }
                KeyCode::End | KeyCode::Char('G') => {
                    p.sel = p.items.len().saturating_sub(1);
                    self.mode = Mode::Pick(p);
                }
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let i = c as usize - '1' as usize;
                    if i < p.items.len() {
                        p.sel = i;
                        self.picked(p);
                    }
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.picked(p),
                _ => {}
            },
            Mode::Input(mut input) => match k.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => self.entered(input),
                KeyCode::Backspace => {
                    input.text.pop();
                    self.mode = Mode::Input(input);
                }
                KeyCode::Char('u') if ctrl => {
                    input.text.clear();
                    self.mode = Mode::Input(input);
                }
                KeyCode::Char(c) if !ctrl && input.text.chars().count() < 80 => {
                    input.text.push(c);
                    self.mode = Mode::Input(input);
                }
                _ => {}
            },
            Mode::Message { .. } => match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {
                    self.mode = Mode::Normal;
                    if std::mem::take(&mut self.autostart_after_intro) {
                        self.ask_autostart_once();
                    }
                }
                // A note longer than the window scrolls.
                KeyCode::Up | KeyCode::Char('k') => self.scroll_help(-1),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_help(1),
                KeyCode::PageUp => self.scroll_help(-10),
                KeyCode::PageDown => self.scroll_help(10),
                _ => {}
            },
            Mode::Cron(jobs) => match k.code {
                KeyCode::Up | KeyCode::BackTab => {
                    self.take_sel = (self.take_sel + TAKES.len() - 1) % TAKES.len();
                }
                KeyCode::Down | KeyCode::Tab => self.take_sel = (self.take_sel + 1) % TAKES.len(),
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let how = TAKES[self.take_sel.min(TAKES.len() - 1)].1;
                    self.ask_import(jobs, how);
                }
                KeyCode::Char(c) if TAKES.iter().any(|t| t.0 == c) => {
                    let how = TAKES.iter().find(|t| t.0 == c).map(|t| t.1);
                    if let Some(how) = how {
                        self.ask_import(jobs, how);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::Log(mut log) => {
                let page = 10;
                match k.code {
                    KeyCode::Char('y') if log.approve.is_some() && log.scroll >= log.max => {
                        if let Some((id, files)) = log.approve.take() {
                            self.approve_shown(id, files);
                        }
                        return;
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('l') => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Up | KeyCode::Char('k') => log.scroll = log.scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        log.scroll = (log.scroll + 1).min(log.max)
                    }
                    KeyCode::PageUp => log.scroll = log.scroll.saturating_sub(page),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        log.scroll = log.scroll.saturating_add(page).min(log.max)
                    }
                    KeyCode::Home | KeyCode::Char('g') => log.scroll = 0,
                    KeyCode::End | KeyCode::Char('G') => log.scroll = log.max,
                    KeyCode::Char('f') => {
                        if let Some(path) = log.file.clone() {
                            if !(self.opener)(OPENER, &path.to_string_lossy()) {
                                self.flash(format!("Could not run {OPENER}"), RED);
                            }
                        }
                    }
                    _ => {}
                }
                self.mode = Mode::Log(log);
            }
            Mode::Normal => {
                if self.editor.is_some() {
                    self.editor_key(k);
                } else if self.form.is_some() {
                    self.form_key(k);
                } else {
                    self.list_key(k);
                }
            }
        }
    }

    fn editor_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            // Like a notes app: leaving keeps what you wrote; Ctrl+Z undoes.
            KeyCode::Esc => self.save_editor(),
            KeyCode::Char('s') if ctrl => self.save_editor(),
            KeyCode::Char('o') if ctrl => self.external = true,
            KeyCode::Char('d') if ctrl => self.discard_editor(),
            KeyCode::Char('c') | KeyCode::Char('x') if ctrl => {
                let Some(ed) = self.editor.as_mut() else {
                    return;
                };
                let text = if k.code == KeyCode::Char('x') {
                    ed.text.cut()
                } else {
                    ed.text.selected_text()
                };
                if let Some(text) = text {
                    clipboard::set(&text);
                    self.clip = text;
                }
            }
            KeyCode::Char('v') if ctrl => {
                let text = clipboard::get().unwrap_or_else(|| self.clip.clone());
                if let Some(ed) = self.editor.as_mut() {
                    ed.text.paste(&text);
                    ed.text.follow();
                }
            }
            _ => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.text.key(&k);
                }
            }
        }
    }

    fn form_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let field = form.current();
        match k.code {
            KeyCode::Char('s') if ctrl => self.try_save(),
            KeyCode::Char('u') if ctrl => form.clear(),
            KeyCode::Esc => self.close_form(),
            KeyCode::Up | KeyCode::BackTab => form.move_by(-1),
            KeyCode::Down | KeyCode::Tab => form.move_by(1),
            KeyCode::PageUp => form.move_by(-6),
            KeyCode::PageDown => form.move_by(6),
            // In a text field the arrows, Home and End move the cursor.
            KeyCode::Left if field.is_text() => form.cursor_by(-1),
            KeyCode::Right if field.is_text() => form.cursor_by(1),
            KeyCode::Home if field.is_text() => form.cursor_home(),
            KeyCode::End if field.is_text() => form.cursor_end(),
            // On the weekday switches they move from day to day, and the
            // digits switch Monday (1) to Sunday (7).
            KeyCode::Left if field == Field::Days => form.day_by(-1),
            KeyCode::Right if field == Field::Days => form.day_by(1),
            KeyCode::Char(c @ '1'..='7') if field == Field::Days => {
                form.toggle_day(c as usize - '1' as usize)
            }
            KeyCode::Home => form.go(0),
            KeyCode::End => form.move_by(99),
            KeyCode::Left => form.cycle(-1),
            KeyCode::Right => form.cycle(1),
            KeyCode::Enter => self.form_enter(),
            KeyCode::Backspace => form.backspace(),
            KeyCode::Delete => form.delete(),
            KeyCode::Char(' ') if !field.is_text() => self.form_enter(),
            KeyCode::Char('?') if !field.is_text() => {
                self.help_scroll = 0;
                self.mode = Mode::Help;
            }
            KeyCode::Char('j') if !field.is_text() => form.move_by(1),
            KeyCode::Char('k') if !field.is_text() => form.move_by(-1),
            KeyCode::Char(c) if !ctrl => form.type_char(c),
            _ => {}
        }
    }

    fn list_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.mode = Mode::AskQuit,
            // Esc in filtered runs removes the filters.
            KeyCode::Esc if self.runs_filtered() && self.view == View::Runs => self.leave_filter(),
            KeyCode::Esc => {}
            KeyCode::Char('p') if self.view == View::Runs => self.toggle_problems(),
            KeyCode::Char('/') if self.view == View::Runs => self.ask_search(),
            KeyCode::Char('o') if self.view == View::Runs => {
                if let Some(run) = self.runs.get(self.sel_run).cloned() {
                    self.open_run(Some(run));
                }
            }
            KeyCode::Char('r') if self.view == View::Runs => {
                if let Some(id) = self.runs.get(self.sel_run).map(|r| r.routine_id.clone()) {
                    self.ask_run_of(&id);
                }
            }
            KeyCode::Char('W') => self.ask_awake(),
            KeyCode::Char('s') => self.toggle_sort(),
            KeyCode::Char('h') => match self.view {
                View::Routines | View::Cron if self.selected_cron_job().is_some() => {
                    self.only_in_crontab()
                }
                View::Routines | View::Cron => self.runs_of_selected(),
                View::Runs => {
                    let id = self.runs.get(self.sel_run).map(|r| r.routine_id.clone());
                    if id.is_some() && id == self.run_filter {
                        self.filter_runs(None);
                    } else if id.is_some() {
                        self.filter_runs(id);
                    }
                }
            },
            KeyCode::Char('f') if self.view == View::Runs => {
                if let Some(run) = self.runs.get(self.sel_run).cloned() {
                    self.open_output_file(&run);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if self.view == View::Runs => {
                if let Some(run) = self.runs.get(self.sel_run).cloned() {
                    self.ask_delete_run(run);
                }
            }
            KeyCode::Char('?') => {
                self.help_scroll = 0;
                self.mode = Mode::Help;
            }
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-10),
            KeyCode::PageDown => self.step(10),
            KeyCode::Char('g') | KeyCode::Home => self.step(i32::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.step(i32::MAX / 2),
            KeyCode::Tab | KeyCode::Right => self.show(self.view.step(1)),
            KeyCode::BackTab | KeyCode::Left => self.show(self.view.step(-1)),
            KeyCode::Char('1') => self.show(View::Routines),
            KeyCode::Char('2') => self.show(View::Cron),
            KeyCode::Char('3') => self.show(View::Runs),
            KeyCode::Char('S') => self.toggle_routines(),
            KeyCode::Char('A') => self.toggle_autostart(),
            KeyCode::Char('C') => self.show_cron(),
            KeyCode::Char('i') => self.toggle_icons(),
            KeyCode::Char('n') => self.new_routine(),
            KeyCode::Enter => self.enter(),
            // A job that is only in the crontab has no runs, prompt or
            // switch here: say so instead of doing nothing.
            KeyCode::Char('l' | 'e' | 'p' | 'r' | 'a' | ' ' | 'c' | 'o' | 'd' | 'x')
            | KeyCode::Delete
                if self.selected_cron_job().is_some() =>
            {
                self.only_in_crontab()
            }
            KeyCode::Char('x') if self.rows_len() > 0 => {
                let run = self.run_at_cursor();
                self.ask_stop(run);
            }
            KeyCode::Char('l') => {
                let run = match self.view {
                    View::Routines | View::Cron => self
                        .selected_routine()
                        .and_then(|r| self.latest_run(&r.id))
                        .cloned(),
                    View::Runs => self.runs.get(self.sel_run).cloned(),
                };
                match run {
                    Some(run) => self.show_log(&run),
                    None if self.rows_len() > 0 => self.flash("No run yet", MUTED),
                    None => {}
                }
            }
            _ if self.selected_routine().is_some() => match k.code {
                KeyCode::Char('e') => self.edit_selected(),
                KeyCode::Char('p') => {
                    let r = self.selected_routine().cloned().unwrap();
                    let what = if r.is_shell() { "Command" } else { "Prompt" };
                    let title = format!("{what} of {}", r.file.name);
                    let name = Routine::text_file(&r.file.agent);
                    // The text as it is now, remembered with the other
                    // files (see `opened_approved`).
                    self.note_opened(&r.id);
                    let text = self
                        .opened
                        .as_ref()
                        .and_then(|(_, files, _)| files.get(name).cloned())
                        .unwrap_or_default();
                    self.open_editor(&text, EditTarget::Prompt(r.id, name), &title);
                }
                KeyCode::Char('r') => self.ask_run(),
                KeyCode::Char('a') => self.ask_approve(),
                KeyCode::Char(' ') => self.toggle(),
                KeyCode::Char('c') => self.duplicate(),
                KeyCode::Char('o') => {
                    let id = self.selected_routine().unwrap().id.clone();
                    let run = self.latest_run(&id).cloned();
                    self.open_run(run);
                }
                KeyCode::Char('d') | KeyCode::Delete => self.ask_delete(),
                _ => {}
            },
            _ => {}
        }
    }

    // ---- mouse ------------------------------------------------------------

    /// Dragging in the editor marks text.
    fn drag(&mut self, x: u16, y: u16) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let Some((rect, _)) = self.hits.iter().find(|(_, h)| *h == Hit::Text).copied() else {
            return;
        };
        if let Some(ed) = self.editor.as_mut() {
            ed.text
                .drag(y as i32 - rect.y as i32, x.saturating_sub(rect.x) as usize);
        }
    }

    /// A right click on a row of a list selects it and opens its menu; a
    /// right click while a menu is open closes it.
    pub fn right_click(&mut self, x: u16, y: u16) {
        if matches!(self.mode, Mode::Menu(_)) {
            self.mode = Mode::Normal;
            return;
        }
        if !matches!(self.mode, Mode::Normal) || self.form.is_some() || self.editor.is_some() {
            return;
        }
        let row = self.hits.iter().rev().find_map(|(r, h)| match h {
            Hit::Row(i) if x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height => {
                Some(*i)
            }
            _ => None,
        });
        let Some(i) = row else { return };
        *self.cursor() = i;
        let items = self.menu_items();
        if !items.is_empty() {
            self.mode = Mode::Menu(Menu {
                x,
                y,
                items,
                sel: 0,
            });
        }
    }

    /// What the menu offers for the row under the cursor.
    fn menu_items(&self) -> Vec<MenuItem> {
        let item = |label: &str, key: &'static str, action: MenuAction| MenuItem {
            label: label.into(),
            key,
            action,
            danger: false,
        };
        let mut v = Vec::new();
        if let Some(r) = self.selected_routine() {
            v.push(item("Edit", "⏎", MenuAction::Key('\n')));
            v.push(item(
                if r.is_shell() {
                    "Write the command"
                } else {
                    "Write the prompt"
                },
                "p",
                MenuAction::Key('p'),
            ));
            if !r.approved && !r.unreadable {
                v.push(item("Approve…", "a", MenuAction::Key('a')));
            } else {
                v.push(item("Run now…", "r", MenuAction::Key('r')));
            }
            v.push(item(
                if r.file.enabled {
                    "Switch off"
                } else {
                    "Switch on"
                },
                "space",
                MenuAction::Key(' '),
            ));
            v.push(item("Duplicate", "c", MenuAction::Key('c')));
            if let Some(run) = self.latest_run(&r.id) {
                if !run.tab_closed && run.cron_pid == 0 {
                    v.push(item("Go to its last run's tab", "o", MenuAction::Key('o')));
                }
                v.push(item("Output of its last run", "l", MenuAction::Key('l')));
                v.push(item("Its runs", "h", MenuAction::Key('h')));
                if run.status.active() {
                    v.push(MenuItem {
                        danger: true,
                        ..item("Stop its run…", "x", MenuAction::Key('x'))
                    });
                }
            }
            v.push(MenuItem {
                danger: true,
                ..item("Delete…", "d", MenuAction::Key('d'))
            });
        } else if self.selected_cron_job().is_some() {
            for (key, how, label, _) in TAKES {
                let key: &'static str = match key {
                    'k' => "k",
                    'm' => "m",
                    _ => "c",
                };
                v.push(item(label, key, MenuAction::Take(how)));
            }
        } else if let (View::Runs, Some(run)) = (self.view, self.runs.get(self.sel_run)) {
            let open = !run.tab_closed && run.cron_pid == 0 && !run.pane_id.is_empty();
            v.push(item("Open", "⏎", MenuAction::Key('\n')));
            if open {
                v.push(item("Go to its tab", "o", MenuAction::Key('o')));
            }
            v.push(item("Its output", "l", MenuAction::Key('l')));
            v.push(item("Open the output file", "f", MenuAction::Key('f')));
            if self.routines.iter().any(|r| r.id == run.routine_id) {
                v.push(item("Run its routine again…", "r", MenuAction::Key('r')));
            }
            if run.status.active() {
                v.push(MenuItem {
                    danger: true,
                    ..item("Stop this run…", "x", MenuAction::Key('x'))
                });
            }
            v.push(item(
                if self.run_filter.is_some() {
                    "All runs"
                } else {
                    "Only its runs"
                },
                "h",
                MenuAction::Key('h'),
            ));
            if !run.status.active() {
                v.push(MenuItem {
                    danger: true,
                    ..item("Delete this run…", "d", MenuAction::Key('d'))
                });
            }
        }
        v
    }

    /// Does what a menu entry says.
    fn menu_pick(&mut self, action: MenuAction) {
        self.mode = Mode::Normal;
        match action {
            MenuAction::Key(c) => {
                let code = match c {
                    '\n' => KeyCode::Enter,
                    c => KeyCode::Char(c),
                };
                self.key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            MenuAction::Take(how) => {
                if let Some(job) = self.selected_cron_job().cloned() {
                    self.ask_import(vec![job], how);
                }
            }
        }
    }

    /// The mouse over a menu or a list to choose from marks the entry
    /// under it, as Herdr's own menus do; over a list it lights the row.
    fn hover(&mut self, x: u16, y: u16) {
        self.mouse = Some((x, y));
        let under = self.hit_at(x, y).map(|(_, h)| h);
        match (&mut self.mode, under) {
            (Mode::Menu(m), Some(Hit::MenuRow(i))) if i < m.items.len() => m.sel = i,
            (Mode::Pick(p), Some(Hit::PickRow(i))) if i < p.items.len() => p.sel = i,
            (Mode::Cron(_), Some(Hit::Take(i))) if i < TAKES.len() => self.take_sel = i,
            _ => {}
        }
    }

    /// The clickable area under the mouse; the one drawn last wins.
    fn hit_at(&self, x: u16, y: u16) -> Option<(Rect, Hit)> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .copied()
    }

    /// One mouse model everywhere: a click on something that only does one
    /// thing (a button, a key in the footer, an entry of a menu or of a
    /// list to choose from) does it; a click on a row or a field marks it,
    /// and a click on the marked one opens it; a click beside a dialog
    /// closes it. While a dialog is open only the dialog and its keys in
    /// the footer can be clicked (`ui::draw` drops everything else).
    pub fn click(&mut self, x: u16, y: u16) {
        self.mouse = Some((x, y));
        self.seen_flash();
        let normal = matches!(self.mode, Mode::Normal);
        let Some((rect, hit)) = self.hit_at(x, y) else {
            let d = self.dialog;
            // No dialog on screen (the question in the footer): anywhere.
            let beside =
                d.is_none_or(|d| x < d.x || x >= d.x + d.width || y < d.y || y >= d.y + d.height);
            if !normal && beside {
                self.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            return;
        };
        let double = self
            .last_click
            .is_some_and(|(at, h)| h == hit && at.elapsed() < Duration::from_millis(400));
        self.last_click = Some((Instant::now(), hit));
        match hit {
            Hit::View(v) => {
                if normal {
                    self.show(v);
                }
            }
            Hit::Row(i) => {
                if normal {
                    // The list stays where it is: a click never scrolls.
                    if *self.cursor() == i {
                        self.enter();
                    } else {
                        *self.cursor() = i;
                    }
                }
            }
            Hit::RunsOf(i) => {
                if normal {
                    *self.cursor() = i;
                    self.runs_of_selected();
                }
            }
            Hit::PickRow(i) => {
                if let Mode::Pick(mut p) = self.mode.clone() {
                    if i < p.items.len() {
                        p.sel = i;
                        self.picked(p);
                    }
                }
            }
            Hit::Field(i) => {
                if !normal {
                    return;
                }
                let Some(form) = self.form.as_mut() else {
                    return;
                };
                let again = form.field == i;
                if !again {
                    form.go(i);
                }
                let f = form.current();
                // Rows that do one thing act at once; a choice opens on
                // the click after the one that marked it.
                if matches!(f, Field::Save | Field::Prompt | Field::More) || (again && !f.is_text())
                {
                    self.form_enter();
                }
            }
            Hit::Cycle(i, by) => {
                if let (true, Some(form)) = (normal, self.form.as_mut()) {
                    if form.field != i {
                        form.go(i);
                    }
                    form.cycle(i32::from(by));
                }
            }
            Hit::Take(i) => {
                if let (Mode::Cron(jobs), Some(take)) = (self.mode.clone(), TAKES.get(i)) {
                    self.ask_import(jobs, take.1);
                }
            }
            Hit::Day(day) => {
                if let (true, Some(form)) = (normal, self.form.as_mut()) {
                    form.select(Field::Days);
                    form.toggle_day(day);
                }
            }
            Hit::Unfilter(which) => {
                if normal {
                    self.unfilter(which);
                }
            }
            Hit::Attention(kind) => {
                if normal {
                    self.attention_click(kind);
                }
            }
            // From the footer: a click is never typed into a field.
            Hit::Key('?') if normal => {
                self.help_scroll = 0;
                self.mode = Mode::Help;
            }
            Hit::Key(c) => {
                let code = match c {
                    '\n' => KeyCode::Enter,
                    '\u{1b}' => KeyCode::Esc,
                    '\t' => KeyCode::Tab,
                    c => KeyCode::Char(c),
                };
                self.key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            Hit::Ctrl(c) => {
                self.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
            }
            Hit::Text => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.text.place((y - rect.y) as usize, (x - rect.x) as usize);
                    if double {
                        ed.text.select_word();
                    }
                }
            }
            Hit::MenuRow(i) => {
                if let Mode::Menu(m) = self.mode.clone() {
                    if let Some(item) = m.items.get(i) {
                        self.menu_pick(item.action);
                    }
                }
            }
            Hit::Link => {
                self.mode = Mode::Normal;
                if (self.opener)(OPENER, env!("CARGO_PKG_REPOSITORY")) {
                    self.flash("Opened in the browser", GREEN);
                } else {
                    self.flash(format!("Could not run {OPENER}"), RED);
                }
            }
        }
    }

    fn wheel(&mut self, by: i32) {
        match self.mode.clone() {
            Mode::Help | Mode::Message { .. } => self.scroll_help(by * 2),
            Mode::Pick(mut p) => {
                p.sel = (p.sel as i32 + by)
                    .clamp(0, p.items.len() as i32 - 1)
                    .max(0) as usize;
                self.mode = Mode::Pick(p);
            }
            Mode::Log(mut log) => {
                log.scroll = (log.scroll as i32 + by * 3).clamp(0, log.max as i32) as u16;
                self.mode = Mode::Log(log);
            }
            Mode::Normal => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.text.scroll(by * 2);
                } else if let Some(form) = self.form.as_mut() {
                    form.move_by(by);
                } else {
                    // The view moves, the cursor stays on its row; drawing
                    // keeps the offset inside the list.
                    let top = &mut self.tops[App::list_index(self.view)];
                    *top = (*top as i32 + by).max(0) as usize;
                    self.follow = false;
                }
            }
            _ => {}
        }
    }

    pub fn event(&mut self, e: Event) {
        match e {
            Event::Key(k) => self.key(k),
            Event::Paste(text) => self.paste(&text),
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => self.click(m.column, m.row),
                MouseEventKind::Down(MouseButton::Right) => self.right_click(m.column, m.row),
                MouseEventKind::Drag(MouseButton::Left) => self.drag(m.column, m.row),
                MouseEventKind::Moved => self.hover(m.column, m.row),
                MouseEventKind::ScrollDown => self.wheel(1),
                MouseEventKind::ScrollUp => self.wheel(-1),
                _ => {}
            },
            _ => {}
        }
    }

    fn paste(&mut self, text: &str) {
        let line = || -> String {
            text.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_string()
        };
        match self.mode.clone() {
            Mode::Input(mut input) => {
                input.text.push_str(&line());
                input.text = input.text.chars().take(80).collect();
                self.mode = Mode::Input(input);
            }
            Mode::Normal => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.text.paste(text);
                    ed.text.follow();
                } else if let Some(form) = self.form.as_mut() {
                    form.paste(&line());
                }
            }
            _ => {}
        }
    }

    fn scroll_help(&mut self, by: i32) {
        self.help_scroll = (self.help_scroll as i32 + by).clamp(0, self.help_max as i32) as u16;
    }

    /// The note on how the list works, once, on first open.
    fn show_intro(&mut self) {
        let seen = self.paths.state.join("intro-seen");
        if seen.exists() || !self.routines.is_empty() {
            self.ask_autostart_once();
            return;
        }
        if paths::write_atomic(&seen, b"").is_ok() {
            self.mode = Mode::Message {
                title: "How routines work".into(),
                lines: INTRO.iter().map(|l| l.to_string()).collect(),
            };
            // Closing the note asks next.
            self.autostart_after_intro = true;
        }
    }

    /// Asks once, on the first open, whether Herdr should start at login.
    fn ask_autostart_once(&mut self) {
        let asked = self.paths.state.join("autostart-asked");
        if asked.exists() || !crate::autostart::supported() || crate::autostart::is_on() {
            return;
        }
        if paths::write_atomic(&asked, b"").is_ok() {
            self.ask_autostart();
        }
    }

    /// Sends the editor's text through the person's own editor ($VISUAL,
    /// $EDITOR or nano) and takes back what it saved.
    fn edit_outside(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let Some(text) = self.editor.as_ref().map(|e| e.text.text()) else {
            return Ok(());
        };
        // A file of this list alone: two open lists never share one.
        let file = self
            .paths
            .state
            .join(format!("editing-{}.md", std::process::id()));
        if let Err(e) = paths::write_atomic(&file, text.as_bytes()) {
            self.flash(format!("Cannot open the editor: {e}"), RED);
            return Ok(());
        }
        let editor = ["VISUAL", "EDITOR"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|e| !e.trim().is_empty())
            .unwrap_or_else(|| "nano".into());
        crate::leave_terminal();
        let status = Command::new("sh")
            .args(["-c", &format!("{editor} \"$1\""), "sh"])
            .arg(&file)
            .status();
        *terminal = crate::enter_terminal();
        terminal.clear()?;
        match status {
            Ok(s) if s.success() => {
                let back = std::fs::read_to_string(&file).unwrap_or(text.clone());
                if back != text {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.text.replace(&back);
                    }
                }
            }
            Ok(_) => self.flash(format!("{editor} ended with an error"), RED),
            Err(e) => self.flash(format!("Cannot open {editor}: {e}"), RED),
        }
        let _ = std::fs::remove_file(&file);
        Ok(())
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        self.show_intro();
        while !self.quit {
            if self.last_load.elapsed() >= RELOAD {
                self.reload();
            }
            if self
                .flash
                .as_ref()
                .is_some_and(|f| !f.sticky && Instant::now() >= f.until)
            {
                self.flash = None;
            }
            terminal.draw(|frame| ui::draw(frame, &mut self))?;
            if std::mem::take(&mut self.external) {
                self.edit_outside(terminal)?;
                continue;
            }
            if event::poll(Duration::from_millis(250))? {
                self.event(event::read()?);
            }
        }
        Ok(())
    }
}

/// `text` with `key = value` set: the line replaced, or added at the end.
pub fn with_value(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            let k = l
                .split('#')
                .next()
                .unwrap_or("")
                .split('=')
                .next()
                .unwrap_or("");
            if !found && l.contains('=') && k.trim() == key {
                found = true;
                line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.push(line);
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// Starts `opener` (`open`, `xdg-open`) on `url`, waited on in the
/// background so the list never waits for it.
pub fn open_in_browser(opener: &str, url: &str) -> bool {
    match Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// Text from a file as it can be shown safely: no control characters (tabs
/// stay) and none of the invisible characters that turn text around or hide
/// it (bidi overrides, zero-width marks), so a log or prompt cannot look
/// different from what it is.
pub fn visible_text(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let invisible = matches!(
                c,
                '\u{061C}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
            );
            !invisible && (!c.is_control() || *c == '\t')
        })
        .collect()
}

/// The approval view: every setting of the routine and its whole prompt
/// or command, as shown text.
fn approval_lines(paths: &Paths, r: &Routine) -> Vec<String> {
    let f = &r.file;
    let mut lines = vec![
        "It was created or changed outside the list (by an agent or an editor). It runs on schedule only once you approve exactly what follows.".to_string(),
        String::new(),
        format!("Name: {}", f.name),
        format!(
            "Schedule: {}{}",
            r.schedule_text(),
            if f.enabled { "" } else { " (switched off)" }
        ),
        format!("When a time is missed: {}", missed_words(f)),
        format!("Folder: {}", f.folder),
        format!(
            "Time limit: {} minutes{}",
            f.time_limit_minutes,
            if f.stop_at_limit { ", then it is stopped" } else { "" }
        ),
    ];
    if f.alert != crate::routine::Alert::Notification {
        lines.push(format!("Tells you by: {}", f.alert.detail()));
    }
    if f.retries > 0 {
        lines.push(format!(
            "Retries: {} after a failure, {} minutes apart",
            f.retries, f.retry_delay_minutes
        ));
    }
    if f.more.off_after_failures > 0 {
        lines.push(format!(
            "Switches itself off: after {} runs in a row that went wrong",
            f.more.off_after_failures
        ));
    }
    lines.push(format!(
        "Keeps: {}",
        crate::form::keep_text(f.keep_runs, f.keep_days).1
    ));
    if !f.more.after_command.trim().is_empty() {
        lines.push(format!(
            "After {} (a shell command, as you, without asking, in the folder above): {}",
            if f.more.after_always {
                "every run"
            } else {
                "a run that went wrong"
            },
            f.more.after_command.trim()
        ));
    }
    if r.is_cron() {
        lines.push("Runs: a shell command, as you, without asking, as a classic cron job".into());
        lines.push(
            "Approving writes its line into your crontab. The system's cron then starts it, also when Herdr is closed, in your home folder (not in the folder above).".into(),
        );
    } else if r.is_shell() {
        lines.push("Runs: a shell command, as you, without asking".into());
    } else {
        if !f.check_command.trim().is_empty() {
            lines.push(format!(
                "Check first (a shell command, as you): {}",
                f.check_command
            ));
        }
        if f.remember {
            lines.push("Remembers the results of its last ten runs".into());
        }
        let mode = if f.permission_mode.is_empty() {
            "agent default".to_string()
        } else {
            agent::label(agent::modes(&f.agent), &f.permission_mode)
        };
        lines.push(format!("Agent: {} · {mode}", f.agent));
        if !f.more.account.is_empty() {
            lines.push(format!(
                "Account: {} (Multiaccount id {}), used also when it is at its limit",
                crate::accounts::name_of(paths, &f.more.account),
                f.more.account
            ));
        }
        let flags = agent::flags(f);
        if !flags.is_empty() {
            lines.push(format!("Options: {}", flags.join(" ")));
        }
        if !f.args.is_empty() {
            lines.push(format!(
                "Warning: it has extra arguments ({}). They go to {} as they are and can change what it may do or load (servers, plugins, settings). Approve only if you know each of them.",
                crate::form::join_args(&f.args),
                f.agent
            ));
        }
    }
    lines.push(String::new());
    lines.push(if r.is_shell() { "Command:" } else { "Prompt:" }.into());
    if r.prompt.trim().is_empty() {
        lines.push("  (empty)".into());
    }
    lines.extend(r.prompt.lines().map(|l| format!("  {l}")));
    if let Some(p) = &r.problem {
        lines.push(String::new());
        lines.push(format!("It cannot run yet: {p}"));
    }
    lines.push(String::new());
    lines.push("y approves exactly this. Esc leaves it waiting.".into());
    lines.iter().map(|l| visible_text(l)).collect()
}

/// What happens with a missed time, with the hours a catch-up run may be
/// late.
pub fn missed_words(f: &routine::RoutineFile) -> String {
    match f.missed {
        routine::Missed::RunOnce => crate::form::late_text(f.more.late_hours).1,
        missed => missed.detail().to_string(),
    }
}

/// What Claude is told when it creates a routine with the person.
pub fn agent_brief(paths: &Paths) -> String {
    let dir = paths::shorten(&paths.routines_dir());
    let bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "herdr-routines".into());
    format!(
        r#"Help me create a scheduled routine for the Herdr plugin herdr-routines. Talk to me in the language I write in, short and plain; I may not be a developer.

How routines work: each routine is a folder {dir}/<id>/ (id: lowercase words joined by dashes, unique). It holds routine.toml and either prompt.md (what the agent is told) or, for agent = "shell", command.sh (a shell command or script, run with my $SHELL in the folder; exit code 0 counts as finished). A scheduler reads these files every few seconds and opens a tab for each run at the scheduled time. Nobody watches a run while it works.

routine.toml:
name = "Weekly dependency check"
enabled = true
schedule = "0 7 * * 1"            # cron, 5 fields, local time: minute hour day month weekday
agent = "claude"                  # claude, codex, gemini, opencode, any kind Herdr knows, or "shell"
permission_mode = "auto"          # optional. claude: auto, acceptEdits, plan, dontAsk, bypassPermissions, manual. codex: auto, read-only, full-access. gemini: auto_edit, yolo
model = "sonnet"                  # optional. claude: fable, opus, sonnet, haiku; others: a model name they know
effort = "medium"                 # optional, claude only: low, medium, high, xhigh, max
args = []                         # extra command line options for the agent, usually empty; never for shell
folder = "~/code/app"             # where it runs
missed = "run-once"               # when the computer was off at the time: "run-once" or "skip"
late_hours = 0                    # with run-once: catch up only when less than this many hours late (0: however late, up to a week)
time_limit_minutes = 120
stop_at_limit = false             # true: close the run's tab at the time limit
retries = 0                       # try a failed run again this many times (0-3)...
retry_delay_minutes = 10          # ...this many minutes apart
off_after_failures = 0            # switch the routine off after this many runs in a row that went wrong (0: never)
after_command = ""                # optional: one line of shell run after a run that went wrong, in the routine's folder, at most 5 minutes. It gets HERDR_ROUTINES_STATUS (finished, failed, timed-out, closed, no-result), _NAME, _ID, _RUN, _MESSAGE, _RESULT, _OUTPUT and _STARTED. Example: "~/bin/tell-me.sh"
after_always = false              # true: run after_command after every run, not only one that went wrong
check_command = ""                # agents only: a shell command run first; the agent starts only if it exits 0, and gets its output
remember = false                  # agents only: each run gets the results of the last ten runs
account = ""                      # claude only, and only if I use the Multiaccount plugin and ask for it: the id of the account there; empty lets Multiaccount decide
runs_in = "herdr"                 # shell only: "herdr" (a tab, while Herdr runs) or "cron" (a cron job in my crontab, also when Herdr is closed; command.sh is then one line, run in my home folder; the list writes the crontab line once I approve it)
keep_runs = 30                    # runs kept in the history (0: no limit)...
keep_days = 90                    # ...and none older than this many days (0: no limit)
alert = "notification"            # "notification", "popup-on-problems" (a popup that stays until OK when a run fails, runs too long or needs me) or "popup" (also when it finishes)

Other forms of schedule: "@once YYYY-MM-DD HH:MM" (runs once, then the routine switches off); "@start" (each time Herdr starts); "@every 2 weeks from YYYY-MM-DD HH:MM" or "@every 3 days from YYYY-MM-DD HH:MM" (every N weeks or days, counted from that first time); "@hourly", "@daily", "@weekly", "@monthly", "@yearly". In cron, the day of the month can be "L" (the last day) or "LW" (the last working day), and the weekday can be "mon#1" to "mon#5" (the first to fifth Monday of the month) or "friL" (the last Friday). A cron job (runs_in = "cron") takes only plain 5-field cron.

What to do:
1. Ask me what the routine should do, how often and when, and in which folder, unless I already said it. Ask everything you need in one short message. Suggest sensible choices (a cheaper model for simple jobs, "auto" so nobody has to approve steps, a plain shell command when no AI is needed).
2. For an agent routine, write prompt.md as clear instructions for an agent working alone: what to do, where, what to report at the end, and what it must not do. For a shell routine, write command.sh.
3. Show me a short summary (name, schedule in words, agent, model, folder) and create the files once I agree. Do not change or delete other routines unless I ask.
4. Run `{bin} routines` and check that the new routine shows "problem": null; fix the files if not. It shows "approved": false, which is expected: a routine that was not saved in the list runs only after I approve it there. Do not try to change that. Then tell me it is ready and that I approve it in the list (Ctrl+B, U, then a on the routine)."#
    )
}

/// Regression tests for the approval rules and the crontab, in their own
/// file (`app_security_tests.rs`).
#[cfg(test)]
#[path = "app_security_tests.rs"]
mod security_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn relative_times() {
        let now = Local.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        assert_eq!(
            ago(&(now - chrono::Duration::minutes(5)), &now),
            "5 min ago"
        );
        assert_eq!(ago(&(now - chrono::Duration::hours(3)), &now), "3 h ago");
        assert_eq!(ago(&(now - chrono::Duration::days(2)), &now), "2 d ago");
        assert_eq!(
            until(&(now + chrono::Duration::minutes(12)), &now),
            "in 12 min"
        );
        assert_eq!(until(&(now + chrono::Duration::hours(3)), &now), "15:00");
    }

    #[test]
    fn config_values_are_replaced_or_added() {
        assert_eq!(
            with_value("a = 1\nicons = \"nerd\" # x\n", "icons", "\"plain\""),
            "a = 1\nicons = \"plain\"\n"
        );
        assert_eq!(with_value("", "icons", "\"plain\""), "icons = \"plain\"\n");
    }

    fn opens(_: &str, _: &str) -> bool {
        true
    }

    /// An app on made-up folders with two routines and one run.
    fn app(tag: &str) -> App {
        let dir =
            std::env::temp_dir().join(format!("herdr-routines-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        };
        std::fs::create_dir_all(&paths.state).unwrap();
        // By name, so the tests know which routine is where; the order by
        // what needs the owner has tests of its own.
        std::fs::write(paths.state.join("list-sort"), "name").unwrap();
        for (i, name) in ["Morning report", "Weekly cleanup"].iter().enumerate() {
            let mut f = Form::new("/tmp");
            f.name = name.to_string();
            f.prompt = format!("Do task {i}.\nThen stop.");
            let (file, prompt) = f.build().unwrap();
            let r = Routine {
                id: routine::new_id(&paths, name),
                file,
                prompt,
                problem: None,
                unreadable: false,
                approved: false,
            };
            // The first is saved in the list; the second written by an agent.
            if i == 0 {
                r.save_approved(&paths).unwrap();
            } else {
                r.save(&paths).unwrap();
            }
        }
        let run = Run::new(
            "morning-report",
            "Morning report",
            "claude",
            store::Trigger::Manual,
            None,
        );
        run.save(&paths).unwrap();
        let mut a = App::new(paths);
        a.opener = opens;
        a
    }

    fn screen(a: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| ui::draw(f, a)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    fn press(a: &mut App, code: KeyCode) {
        a.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn draws_every_view_at_many_sizes() {
        let mut a = app("sizes");
        for (w, h) in [(30, 8), (60, 14), (80, 24), (120, 40), (200, 60)] {
            a.form = None;
            a.editor = None;
            a.mode = Mode::Normal;
            a.view = View::Routines;
            screen(&mut a, w, h);
            a.view = View::Runs;
            screen(&mut a, w, h);
            a.mode = Mode::Help;
            screen(&mut a, w, h);
            a.mode = Mode::Normal;
            a.view = View::Routines;
            press(&mut a, KeyCode::Enter);
            assert!(a.form.is_some());
            screen(&mut a, w, h);
            a.form.as_mut().unwrap().select(Field::Prompt);
            screen(&mut a, w, h);
            press(&mut a, KeyCode::Enter);
            assert!(a.editor.is_some(), "⏎ on the prompt opens the editor");
            screen(&mut a, w, h);
            a.editor = None;
            a.form.as_mut().unwrap().select(Field::Agent);
            press(&mut a, KeyCode::Enter);
            assert!(matches!(a.mode, Mode::Pick(_)));
            screen(&mut a, w, h);
            a.mode = Mode::AskQuit;
            screen(&mut a, w, h);
        }
    }

    #[test]
    fn the_list_shows_routines_and_keys() {
        let mut a = app("list");
        let rows = screen(&mut a, 120, 30);
        let all = rows.join("\n");
        assert!(all.contains("Morning report"), "{all}");
        assert!(all.contains("Weekly cleanup"));
        assert!(all.contains("Weekdays at 09:00"));
        assert!(all.contains("? all keys"));
        a.view = View::Runs;
        let all = screen(&mut a, 120, 30).join("\n");
        assert!(all.contains("By hand"), "{all}");
    }

    /// A stand-in for `crontab` on this test's thread: -l prints the file,
    /// a path replaces it. Returns the file.
    fn fake_crontab(a: &App, text: &str) -> std::path::PathBuf {
        let dir = a.paths.state.clone();
        let tab = dir.join("fake-crontab.txt");
        std::fs::write(&tab, text).unwrap();
        let fake = dir.join("crontab");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -l ]; then cat '{0}'; else cp \"$1\" '{0}'; fi\n",
                tab.display()
            ),
        )
        .unwrap();
        std::process::Command::new("chmod")
            .args(["+x", &fake.to_string_lossy()])
            .status()
            .unwrap();
        crate::crontab::use_fake(Some(&fake));
        tab
    }

    #[test]
    fn cron_jobs_show_in_the_list_and_move_to_herdr() {
        let mut a = app("cron");
        let tab = fake_crontab(
            &a,
            "PATH=/usr/bin:/bin\n0 2 * * * /opt/jobs/backup.sh --all\n@reboot ~/start.sh\n",
        );
        a.crontab_read = None;
        a.reload();
        // The crontab's jobs show in their own tab, not among the routines.
        assert_eq!(a.crontab_jobs.len(), 2);
        let lines = screen(&mut a, 120, 40);
        assert!(!lines.iter().any(|l| l.contains("only in your crontab")));
        assert!(lines.iter().any(|l| l.contains("Cron jobs (2)")));
        press(&mut a, KeyCode::Char('2'));
        assert_eq!(a.view, View::Cron);
        let lines = screen(&mut a, 120, 40);
        assert!(lines
            .iter()
            .any(|l| l.contains("backup.sh --all") && l.contains("Daily at 02:00")));
        assert!(lines.iter().any(|l| l.contains("only in your crontab")));
        assert!(a.selected_cron_job().is_some());
        assert!(lines.iter().any(|l| l.contains("⏎ takes it over")));
        press(&mut a, KeyCode::Char('C'));
        let Mode::Cron(jobs) = a.mode.clone() else {
            panic!("no cron list: {:?}", a.flash);
        };
        assert_eq!(jobs.len(), 2);
        let lines = screen(&mut a, 100, 30);
        assert!(lines
            .iter()
            .any(|l| l.contains("Take over 2 cron jobs from your crontab")));
        assert!(lines.iter().any(|l| l.contains("When the computer starts")));
        // Moving changes the crontab, so it asks first; any other key
        // leaves everything as it is.
        press(&mut a, KeyCode::Char('m'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("moving does not ask");
        };
        assert_eq!(c.question, "Move 2 cron jobs to Herdr?");
        press(&mut a, KeyCode::Char('n'));
        assert_eq!(a.crontab_jobs.len(), 2, "nothing moved");
        assert!(std::fs::read_to_string(&tab)
            .unwrap()
            .contains("\n0 2 * * * /opt/jobs"));
        // ⏎ on the marked way chooses it, and ⏎ confirms the question.
        press(&mut a, KeyCode::Char('C'));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::Confirm(_)));
        press(&mut a, KeyCode::Enter);
        let moved: Vec<&Routine> = a
            .routines
            .iter()
            .filter(|r| r.file.agent == routine::SHELL)
            .collect();
        assert_eq!(moved.len(), 2, "{:?}", a.flash);
        assert!(moved
            .iter()
            .all(|r| r.file.enabled && r.problem.is_none() && !r.is_cron()));
        assert!(moved.iter().any(|r| r.file.name == "backup.sh --all"));
        let now = std::fs::read_to_string(&tab).unwrap();
        assert!(now.contains("# moved to herdr-routines (routine "), "{now}");
        assert!(crate::crontab::parse(&now).is_empty());
        assert!(a.crontab_jobs.is_empty(), "the list shows them as routines");
        assert!(std::fs::read_dir(a.paths.state.join("crontab-backups"))
            .unwrap()
            .flatten()
            .next()
            .is_some());
        crate::crontab::use_fake(None);
    }

    #[test]
    fn cron_jobs_kept_in_cron_are_managed_here() {
        let mut a = app("keepcron");
        let tab = fake_crontab(&a, "MAILTO=\"\"\n*/5 * * * * echo hi >> /tmp/x.log\n");
        a.crontab_read = None;
        a.reload();
        a.view = View::Cron;
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::Cron(_)));
        press(&mut a, KeyCode::Char('k'));
        assert!(matches!(a.mode, Mode::Confirm(_)), "keeping asks first");
        press(&mut a, KeyCode::Char('y'));
        assert_eq!(a.view, View::Cron);
        assert_eq!(a.listed().len(), 1, "now one of this plugin's cron jobs");
        let job = a
            .routines
            .iter()
            .find(|r| r.is_cron())
            .cloned()
            .unwrap_or_else(|| panic!("{:?}", a.flash));
        assert!(job.file.enabled && job.approved && job.problem.is_none());
        let now = std::fs::read_to_string(&tab).unwrap();
        // The line stays where it was, as written; the marker and SHELL go
        // around it, so cron starts it through the wrapper.
        let wrapper = crate::crontab::wrapper(&a.paths, &job.id);
        assert_eq!(
            now,
            format!(
                "MAILTO=\"\"\n# herdr-routines: {} (its name and runs are in the herdr-routines list)\nSHELL={}\n*/5 * * * * echo hi >> /tmp/x.log\nSHELL=/bin/sh\n",
                job.id,
                wrapper.display()
            )
        );
        assert!(crate::crontab::parse(&now).is_empty(), "the job is managed");
        assert_eq!(
            std::fs::read_link(&wrapper).unwrap(),
            crate::crontab::binary(&a.paths)
        );
        assert!(crate::crontab::binary(&a.paths).exists());
        // Off: the line is commented out with #off; on again: it is back.
        a.select_id(&job.id);
        press(&mut a, KeyCode::Char(' '));
        let now = std::fs::read_to_string(&tab).unwrap();
        assert!(
            now.contains("\n#off */5 * * * * echo hi >> /tmp/x.log\n"),
            "{now}"
        );
        press(&mut a, KeyCode::Char(' '));
        let now = std::fs::read_to_string(&tab).unwrap();
        assert!(
            now.contains("\n*/5 * * * * echo hi >> /tmp/x.log\n"),
            "{now}"
        );
        // A name given in the list stays in the plugin, not the crontab.
        let mut named = Routine::load(&a.paths, &job.id).unwrap();
        named.file.name = "Say hi".into();
        named.save_approved(&a.paths).unwrap();
        a.crontab_read = None;
        a.reload();
        assert!(!std::fs::read_to_string(&tab).unwrap().contains("Say hi"));
        // Edited by hand in the crontab: the crontab wins.
        let edited = std::fs::read_to_string(&tab)
            .unwrap()
            .replace("*/5 * * * * echo hi", "0 7 * * * echo hello");
        std::fs::write(&tab, edited).unwrap();
        a.crontab_read = None;
        a.reload();
        let now = Routine::load(&a.paths, &job.id).unwrap();
        assert_eq!(now.file.schedule, "0 7 * * *");
        assert_eq!(now.prompt, "echo hello >> /tmp/x.log");
        assert_eq!(now.file.name, "Say hi");
        assert!(now.approved && now.file.enabled);
        // Deleting it removes the whole group, line included.
        press(&mut a, KeyCode::Char('d'));
        press(&mut a, KeyCode::Char('y'));
        let now = std::fs::read_to_string(&tab).unwrap();
        assert_eq!(now, "MAILTO=\"\"\n");
        assert!(!wrapper.exists());
        crate::crontab::use_fake(None);
    }

    #[test]
    fn a_new_cron_job_goes_into_the_crontab_and_runs_now_through_the_wrapper() {
        let mut a = app("newcron");
        let tab = fake_crontab(&a, "SHELL=/bin/bash\n0 1 * * * other\n");
        let mut f = Form::new("/tmp");
        f.name = "Stamp".into();
        f.choose(Field::Agent, routine::SHELL);
        f.choose(Field::RunsIn, crate::routine::RunsIn::Cron.label());
        assert!(!f.fields().contains(&Field::Folder), "cron runs in home");
        f.prompt = "date +%H%M >> /tmp/stamp.log".into();
        a.save_form(&f).unwrap();
        let r = a
            .routines
            .iter()
            .find(|r| r.is_cron())
            .cloned()
            .unwrap_or_else(|| panic!("{:?}", a.flash));
        let now = std::fs::read_to_string(&tab).unwrap();
        // Appended, with % escaped for cron and the shell set back after.
        assert!(
            now.ends_with(&format!(
                "SHELL={}\n0 9 * * 1-5 date +\\%H\\%M >> /tmp/stamp.log\nSHELL=/bin/bash\n",
                crate::crontab::wrapper(&a.paths, &r.id).display()
            )),
            "{now}"
        );
        // Several lines cannot be one crontab line.
        let mut f = Form::edit(&r);
        f.prompt = "echo a\necho b".into();
        assert!(f.build().unwrap_err().contains("one line"));
        // Its details show the crontab line and how many runs it keeps.
        a.view = View::Cron;
        a.select_id(&r.id);
        let lines = screen(&mut a, 100, 40);
        let all = lines.join("\n");
        assert!(
            all.contains("Crontab line    0 9 * * 1-5 and the command below"),
            "{all}"
        );
        assert!(
            all.contains("Command         date +%H%M >> /tmp/stamp.log"),
            "{all}"
        );
        assert!(
            all.contains("Run history     The last 30 runs, none older than 90 days"),
            "{all}"
        );
        // Run now asks, then starts it through its wrapper, not in a tab.
        press(&mut a, KeyCode::Char('r'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert!(c.lines[0].contains("in the background"), "{:?}", c.lines);
        a.mode = Mode::Normal;
        // The form: no folder, the history choice in its part.
        a.form = Some(Box::new(Form::edit(&r)));
        a.form.as_mut().unwrap().select(Field::Keep);
        let all = screen(&mut a, 100, 40).join("\n");
        assert!(!all.contains("Folder"), "{all}");
        assert!(
            all.contains("Run history        ‹ 30 runs, 90 days ›"),
            "{all}"
        );
        assert!(
            all.contains("Time limit, retries, alerts and run history"),
            "{all}"
        );
        crate::crontab::use_fake(None);
    }

    #[test]
    fn a_cron_run_is_recorded_with_its_output() {
        let a = app("cronrun");
        let mut f = Form::new("/tmp");
        f.name = "Say hi".into();
        f.choose(Field::Agent, routine::SHELL);
        f.choose(Field::RunsIn, crate::routine::RunsIn::Cron.label());
        f.prompt = "echo first; echo hi from cron".into();
        f.choose(Field::Keep, "2:90");
        let (file, prompt) = f.build().unwrap();
        let r = Routine {
            id: "say-hi".into(),
            file,
            prompt,
            problem: None,
            unreadable: false,
            approved: true,
        };
        r.save_approved(&a.paths).unwrap();
        // What cron starts: the wrapper link with -c and the command.
        crate::crontab::prepare_wrapper(&a.paths, "say-hi").unwrap();
        let wrapper = crate::crontab::wrapper(&a.paths, "say-hi");
        let args: Vec<String> = vec![
            wrapper.to_string_lossy().into(),
            "-c".into(),
            r.prompt.clone(),
        ];
        let (me, cmd) = crate::crontab::wrapper_call(&args).expect("seen as the wrapper");
        assert_eq!(crate::crontab::wrapper_main(&me, &cmd), 0);
        let runs = store::runs_of(&a.paths, "say-hi");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, store::Status::Finished);
        assert_eq!(runs[0].result, "hi from cron");
        assert!(runs[0].cron_pid != 0 && runs[0].tab_closed);
        let out = std::fs::read_to_string(runs[0].file(&a.paths, "out")).unwrap();
        assert!(out.contains("first") && out.contains("hi from cron"));
        // A failing command: recorded as failed, its exit code handed on.
        assert_eq!(crate::crontab::wrapper_main(&me, "echo oops; exit 3"), 3);
        let runs = store::runs_of(&a.paths, "say-hi");
        assert_eq!(runs[0].status, store::Status::Failed);
        assert_eq!(runs[0].result, "oops");
        // It keeps 2 runs: the third run deletes the oldest.
        assert_eq!(crate::crontab::wrapper_main(&me, &cmd), 0);
        assert_eq!(store::runs_of(&a.paths, "say-hi").len(), 2);
        // Switched off in the list: the next cron time is skipped.
        scheduler::set_paused(&a.paths, true).unwrap();
        assert_eq!(crate::crontab::wrapper_main(&me, &cmd), 0);
        assert_eq!(store::runs_of(&a.paths, "say-hi").len(), 2);
    }

    #[test]
    fn the_routines_chip_switches_all_of_them_after_asking() {
        let mut a = app("chip");
        a.scheduler = None;
        let lines = screen(&mut a, 120, 30);
        assert!(lines.iter().any(|l| l.contains("○ routines off")));
        let chip = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::Key('S')).then_some(*r))
            .expect("the chip is clickable");
        a.click(chip.x + 1, chip.y);
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert_eq!(c.question, "Switch routines on?");
        press(&mut a, KeyCode::Char('n'));
        a.scheduler = Some((1, String::new()));
        let lines = screen(&mut a, 120, 30);
        assert!(lines.iter().any(|l| l.contains("routines on")));
        press(&mut a, KeyCode::Char('S'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert_eq!(c.question, "Switch all routines off?");
    }

    #[test]
    fn a_new_cron_job_is_a_shell_command_in_cron() {
        let mut a = app("newcron-form");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        let form = a.form.as_ref().expect("the form");
        assert!(form.is_cron());
        let fields = form.fields();
        assert!(fields.contains(&Field::RunsIn));
        assert!(!fields.contains(&Field::Missed) && !fields.contains(&Field::AtLimit));
        assert!(!fields.contains(&Field::Mode));
        let lines = screen(&mut a, 100, 40);
        assert!(lines
            .iter()
            .any(|l| l.contains("Runs in") && l.contains("‹ Cron ›")));
    }

    #[test]
    fn a_right_click_opens_the_rows_menu() {
        let mut a = app("menu");
        screen(&mut a, 110, 34);
        let row = |a: &App, i: usize| {
            a.hits
                .iter()
                .find_map(|(r, h)| (*h == Hit::Row(i)).then_some(*r))
                .unwrap()
        };
        let second = row(&a, 1);
        a.right_click(second.x + 4, second.y);
        assert_eq!(a.sel_routine, 1, "the row is selected");
        let Mode::Menu(m) = a.mode.clone() else {
            panic!("no menu");
        };
        let labels: Vec<&str> = m.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Edit") && labels.contains(&"Duplicate"));
        assert!(m.items.last().unwrap().danger, "delete comes last, in red");
        let lines = screen(&mut a, 110, 34);
        assert!(lines
            .iter()
            .any(|l| l.contains("Switch off") && l.contains("space")));
        // Clicking an entry does what its key does.
        let copy = m.items.iter().position(|i| i.label == "Duplicate").unwrap();
        let at = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::MenuRow(copy)).then_some(*r))
            .unwrap();
        let before = a.routines.len();
        a.click(at.x + 1, at.y);
        assert_eq!(a.routines.len(), before + 1, "{:?}", a.flash);
        assert!(matches!(a.mode, Mode::Normal));
        // A click elsewhere only closes it; so does Esc.
        screen(&mut a, 110, 34);
        let first = row(&a, 0);
        a.right_click(first.x + 4, first.y);
        a.click(0, 0);
        assert!(matches!(a.mode, Mode::Normal));
        a.right_click(first.x + 4, first.y);
        press(&mut a, KeyCode::Esc);
        assert!(matches!(a.mode, Mode::Normal));
        // Runs have their own entries.
        press(&mut a, KeyCode::Char('3'));
        screen(&mut a, 110, 34);
        let run = row(&a, 0);
        a.right_click(run.x + 4, run.y);
        let Mode::Menu(m) = a.mode.clone() else {
            panic!("no menu on a run");
        };
        assert!(m.items.iter().any(|i| i.label == "Open the output file"));
    }

    #[test]
    fn deleting_a_routine_keeps_its_runs_unless_asked() {
        let mut a = app("delete");
        let name = |a: &App| a.selected_routine().unwrap().file.name.clone();
        a.sel_routine = a
            .routines
            .iter()
            .position(|r| r.file.name == "Morning report")
            .unwrap();
        let id = a.selected_routine().unwrap().id.clone();
        assert_eq!(name(&a), "Morning report");
        assert!(a.run_count(&id) > 0, "the fixture has a run");
        // The fixture's run is still starting; an ended one can be deleted.
        for mut run in store::runs_of(&a.paths, &id) {
            run.finish(store::Status::Finished, "");
            run.save(&a.paths).unwrap();
        }
        a.reload();
        press(&mut a, KeyCode::Char('d'));
        let lines = screen(&mut a, 100, 30);
        assert!(lines
            .iter()
            .any(|l| l.contains("y delete, keep runs") && l.contains("r delete with its runs")));
        press(&mut a, KeyCode::Char('y'));
        assert!(!a.routines.iter().any(|r| r.id == id));
        assert!(a.run_count(&id) > 0, "its runs stay");
        // They show as runs of a deleted routine, and `h` still filters.
        press(&mut a, KeyCode::Char('3'));
        let lines = screen(&mut a, 120, 30);
        assert!(lines.iter().any(|l| l.contains("deleted routine")));
        a.sel_run = a.runs.iter().position(|r| r.routine_id == id).unwrap();
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.run_filter.as_deref(), Some(id.as_str()));
        assert_eq!(a.filter_name(), Some("Morning report"));
        // A new routine of the same name does not take the old runs over.
        assert_ne!(routine::new_id(&a.paths, "Morning report"), id);
        // A kept run can be deleted on its own.
        press(&mut a, KeyCode::Char('d'));
        press(&mut a, KeyCode::Char('y'));
        assert_eq!(a.run_count(&id), 0);

        // With `r`, the runs go too.
        let mut b = app("delete-runs");
        b.sel_routine = b
            .routines
            .iter()
            .position(|r| r.file.name == "Morning report")
            .unwrap();
        let id = b.selected_routine().unwrap().id.clone();
        press(&mut b, KeyCode::Char('d'));
        press(&mut b, KeyCode::Char('r'));
        assert!(!b.routines.iter().any(|r| r.id == id));
        assert_eq!(b.run_count(&id), 0);
    }

    #[test]
    fn a_copy_starts_off_and_keeps_the_approval_state() {
        let mut a = app("copy");
        // The first routine was saved in the list, so its copy is approved.
        a.sel_routine = a
            .routines
            .iter()
            .position(|r| r.file.name == "Morning report")
            .unwrap();
        press(&mut a, KeyCode::Char('c'));
        let copy = a.selected_routine().unwrap().clone();
        assert_eq!(copy.file.name, "Morning report copy");
        assert!(!copy.file.enabled && copy.approved);
        assert_eq!(copy.prompt, "Do task 0.\nThen stop.");
        assert_eq!(a.run_count(&copy.id), 0);
        // A second copy gets its own name.
        a.sel_routine = a
            .routines
            .iter()
            .position(|r| r.file.name == "Morning report")
            .unwrap();
        press(&mut a, KeyCode::Char('c'));
        assert_eq!(
            a.selected_routine().unwrap().file.name,
            "Morning report copy 2"
        );
        // A copy of a routine that waits for approval waits too.
        a.sel_routine = a
            .routines
            .iter()
            .position(|r| r.file.name == "Weekly cleanup")
            .unwrap();
        press(&mut a, KeyCode::Char('c'));
        let copy = a.selected_routine().unwrap();
        assert!(!copy.approved && !copy.file.enabled);
    }

    #[test]
    fn a_tall_window_shows_the_selected_routines_details() {
        let mut a = app("details");
        a.sel_routine = 0;
        let name = a.routines[0].file.name.clone();
        let lines = screen(&mut a, 110, 34);
        let at = |s: &str| lines.iter().position(|l| l.contains(s));
        let head = at(&format!(" {name} ─")).expect("details heading");
        assert!(at(" Schedule        Weekdays").is_some_and(|i| i > head));
        assert!(at("Do task").is_some_and(|i| i > head), "the prompt shows");
        // The routines follow each other without empty rows (the line on
        // top names the next one too).
        let first = lines
            .iter()
            .position(|l| l.contains(&name) && !l.contains("next:"))
            .unwrap();
        let other = a.routines[1].file.name.clone();
        assert_eq!(at(&other).unwrap(), first + 2);
        // They show whenever rows are free below the list, also in a
        // small window.
        let lines = screen(&mut a, 80, 16);
        assert!(lines.iter().any(|l| l.contains(&format!(" {name} ─"))));
        // A window with no rows to spare shows only the list.
        let lines = screen(&mut a, 110, 9);
        assert!(!lines.iter().any(|l| l.contains(&format!(" {name} ─"))));
        assert!(lines.iter().any(|l| l.contains(&other)));
    }

    #[test]
    fn run_counts_link_to_the_routines_runs() {
        let mut a = app("filter");
        let ids: Vec<(String, String)> = a
            .routines
            .iter()
            .map(|r| (r.id.clone(), r.file.name.clone()))
            .collect();
        for (n, (id, name)) in ids.iter().enumerate() {
            for i in 0..=n {
                let mut run = Run::new(id, name, "shell", store::Trigger::Manual, None);
                run.id = format!("2026111{n}-08000{i}-000");
                run.finish(store::Status::Finished, "");
                run.result = format!("Result {n}.{i}");
                run.save(&a.paths).unwrap();
            }
        }
        a.reload();
        let lines = screen(&mut a, 140, 30);
        let count = a.run_count(&ids[1].0);
        assert!(lines.iter().any(|l| l.contains(&format!("{count} runs ›"))));
        let link = a
            .hits
            .iter()
            .rev()
            .find_map(|(r, h)| (*h == Hit::RunsOf(1)).then_some(*r))
            .unwrap();
        a.click(link.x + 1, link.y);
        assert_eq!(a.view, View::Runs);
        assert_eq!(a.runs.len(), count);
        assert!(a.runs.iter().all(|r| r.routine_id == ids[1].0));
        let lines = screen(&mut a, 140, 30);
        // The tab stays "Runs" with all of them; the filter is a line in the
        // view, with a button that removes it.
        let all = a.all_runs.len();
        assert!(
            lines[0].contains(&format!(" Runs ({all}) ")),
            "{}",
            lines[0]
        );
        assert!(lines.iter().any(|l| l
            .contains(&format!("Only  runs of {} ✕   {count} of {all}", ids[1].1))
            && l.contains("Esc back to the routine")
            && l.contains("h all runs")));
        assert!(lines.iter().any(|l| l.contains("Result 1.1")));
        // "h all runs" removes the filter and stays in Runs.
        let button = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::Key('h')).then_some(*r))
            .unwrap();
        a.click(button.x + 1, button.y);
        assert_eq!(a.view, View::Runs);
        assert_eq!(a.runs.len(), a.all_runs.len());
        assert!(a.run_filter.is_none());
        let lines = screen(&mut a, 140, 30);
        assert!(!lines.iter().any(|l| l.contains("runs of")));
        // Esc goes back to the routine the filter came from.
        a.view = View::Routines;
        a.select_id(&ids[1].0);
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.view, View::Runs);
        assert_eq!(a.run_filter.as_deref(), Some(ids[1].0.as_str()));
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.view, View::Routines);
        assert!(a.run_filter.is_none());
        assert_eq!(a.selected_routine().unwrap().id, ids[1].0);
        // A filter set in Runs itself: Esc shows all runs again.
        press(&mut a, KeyCode::Char('3'));
        a.sel_run = a
            .runs
            .iter()
            .position(|r| r.routine_id == ids[1].0)
            .unwrap();
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.runs.len(), count);
        let lines = screen(&mut a, 140, 30);
        assert!(lines.iter().any(|l| l.contains("Esc all runs")));
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.view, View::Runs);
        assert!(a.run_filter.is_none());
    }

    #[test]
    fn the_mouse_marks_the_menu_entry_under_it() {
        let mut a = app("hover");
        screen(&mut a, 120, 30);
        let row = a
            .hits
            .iter()
            .find_map(|(r, h)| matches!(h, Hit::Row(0)).then_some(*r))
            .unwrap();
        a.right_click(row.x + 2, row.y);
        screen(&mut a, 120, 30);
        let third = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::MenuRow(2)).then_some(*r))
            .unwrap();
        a.event(Event::Mouse(ratatui::crossterm::event::MouseEvent {
            kind: MouseEventKind::Moved,
            column: third.x + 3,
            row: third.y,
            modifiers: KeyModifiers::NONE,
        }));
        let Mode::Menu(m) = a.mode.clone() else {
            panic!("no menu");
        };
        assert_eq!(m.sel, 2);
    }

    #[test]
    fn a_routine_written_outside_the_list_waits_for_approval() {
        let mut a = app("approve");
        let all = screen(&mut a, 120, 30).join("\n");
        assert!(all.contains("a shows it for approval"), "{all}");
        // Select "Weekly cleanup" (written outside the list).
        a.sel_routine = a.routines.iter().position(|r| !r.approved).unwrap();
        press(&mut a, KeyCode::Char('r'));
        assert_eq!(a.mode, Mode::Normal, "no run before approval");
        press(&mut a, KeyCode::Char('a'));
        let Mode::Log(v) = &a.mode else {
            panic!("shows it first: {:?}", a.mode)
        };
        assert!(v.lines.iter().any(|l| l.starts_with("Options:")), "{v:?}");
        assert!(v.lines.iter().any(|l| l == "  Then stop."), "whole prompt");
        // `y` before it was shown, or before the end was read, does nothing.
        press(&mut a, KeyCode::Char('y'));
        assert!(matches!(a.mode, Mode::Log(_)));
        let all = screen(&mut a, 60, 12).join("\n");
        assert!(all.contains("Approve Weekly cleanup?"), "{all}");
        assert!(all.contains("read to the end"), "{all}");
        press(&mut a, KeyCode::Char('y'));
        assert!(!Routine::load(&a.paths, "weekly-cleanup").unwrap().approved);
        press(&mut a, KeyCode::End);
        let all = screen(&mut a, 60, 12).join("\n");
        assert!(all.contains("y approve"), "{all}");
        press(&mut a, KeyCode::Char('y'));
        assert!(a.routines.iter().all(|r| r.approved));
        // Switching on and off keeps it approved.
        press(&mut a, KeyCode::Char(' '));
        assert!(Routine::load(&a.paths, "weekly-cleanup").unwrap().approved);
    }

    #[test]
    fn switching_does_not_approve_a_change() {
        let mut a = app("toggle");
        a.sel_routine = a.routines.iter().position(|r| !r.approved).unwrap();
        press(&mut a, KeyCode::Char(' '));
        assert!(!Routine::load(&a.paths, "weekly-cleanup").unwrap().approved);
    }

    #[test]
    fn the_approval_view_shows_what_it_approves() {
        let mut a = app("approve-changed");
        a.sel_routine = a.routines.iter().position(|r| !r.approved).unwrap();
        press(&mut a, KeyCode::Char('a'));
        screen(&mut a, 120, 40);
        // Something writes again while the view is open.
        let dir = a.paths.routine_dir("weekly-cleanup");
        paths::write_atomic(&dir.join("prompt.md"), b"Something else.").unwrap();
        press(&mut a, KeyCode::Char('y'));
        assert_eq!(a.mode, Mode::Normal);
        assert!(
            !Routine::load(&a.paths, "weekly-cleanup").unwrap().approved,
            "only what was shown is approved"
        );
    }

    #[test]
    fn saving_the_form_keeps_a_waiting_routine_waiting() {
        let mut a = app("form-waiting");
        a.sel_routine = a.routines.iter().position(|r| !r.approved).unwrap();
        press(&mut a, KeyCode::Enter);
        assert!(a.form.is_some());
        a.form.as_mut().unwrap().name = "Weekly cleanup 2".into();
        a.form.as_mut().unwrap().dirty = true;
        a.try_save();
        assert!(a.form.is_none(), "saved");
        let r = Routine::load(&a.paths, "weekly-cleanup").unwrap();
        assert_eq!(r.file.name, "Weekly cleanup 2");
        assert!(!r.approved, "a approves it, not the form");
        // A routine that was approved stays approved when saved.
        a.sel_routine = a.routines.iter().position(|r| r.approved).unwrap();
        press(&mut a, KeyCode::Enter);
        a.form.as_mut().unwrap().dirty = true;
        a.try_save();
        assert!(Routine::load(&a.paths, "morning-report").unwrap().approved);
    }

    #[test]
    fn hidden_characters_are_dropped_from_shown_text() {
        assert_eq!(
            visible_text("ok\u{202E}txt.exe\u{200B}\u{1b}[2J\tend"),
            "oktxt.exe[2J\tend"
        );
    }

    #[test]
    fn mouse_selects_rows_and_switches_views() {
        let mut a = app("mouse");
        screen(&mut a, 120, 30);
        let row1 = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::Row(1)).then_some(*r))
            .unwrap();
        a.click(row1.x + 2, row1.y);
        assert_eq!(a.sel_routine, 1);
        a.click(row1.x + 2, row1.y);
        assert!(a.form.is_some(), "a double click opens the routine");
        a.form = None;
        screen(&mut a, 120, 30);
        let runs = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::View(View::Runs)).then_some(*r))
            .unwrap();
        a.click(runs.x, runs.y);
        assert_eq!(a.view, View::Runs);
        let help = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::Key('?')).then_some(*r))
            .unwrap();
        a.click(help.x, help.y);
        assert_eq!(a.mode, Mode::Help);
    }

    #[test]
    fn the_form_starts_with_what_when_and_where_and_keeps_save_in_sight() {
        let mut a = app("layout");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        let lines = screen(&mut a, 90, 40);
        let at = |s: &str| {
            lines
                .iter()
                .position(|l| l.contains(s))
                .unwrap_or_else(|| panic!("no {s:?} in\n{}", lines.join("\n")))
        };
        // What to do, when and where come first; the rest is one row.
        let order = [
            at(" Name "),
            at(" Runs with "),
            at(" Prompt "),
            at(" Repeat "),
            at(" Folder "),
            at(" More settings "),
            at("⏎ save"),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(!lines.iter().any(|l| l.contains(" Permission mode ")));
        assert!(lines[at(" More settings ")].contains("▸ Auto"));
        // Each part ends with an empty row when the window is tall enough,
        // and one empty row sits before the button.
        assert!(lines[at(" When ") - 1].trim().is_empty());
        assert!(lines[at("⏎ save") - 1].trim().is_empty());
        // ⏎ on "More settings" shows the rest, with today's defaults.
        a.form.as_mut().unwrap().select(Field::More);
        press(&mut a, KeyCode::Enter);
        let lines = screen(&mut a, 90, 50);
        for label in [" Permission mode ", " If missed ", " Run history "] {
            assert!(lines.iter().any(|l| l.contains(label)), "{label}");
        }
        assert!(lines.iter().any(|l| l.contains("‹ Auto ›")));
        // In a small window the fields scroll and say how many are out of
        // sight; the button stays, with an empty row above the footer.
        for (w, h) in [(80, 24), (60, 20)] {
            let lines = screen(&mut a, w, h);
            let all = lines.join("\n");
            assert!(all.contains("⏎ save"), "{all}");
            assert!(
                all.contains("more below") || all.contains(" below"),
                "{all}"
            );
        }
        a.form.as_mut().unwrap().select(Field::Keep);
        let all = screen(&mut a, 80, 24).join("\n");
        assert!(
            all.contains("⏎ save") && all.contains("more above"),
            "{all}"
        );
        assert!(all.contains(" Run history "), "{all}");
        // Hidden again, the cursor is back on a row that shows.
        a.form.as_mut().unwrap().select(Field::Keep);
        a.form.as_mut().unwrap().toggle_more();
        let form = a.form.as_ref().unwrap();
        assert!(form.fields().contains(&form.current()));
    }

    #[test]
    fn the_form_shows_more_of_the_prompt_and_says_when_it_goes_on() {
        let mut a = app("preview");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        let text: Vec<String> = (1..=12).map(|i| format!("Step {i} of the task.")).collect();
        a.form.as_mut().unwrap().prompt = text.join("\n");
        let lines = screen(&mut a, 90, 50);
        assert!(lines.iter().any(|l| l.contains("Step 1 of")));
        assert!(lines.iter().any(|l| l.contains("Step 8 of the task. …")));
        assert!(!lines.iter().any(|l| l.contains("Step 9 of")));
        // A short window still shows the first line and the "…".
        let lines = screen(&mut a, 90, 18);
        assert!(
            lines.iter().any(|l| l.contains("Step 1 of the task. …")),
            "{}",
            lines.join("\n")
        );
    }

    #[test]
    fn the_selected_field_explains_itself() {
        let mut a = app("explain");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        a.form.as_mut().unwrap().select(Field::Mode);
        for (w, h) in [(90, 34), (80, 24)] {
            let lines = screen(&mut a, w, h);
            assert!(lines.iter().any(|l| l.contains("▎ Permission mode")));
            assert!(lines.iter().any(|l| l.contains("‹ Auto ›")));
            assert!(lines.iter().any(|l| l.contains("▎ What the agent may do")));
        }
    }

    #[test]
    fn the_form_saves_mode_and_model() {
        let mut a = app("form");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        for c in "Check links".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        a.form.as_mut().unwrap().select(Field::Prompt);
        press(&mut a, KeyCode::Enter);
        for c in "Find broken links.".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(a.editor.is_none());
        let form = a.form.as_mut().unwrap();
        form.select(Field::Model);
        press(&mut a, KeyCode::Enter);
        let Mode::Pick(p) = a.mode.clone() else {
            panic!("no picker");
        };
        let opus = p.items.iter().position(|i| i.value == "opus").unwrap();
        press(&mut a, KeyCode::Char(char::from(b'1' + opus as u8)));
        a.form.as_mut().unwrap().select(Field::Folder);
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "/tmp".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(a.form.is_none(), "{:?}", a.flash);
        let r = Routine::load(&a.paths, "check-links").unwrap();
        assert_eq!(r.file.permission_mode, "auto");
        assert_eq!(r.file.model, "opus");
        assert_eq!(r.prompt, "Find broken links.");
    }

    #[test]
    fn models_listed_by_an_agent_can_be_narrowed() {
        let mut a = app("models");
        let dir = a.paths.state.join("models");
        std::fs::create_dir_all(&dir).unwrap();
        let names: Vec<String> = (0..20)
            .map(|i| format!("vendor{}/model-{i}", i % 3))
            .collect();
        std::fs::write(dir.join("opencode.txt"), names.join("\n")).unwrap();
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        let form = a.form.as_mut().unwrap();
        form.choose(Field::Agent, "opencode");
        form.select(Field::Model);
        press(&mut a, KeyCode::Enter);
        let Mode::Pick(p) = a.mode.clone() else {
            panic!("no picker");
        };
        assert!(p.filters());
        assert!(p.items.iter().any(|i| i.value == "vendor1/model-4"));
        let s = screen(&mut a, 90, 30);
        assert!(s.iter().any(|l| l.contains("Filter")));
        for c in "vendor2/model-1".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        let Mode::Pick(p) = a.mode.clone() else {
            panic!("no picker");
        };
        // "Other…" always stays.
        let shown: Vec<&str> = p.items.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(
            shown,
            vec![
                "vendor2/model-11",
                "vendor2/model-14",
                "vendor2/model-17",
                OTHER_MODEL
            ]
        );
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.form.as_ref().unwrap().model, "vendor2/model-11");
    }

    #[test]
    fn esc_keeps_what_was_written_and_ctrl_z_undoes() {
        let mut a = app("editor");
        press(&mut a, KeyCode::Char('p'));
        assert!(a.editor.is_some());
        press(&mut a, KeyCode::Char('!'));
        press(&mut a, KeyCode::Char('?'));
        a.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        press(&mut a, KeyCode::Char('.'));
        press(&mut a, KeyCode::Esc);
        assert!(a.editor.is_none());
        assert!(matches!(a.mode, Mode::Normal));
        let r = Routine::load(&a.paths, "morning-report").unwrap();
        assert_eq!(r.prompt, "Do task 0.\nThen stop..");
    }

    // ---- the list's own rules: who owns the mouse, what scrolls ----------

    /// The screen's cells, for what text alone does not show.
    fn cells(a: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| ui::draw(f, a)).unwrap();
        t.backend().buffer().clone()
    }

    /// Ended runs of the first routine, oldest first by `n`.
    fn add_runs(a: &mut App, n: usize) {
        let (id, name) = (a.routines[0].id.clone(), a.routines[0].file.name.clone());
        for i in 0..n {
            let mut run = Run::new(&id, &name, "claude", store::Trigger::Scheduled, None);
            run.id = format!("20261101-08{i:02}00-000");
            run.finish(store::Status::Finished, "");
            run.result = format!("Result {i}");
            run.save(&a.paths).unwrap();
        }
        a.reload();
    }

    fn hit_rect(a: &App, hit: Hit) -> Rect {
        a.hits
            .iter()
            .rev()
            .find_map(|(r, h)| (*h == hit).then_some(*r))
            .unwrap_or_else(|| panic!("no {hit:?} in {:?}", a.hits))
    }

    fn mouse(a: &mut App, kind: MouseEventKind, x: u16, y: u16) {
        a.event(Event::Mouse(ratatui::crossterm::event::MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }

    #[test]
    fn a_dialog_owns_the_footer_and_the_mouse() {
        let mut a = app("owner");
        a.select_id("morning-report");
        let id = a.selected_routine().unwrap().id.clone();
        for mut run in store::runs_of(&a.paths, &id) {
            run.finish(store::Status::Finished, "");
            run.save(&a.paths).unwrap();
        }
        a.reload();
        // In the list, the footer offers the list's keys.
        let lines = screen(&mut a, 100, 30);
        assert!(lines[29].contains("r run now"), "{}", lines[29]);
        let run_now = hit_rect(&a, Hit::Key('r'));
        let row = hit_rect(&a, Hit::Row(1));
        // Under the delete question it offers only the question's keys, so
        // a click where "r run now" was cannot delete the runs.
        press(&mut a, KeyCode::Char('d'));
        let lines = screen(&mut a, 100, 30);
        assert!(!lines[29].contains("run now"), "{}", lines[29]);
        assert!(
            lines[29].contains("y delete, keep runs") && lines[29].contains("n no"),
            "{}",
            lines[29]
        );
        for (r, h) in &a.hits {
            assert!(
                matches!(h, Hit::Key('y' | 'r' | 'n')),
                "{h:?} at {r:?} is live under the dialog"
            );
        }
        let foot_r = a
            .hits
            .iter()
            .find_map(|(r, h)| (*h == Hit::Key('r') && r.y == 29).then_some(*r))
            .expect("the footer names r");
        let label: String = lines[29]
            .chars()
            .skip(foot_r.x as usize)
            .take(foot_r.width as usize)
            .collect();
        assert_eq!(label, "r delete with its runs");
        // A click on a row or a tab under the dialog only closes it.
        if a.hit_at(run_now.x, run_now.y).is_none() {
            a.click(run_now.x, run_now.y);
        } else {
            a.click(row.x + 2, row.y);
        }
        assert!(matches!(a.mode, Mode::Normal));
        assert!(a.routines.iter().any(|r| r.id == id), "nothing deleted");
        assert!(a.run_count(&id) > 0, "its runs are still there");
        assert_eq!(a.selected_routine().unwrap().id, id, "no row chosen");

        // The same holds for every dialog: no row, tab or link of the
        // list is live under it.
        let openers: [&dyn Fn(&mut App); 5] = [
            &|a| press(a, KeyCode::Char('?')),
            &|a| press(a, KeyCode::Char('r')),
            &|a| press(a, KeyCode::Char('q')),
            &|a| press(a, KeyCode::Char('l')),
            &|a| {
                screen(a, 100, 30);
                let r = hit_rect(a, Hit::Row(0));
                a.right_click(r.x + 3, r.y);
            },
        ];
        for (n, open) in openers.iter().enumerate() {
            a.mode = Mode::Normal;
            open(&mut a);
            assert!(!matches!(a.mode, Mode::Normal), "dialog {n} opened");
            for (w, h) in [(100, 30), (60, 20)] {
                screen(&mut a, w, h);
                for (r, hit) in &a.hits {
                    assert!(
                        !matches!(hit, Hit::Row(_) | Hit::View(_) | Hit::RunsOf(_)),
                        "{hit:?} at {r:?} is live under dialog {n}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_cursor_stays_on_its_item_when_the_list_changes() {
        let mut a = app("identity");
        add_runs(&mut a, 8);
        a.view = View::Runs;
        a.sel_run = 5;
        let on = a.runs[5].id.clone();
        // A new run comes in on top: the cursor follows its run.
        let mut run = Run::new(
            "morning-report",
            "Morning report",
            "claude",
            store::Trigger::Scheduled,
            None,
        );
        run.id = "20261231-090000-000".into();
        run.finish(store::Status::Finished, "");
        run.save(&a.paths).unwrap();
        a.reload();
        assert_eq!(a.runs[0].id, "20261231-090000-000");
        assert_eq!(a.runs[a.sel_run].id, on);
        // Only one routine's runs, and back to all: still the same run.
        press(&mut a, KeyCode::Char('h'));
        assert!(a.run_filter.is_some());
        assert_eq!(a.runs[a.sel_run].id, on);
        press(&mut a, KeyCode::Char('h'));
        assert!(a.run_filter.is_none());
        assert_eq!(a.runs[a.sel_run].id, on);
        // A deleted run leaves the cursor in the list.
        let last = a.runs.len() - 1;
        a.sel_run = last;
        store::remove(&a.paths, &a.runs[last]);
        a.reload();
        assert_eq!(a.sel_run, a.runs.len() - 1);

        // Routines: one that sorts first appears; the cursor stays on its
        // routine.
        a.view = View::Routines;
        a.select_id("weekly-cleanup");
        let mut f = Form::new("/tmp");
        f.name = "A new one".into();
        f.prompt = "Say hello.".into();
        let (file, prompt) = f.build().unwrap();
        Routine {
            id: routine::new_id(&a.paths, "A new one"),
            file,
            prompt,
            problem: None,
            unreadable: false,
            approved: true,
        }
        .save_approved(&a.paths)
        .unwrap();
        a.reload();
        assert_eq!(a.routines[0].file.name, "A new one");
        assert_eq!(a.selected_routine().unwrap().id, "weekly-cleanup");
    }

    #[test]
    fn a_click_never_scrolls_and_the_wheel_moves_the_view() {
        let mut a = app("scroll");
        add_runs(&mut a, 30);
        a.view = View::Runs;
        a.sel_run = 0;
        let all = a.runs.len();
        let lines = screen(&mut a, 100, 24);
        // The list says which rows show.
        assert!(lines[2].contains(&format!("1–10 of {all}")), "{}", lines[2]);
        // The cursor at the end scrolls the list there.
        press(&mut a, KeyCode::End);
        let lines = screen(&mut a, 100, 24);
        let top = a.tops[2];
        assert_eq!(top, all - 10);
        assert!(lines[2].contains(&format!("{}–{all} of {all}", top + 1)));
        // A click on a row in the middle marks it; no row moves.
        let target = top + 4;
        let before = hit_rect(&a, Hit::Row(target));
        a.click(before.x + 5, before.y);
        assert_eq!(a.sel_run, target);
        screen(&mut a, 100, 24);
        assert_eq!(a.tops[2], top);
        assert_eq!(hit_rect(&a, Hit::Row(target)), before);
        // So a second click lands on the same run and opens it.
        a.click(before.x + 5, before.y);
        assert!(matches!(a.mode, Mode::RunPage(_)), "{:?}", a.mode);
        press(&mut a, KeyCode::Esc);
        // The wheel moves the view, not the cursor.
        for _ in 0..3 {
            mouse(&mut a, MouseEventKind::ScrollUp, 10, 10);
        }
        screen(&mut a, 100, 24);
        assert_eq!((a.tops[2], a.sel_run), (top - 3, target));
        for _ in 0..40 {
            mouse(&mut a, MouseEventKind::ScrollUp, 10, 10);
        }
        let lines = screen(&mut a, 100, 24);
        assert_eq!((a.tops[2], a.sel_run), (0, target));
        assert!(lines[2].contains(&format!("1–10 of {all}")));
        for _ in 0..80 {
            mouse(&mut a, MouseEventKind::ScrollDown, 10, 10);
        }
        screen(&mut a, 100, 24);
        assert_eq!(a.tops[2], all - 10, "the view stops at the end");
        // A key brings the cursor's row back into view.
        for _ in 0..40 {
            mouse(&mut a, MouseEventKind::ScrollUp, 10, 10);
        }
        press(&mut a, KeyCode::Up);
        screen(&mut a, 100, 24);
        let shown = a.tops[2]..a.tops[2] + 10;
        assert!(shown.contains(&a.sel_run), "{shown:?} {}", a.sel_run);
        // A reload while scrolled keeps the picture: a new run on top
        // moves the offset along with the rows.
        let (top, on) = (a.tops[2], a.runs[a.sel_run].id.clone());
        let mut run = Run::new(
            "morning-report",
            "Morning report",
            "claude",
            store::Trigger::Scheduled,
            None,
        );
        run.id = "20261231-090000-000".into();
        run.finish(store::Status::Finished, "");
        run.save(&a.paths).unwrap();
        a.reload();
        assert_eq!(a.runs[a.sel_run].id, on);
        assert_eq!(a.tops[2], top + 1);
    }

    #[test]
    fn a_narrow_window_keeps_the_status_the_off_marker_and_whole_tabs() {
        let mut a = app("narrow");
        a.icons.nerd = false;
        // A name and a schedule that are longer than their columns.
        let mut r = Routine::load(&a.paths, "weekly-cleanup").unwrap();
        r.file.name = "Weekly cleanup of every old branch and stale build folder".into();
        r.file.schedule = "15 8-18 * * 1-5".into();
        r.save(&a.paths).unwrap();
        a.reload();
        for (w, h) in [
            (160, 50),
            (120, 40),
            (100, 30),
            (94, 30),
            (80, 24),
            (60, 20),
        ] {
            a.scheduler = None;
            let lines = screen(&mut a, w, h);
            let all = lines.join("\n");
            // The warning that nothing runs stays, with a symbol of its own.
            assert!(lines[0].contains("○ routines off"), "{w}: {}", lines[0]);
            // The tabs are whole; with too little room for the warning
            // beside them they lose their counts, never their names.
            let tabs = if w >= 80 {
                ["Routines (2)", "Cron jobs (0)", "Runs (1)"]
            } else {
                ["Routines", "Cron jobs", "Runs"]
            };
            for tab in tabs {
                assert!(lines[0].contains(tab), "{w}: {}", lines[0]);
            }
            // How the last run went shows at every width.
            assert!(all.contains("… Starting"), "{w}x{h}:\n{all}");
            // Nothing runs off the edge: what is cut ends in "…".
            for l in &lines[1..] {
                assert!(l.ends_with(' ') || l.ends_with('…'), "{w}: {l:?}");
            }
            if w <= 100 {
                assert!(
                    lines
                        .iter()
                        .any(|l| l.contains("Weekly cleanup of") && l.contains('…')),
                    "{w}x{h}:\n{all}"
                );
            }
            a.scheduler = Some((1, String::new()));
            let lines = screen(&mut a, w, h);
            assert!(!lines[0].contains("routines off"), "{w}: {}", lines[0]);
        }
        // From 80 columns on the last run has its own column, with the
        // link to the runs; the schedule column holds what it must.
        let lines = screen(&mut a, 80, 24);
        assert!(lines.iter().any(|l| l.contains("Last run")));
        assert!(lines.iter().any(|l| l.contains("1 run ›")));
        let lines = screen(&mut a, 160, 50);
        assert!(lines
            .iter()
            .any(|l| l.contains("Weekdays, hourly at :15 from 08:15 to 18:15")));
        // Very narrow: the tabs get shorter names before anything is cut,
        // and "off" still shows.
        a.scheduler = None;
        for w in [50, 44, 40] {
            let lines = screen(&mut a, w, 20);
            for tab in ["Routines", "Cron", "Runs"] {
                assert!(lines[0].contains(tab), "{w}: {}", lines[0]);
            }
            assert!(lines[0].contains("off"), "{w}: {}", lines[0]);
            assert!(lines.join("\n").contains("… Starting"), "{w}");
        }
    }

    #[test]
    fn messages_are_whole_and_problems_stay_until_a_key() {
        let mut a = app("messages");
        let text = "Starting; routines are off, so the run is not followed (S switches them on)";
        a.flash(text, YELLOW);
        for (w, h) in [(120, 30), (80, 24), (60, 20), (40, 20)] {
            let lines = screen(&mut a, w, h);
            // The footer grows; the message reads whole across its rows.
            let foot: Vec<&str> = lines.iter().rev().take(3).rev().map(|l| l.trim()).collect();
            let joined = foot.join(" ");
            assert!(joined.contains(text), "{w}: {joined}");
        }
        // It does not go by itself, only with the next key or click.
        assert!(a.flash.as_ref().unwrap().sticky);
        press(&mut a, KeyCode::Down);
        assert!(a.flash.is_none());
        a.flash("Could not read the crontab", RED);
        screen(&mut a, 80, 24);
        a.click(40, 10);
        assert!(a.flash.is_none());
        // Good news goes after a moment, as before.
        a.flash("Saved", GREEN);
        assert!(!a.flash.as_ref().unwrap().sticky);
        // One too long for three rows ends in "…".
        a.flash("word ".repeat(80), RED);
        let lines = screen(&mut a, 40, 20);
        assert!(lines[19].trim_end().ends_with('…'), "{}", lines[19]);
        assert!(lines[17].contains("word"));
    }

    #[test]
    fn form_text_fields_have_a_cursor_and_scroll_sideways() {
        let mut a = app("cursor");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        for c in "Chck".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        press(&mut a, KeyCode::Left);
        press(&mut a, KeyCode::Left);
        press(&mut a, KeyCode::Char('e'));
        assert_eq!(a.form.as_ref().unwrap().name, "Check");
        let lines = screen(&mut a, 80, 24);
        assert!(lines.iter().any(|l| l.contains("Che▏ck")), "{lines:#?}");
        press(&mut a, KeyCode::Home);
        press(&mut a, KeyCode::Delete);
        assert_eq!(a.form.as_ref().unwrap().name, "heck");
        press(&mut a, KeyCode::End);
        press(&mut a, KeyCode::Backspace);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Char('!'));
        assert_eq!(a.form.as_ref().unwrap().name, "hec!");
        // q, ? and the digits are text here, not commands.
        press(&mut a, KeyCode::Char('q'));
        press(&mut a, KeyCode::Char('?'));
        assert_eq!(a.form.as_ref().unwrap().name, "hec!q?");
        assert!(matches!(a.mode, Mode::Normal));
        // A value longer than its row shows the part around the cursor.
        let form = a.form.as_mut().unwrap();
        form.folder = "~/projects/a-very-long-folder-name/with/many/parts/inside/it".into();
        form.select(Field::Folder);
        form.cursor_end();
        let lines = screen(&mut a, 60, 20);
        let row = lines.iter().find(|l| l.contains(" Folder ")).unwrap();
        assert!(row.contains('…') && row.contains("inside/it▏"), "{row}");
        press(&mut a, KeyCode::Home);
        let lines = screen(&mut a, 60, 20);
        let row = lines.iter().find(|l| l.contains(" Folder ")).unwrap();
        assert!(
            row.contains("▏~/projects") && row.trim_end().ends_with('…'),
            "{row}"
        );
        // The footer names the keys of the field the cursor is in.
        assert!(lines[19].contains("← → cursor"), "{}", lines[19]);
        a.form.as_mut().unwrap().select(Field::Repeat);
        let lines = screen(&mut a, 80, 24);
        assert!(lines[23].contains("← → change"), "{}", lines[23]);
    }

    #[test]
    fn one_mouse_model_everywhere() {
        let mut a = app("mouse-model");
        add_runs(&mut a, 2);
        a.icons.nerd = false;
        a.sel_routine = 0;
        screen(&mut a, 120, 30);
        // One click marks a row; a click on the marked row opens it.
        let second = hit_rect(&a, Hit::Row(1));
        a.click(second.x + 4, second.y);
        assert_eq!(a.sel_routine, 1);
        assert!(a.form.is_none());
        a.click(second.x + 4, second.y);
        assert!(a.form.is_some(), "the marked row opens");
        press(&mut a, KeyCode::Esc);
        // Only the "n runs ›" text is a link; the cell beside it marks.
        a.sel_routine = 1;
        let lines = screen(&mut a, 120, 30);
        let link = hit_rect(&a, Hit::RunsOf(0));
        let label: String = lines[link.y as usize]
            .chars()
            .skip(link.x as usize)
            .take(link.width as usize)
            .collect();
        assert_eq!(label, "3 runs ›");
        a.click(link.x - 4, link.y);
        assert_eq!((a.view, a.sel_routine), (View::Routines, 0));
        a.click(link.x + 1, link.y);
        assert_eq!(a.view, View::Runs);
        assert_eq!(a.runs.len(), 3);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.view, View::Routines);

        // The row under the mouse is lit.
        mouse(&mut a, MouseEventKind::Moved, second.x + 4, second.y);
        let buf = cells(&mut a, 120, 30);
        assert_eq!(buf[(second.x + 4, second.y)].bg, crate::theme::HOVER_BG);
        assert_eq!(
            buf[(second.x + 4, second.y - 2)].bg,
            crate::theme::SELECTED_BG
        );

        // In the form: a click on ‹ or › steps the choice at once.
        press(&mut a, KeyCode::Enter);
        screen(&mut a, 100, 30);
        let repeat = a
            .form
            .as_ref()
            .unwrap()
            .fields()
            .iter()
            .position(|f| *f == Field::Repeat)
            .unwrap();
        let before = a.form.as_ref().unwrap().repeat;
        let next = hit_rect(&a, Hit::Cycle(repeat, 1));
        a.click(next.x, next.y);
        assert_ne!(a.form.as_ref().unwrap().repeat, before);
        assert_eq!(a.form.as_ref().unwrap().current(), Field::Repeat);
        screen(&mut a, 100, 30);
        let back = hit_rect(&a, Hit::Cycle(repeat, -1));
        a.click(back.x, back.y);
        assert_eq!(a.form.as_ref().unwrap().repeat, before);
        // A click on a choice marks it, the next one opens its list, and a
        // click on an entry chooses it.
        a.form.as_mut().unwrap().select(Field::Name);
        screen(&mut a, 100, 30);
        let row = hit_rect(&a, Hit::Field(repeat));
        a.click(row.x + 2, row.y);
        assert!(matches!(a.mode, Mode::Normal));
        a.click(row.x + 2, row.y);
        let Mode::Pick(p) = a.mode.clone() else {
            panic!("no list: {:?}", a.mode);
        };
        screen(&mut a, 100, 30);
        let entry = hit_rect(&a, Hit::PickRow(1));
        a.click(entry.x + 3, entry.y);
        assert!(matches!(a.mode, Mode::Normal));
        assert_eq!(a.form.as_ref().unwrap().repeat.label(), p.items[1].label);
        // A click beside a dialog closes it and changes nothing.
        let chosen = a.form.as_ref().unwrap().repeat;
        screen(&mut a, 100, 30);
        a.click(row.x + 2, row.y);
        assert!(matches!(a.mode, Mode::Pick(_)));
        screen(&mut a, 100, 30);
        let d = a.dialog.expect("the list is a dialog");
        a.click(d.x.saturating_sub(1), d.y + 1);
        assert!(matches!(a.mode, Mode::Normal));
        assert_eq!(a.form.as_ref().unwrap().repeat, chosen);
        // "More settings" and the buttons act on the first click.
        screen(&mut a, 100, 30);
        let more = a
            .form
            .as_ref()
            .unwrap()
            .fields()
            .iter()
            .position(|f| *f == Field::More)
            .unwrap();
        let row = hit_rect(&a, Hit::Field(more));
        a.click(row.x + 2, row.y);
        assert!(a.form.as_ref().unwrap().more);
    }

    #[test]
    fn questions_are_even_and_enter_confirms_only_what_loses_nothing() {
        let mut a = app("confirm");
        a.select_id("morning-report");
        for mut run in store::runs_of(&a.paths, "morning-report") {
            run.finish(store::Status::Finished, "");
            run.save(&a.paths).unwrap();
        }
        a.reload();
        // Deleting and running by hand need y: ⏎ is "no".
        press(&mut a, KeyCode::Char('d'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert!(c.danger() && !c.enter_confirms());
        let lines = screen(&mut a, 100, 30);
        assert!(!lines.join("\n").contains("⏎"), "no ⏎ on a delete");
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::Normal));
        assert!(a.routines.iter().any(|r| r.id == "morning-report"));
        press(&mut a, KeyCode::Char('r'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert!(!c.enter_confirms(), "running by hand asks for y");
        assert_eq!(c.yes(), "run");
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::Normal));
        assert_eq!(a.run_count("morning-report"), 1, "nothing started");
        // What the same key undoes is confirmed by ⏎ too, and says so.
        a.scheduler = Some((1, String::new()));
        press(&mut a, KeyCode::Char('S'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question");
        };
        assert!(c.enter_confirms() && !c.danger());
        let lines = screen(&mut a, 100, 30);
        assert!(lines.iter().any(|l| l.contains("y or ⏎ turn off")));
        press(&mut a, KeyCode::Esc);
        // A dialog that is taller than the window keeps its buttons.
        press(&mut a, KeyCode::Char('d'));
        for (w, h) in [(60, 9), (40, 8), (30, 6)] {
            let all = screen(&mut a, w, h).join("\n");
            assert!(all.contains("y delete"), "{w}x{h}:\n{all}");
        }
        press(&mut a, KeyCode::Esc);
        // A note that does not fit scrolls, and says so.
        a.mode = Mode::Message {
            title: "How routines work".into(),
            lines: INTRO.iter().map(|l| l.to_string()).collect(),
        };
        let all = screen(&mut a, 60, 12).join("\n");
        assert!(all.contains("more lines") && all.contains("⏎ ok"), "{all}");
        let before = a.help_scroll;
        press(&mut a, KeyCode::Down);
        screen(&mut a, 60, 12);
        assert_eq!(a.help_scroll, before + 1);
    }

    #[test]
    fn the_editor_can_discard_and_says_what_esc_does() {
        let mut a = app("discard");
        a.select_id("morning-report");
        press(&mut a, KeyCode::Char('p'));
        assert!(a.editor.is_some());
        let lines = screen(&mut a, 100, 30);
        // Esc saves a routine's prompt at once, and the footer says so.
        assert!(lines[29].contains("Esc save and close"), "{}", lines[29]);
        assert!(lines[29].contains("Ctrl+D discard"), "{}", lines[29]);
        // Nothing written: Ctrl+D just leaves.
        a.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(a.editor.is_none() && matches!(a.mode, Mode::Normal));
        // Something written: it asks, and y throws it away.
        press(&mut a, KeyCode::Char('p'));
        for c in "Extra".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        a.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("no question: {:?}", a.mode);
        };
        assert_eq!(c.purpose, ConfirmFor::DiscardEditor);
        assert!(!c.enter_confirms());
        // "Keep editing" goes back to the text.
        press(&mut a, KeyCode::Char('n'));
        assert!(a.editor.is_some());
        a.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        press(&mut a, KeyCode::Char('y'));
        assert!(a.editor.is_none());
        let r = Routine::load(&a.paths, "morning-report").unwrap();
        assert_eq!(r.prompt, "Do task 0.\nThen stop.");
        // In a new routine's form, Esc only goes back to the form.
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        a.form.as_mut().unwrap().select(Field::Prompt);
        press(&mut a, KeyCode::Enter);
        let lines = screen(&mut a, 100, 30);
        assert!(lines[29].contains("Esc back to the form"), "{}", lines[29]);
    }

    #[test]
    fn keys_that_do_not_apply_say_so() {
        let mut a = app("applies");
        a.icons.nerd = false;
        // `h`, `l` and `o` on a routine without runs.
        a.select_id("weekly-cleanup");
        let lines = screen(&mut a, 140, 30);
        assert!(!lines[29].contains("l output"), "{}", lines[29]);
        assert!(!lines[29].contains("its runs"), "{}", lines[29]);
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.view, View::Routines, "no empty list of runs");
        assert_eq!(a.flash.as_ref().unwrap().text, "No run yet");
        for key in ['l', 'o'] {
            press(&mut a, KeyCode::Char(key));
            assert!(a.flash.is_some() && matches!(a.mode, Mode::Normal));
        }
        // The footer offers them where there is a run.
        a.flash = None;
        a.select_id("morning-report");
        let lines = screen(&mut a, 140, 30);
        for key in ["l output", "h its runs"] {
            assert!(lines[29].contains(key), "{key}: {}", lines[29]);
        }
        // Going to its tab is offered while the run has one.
        assert!(!lines[29].contains("go to its tab"), "{}", lines[29]);
        // A job that is only in the crontab: the keys of a routine say
        // what to do first, and the footer offers only what works.
        fake_crontab(&a, "0 2 * * * /opt/jobs/backup.sh --all\n");
        a.crontab_read = None;
        a.reload();
        press(&mut a, KeyCode::Char('2'));
        let lines = screen(&mut a, 140, 30);
        assert!(lines[29].contains("⏎ take it over"), "{}", lines[29]);
        assert!(!lines[29].contains("run now"), "{}", lines[29]);
        for key in ['r', ' ', 'd', 'l', 'h', 'e', 'p', 'c', 'o', 'a'] {
            press(&mut a, KeyCode::Char(key));
            assert!(matches!(a.mode, Mode::Normal), "{key}: {:?}", a.mode);
            assert!(a.form.is_none() && a.editor.is_none(), "{key}");
            let said = a.flash.as_ref().map(|f| f.text.clone()).unwrap_or_default();
            assert!(said.contains("only in your crontab"), "{key}: {said:?}");
            a.flash = None;
        }
        assert_eq!(a.crontab_jobs.len(), 1);
        // The dialog is a list like any other: a click chooses.
        press(&mut a, KeyCode::Enter);
        screen(&mut a, 100, 30);
        let copy = hit_rect(&a, Hit::Take(2));
        mouse(&mut a, MouseEventKind::Moved, copy.x + 2, copy.y);
        assert_eq!(a.take_sel, 2);
        let keep = hit_rect(&a, Hit::Take(0));
        a.click(keep.x + 2, keep.y);
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("keeping does not ask: {:?}", a.mode);
        };
        assert_eq!(c.question, "Take over 1 cron job?");
        press(&mut a, KeyCode::Esc);
        crate::crontab::use_fake(None);
    }

    #[test]
    fn the_output_view_keeps_why_a_run_failed_above_its_output() {
        let mut a = app("why");
        let mut run = Run::new(
            "morning-report",
            "Morning report",
            "shell",
            store::Trigger::Scheduled,
            None,
        );
        run.id = "20261231-090000-000".into();
        run.finish(store::Status::Failed, "Ended with exit code 23");
        run.save(&a.paths).unwrap();
        let out: String = (1..=80).map(|i| format!("output line {i}\n")).collect();
        std::fs::write(run.file(&a.paths, "out"), out).unwrap();
        a.reload();
        a.view = View::Runs;
        a.sel_run = a.runs.iter().position(|r| r.id == run.id).unwrap();
        // The list shows the reason and how it was started, once.
        a.icons.nerd = false;
        let lines = screen(&mut a, 120, 30);
        let all = lines.join("\n");
        assert!(all.contains("Ended with exit code 23"), "{all}");
        assert_eq!(all.matches("By hand").count(), 1, "{all}");
        assert!(lines
            .iter()
            .any(|l| l.contains("✗") && l.contains("Failed")));
        // The output opens at its end, where the answer is; the reason
        // stays on top of it.
        press(&mut a, KeyCode::Char('l'));
        let lines = screen(&mut a, 100, 24);
        assert!(lines[2].contains("Ended with exit code 23"), "{}", lines[2]);
        assert!(lines.iter().any(|l| l.contains("output line 80")));
        assert!(lines.iter().any(|l| l.contains("–80 of 80")));
        assert!(!lines.iter().any(|l| l.contains("output line 1 ")));
        // Scrolled to the start, the reason is still there.
        press(&mut a, KeyCode::Home);
        let lines = screen(&mut a, 100, 24);
        assert!(lines[2].contains("Ended with exit code 23"), "{}", lines[2]);
        assert!(lines.iter().any(|l| l.contains("output line 1 ")));
        assert!(lines.iter().any(|l| l.contains("lines 1–")));
    }

    #[test]
    fn the_details_say_what_the_list_says() {
        let mut a = app("agree");
        a.icons.nerd = false;
        // A routine with a problem, one that waits, and one with a model
        // and a mode.
        let mut r = Routine::load(&a.paths, "morning-report").unwrap();
        r.file.model = "opus".into();
        r.file.permission_mode = "plan".into();
        r.save_approved(&a.paths).unwrap();
        a.reload();
        a.select_id("morning-report");
        let lines = screen(&mut a, 120, 40);
        let about = "claude · Opus · Plan only";
        assert_eq!(
            lines.iter().filter(|l| l.contains(about)).count(),
            2,
            "list and details name model and mode in one order:\n{}",
            lines.join("\n")
        );
        // Waiting for approval: no next time, in the list or below it.
        a.select_id("weekly-cleanup");
        let lines = screen(&mut a, 120, 40);
        let head = lines
            .iter()
            .position(|l| l.contains(" Weekly cleanup ─"))
            .unwrap();
        let details = lines[head..].join("\n");
        assert!(details.contains("waits for approval"), "{details}");
        assert!(details.contains("a shows it for approval"), "{details}");
        assert!(!details.contains("next "), "{details}");
        assert!(lines.iter().any(|l| l.starts_with(" ◐ ")), "its own symbol");
        // A problem: red, with its own symbol, and no next time either.
        let mut r = Routine::load(&a.paths, "weekly-cleanup").unwrap();
        r.file.folder = "/tmp/herdr-routines-no-such-folder".into();
        r.save_approved(&a.paths).unwrap();
        a.reload();
        a.select_id("weekly-cleanup");
        let lines = screen(&mut a, 120, 40);
        let head = lines
            .iter()
            .position(|l| l.contains(" Weekly cleanup ─"))
            .unwrap();
        let details = lines[head..].join("\n");
        assert!(details.contains("Problem"), "{details}");
        assert!(details.contains("does not exist"), "{details}");
        assert!(!details.contains("next "), "{details}");
        assert!(lines.iter().any(|l| l.starts_with(" ! ")), "its own symbol");
        // On and off differ by symbol, not only by colour.
        a.select_id("morning-report");
        press(&mut a, KeyCode::Char(' '));
        let lines = screen(&mut a, 120, 40);
        assert!(lines
            .iter()
            .any(|l| l.starts_with(" ○ ") && l.contains("Morning report")));
        press(&mut a, KeyCode::Char(' '));
        let lines = screen(&mut a, 120, 40);
        assert!(lines
            .iter()
            .any(|l| l.starts_with(" ● ") && l.contains("Morning report")));
    }

    #[test]
    fn the_help_names_every_key_and_wraps() {
        let mut a = app("help");
        press(&mut a, KeyCode::Char('?'));
        for (w, h) in [(60, 20), (80, 24), (130, 45)] {
            a.help_scroll = 0;
            let mut seen = String::new();
            loop {
                let lines = screen(&mut a, w, h);
                // Inside the frame, a line never reaches the border.
                for l in &lines[2..h as usize - 2] {
                    if let Some(body) = l.trim_end().strip_suffix('│') {
                        assert!(body.ends_with(' '), "{w}: cut at the edge: {l}");
                    }
                }
                seen.push_str(&lines.join("\n"));
                if a.help_scroll >= a.help_max {
                    break;
                }
                press(&mut a, KeyCode::PageDown);
            }
            // The list's basic keys come first.
            let first = screen_at_top(&mut a, w, h);
            assert!(
                first.contains("↑ ↓  j k") && first.contains("move"),
                "{first}"
            );
            for key in [
                "PgUp",
                "Home",
                "Tab",
                "Space",
                "Ctrl+D",
                "Ctrl+S",
                "Ctrl+U",
                "Ctrl+Z",
                "Ctrl+O",
                "Del",
                "duplicate",
                "Mouse",
                "How it works",
            ] {
                assert!(seen.contains(key), "{w}x{h}: no {key}");
            }
        }
    }

    fn screen_at_top(a: &mut App, w: u16, h: u16) -> String {
        a.help_scroll = 0;
        screen(a, w, h).join("\n")
    }

    /// A made-up routine, saved in the list (so it is approved).
    fn add_routine(a: &mut App, name: &str, enabled: bool) -> String {
        let mut f = Form::new("/tmp");
        f.name = name.to_string();
        f.prompt = "Do the task.".into();
        f.enabled = enabled;
        let (file, prompt) = f.build().unwrap();
        let r = Routine {
            id: routine::new_id(&a.paths, name),
            file,
            prompt,
            problem: None,
            unreadable: false,
            approved: false,
        };
        r.save_approved(&a.paths).unwrap();
        r.id
    }

    /// A made-up run of a routine that ended (or is going) in `status`.
    /// `n` orders the runs: a higher one is newer.
    fn add_run(a: &mut App, id: &str, n: u32, status: store::Status, message: &str) -> Run {
        let name = a
            .routines
            .iter()
            .find(|r| r.id == id)
            .map_or(id.to_string(), |r| r.file.name.clone());
        let mut run = Run::new(id, &name, "claude", store::Trigger::Scheduled, None);
        run.id = format!("20261101-08{n:02}00-000");
        run.started = Local::now() - chrono::Duration::minutes(i64::from(60 - n));
        if status.active() {
            run.status = status;
            run.message = message.into();
        } else {
            run.finish(status, message);
        }
        run.save(&a.paths).unwrap();
        run
    }

    fn footer(a: &mut App, w: u16, h: u16) -> String {
        screen(a, w, h)[h as usize - 1].clone()
    }

    #[test]
    fn the_awake_chip_opens_a_choice_of_three() {
        use crate::awake;
        let mut a = app("awake");
        if !awake::available() {
            // No helper on this system: no chip, and the key says why.
            assert!(!screen(&mut a, 160, 30)[0].contains("awake"));
            press(&mut a, KeyCode::Char('W'));
            assert!(a.flash.as_ref().unwrap().text.contains("not supported"));
            return;
        }
        let top = screen(&mut a, 160, 30)[0].clone();
        assert!(top.contains("awake during runs"), "{top}");
        // It stands left of autostart and routines.
        assert!(top.find("awake").unwrap() < top.find("routines o").unwrap());
        // A click on it (or W) offers the three ways, with a note about
        // the lid.
        let chip = hit_rect(&a, Hit::Key('W'));
        a.click(chip.x + 2, chip.y);
        let Mode::Pick(p) = a.mode.clone() else {
            panic!("no choice: {:?}", a.mode);
        };
        let names: Vec<&str> = p.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(names, ["During runs", "Always", "Off"]);
        assert_eq!(p.sel, 0, "the way that holds now is marked");
        for (w, h) in [(160, 50), (80, 24), (60, 20)] {
            let all = screen(&mut a, w, h).join("\n");
            assert!(all.contains("Keep the computer awake"), "{all}");
            assert!(all.contains("No idle sleep while a run is going"), "{all}");
            assert!(all.contains("lid closed still goes to sleep"), "{all}");
        }
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert_eq!(awake::mode(&a.paths), awake::Mode::Always);
        assert_eq!(
            a.flash.as_ref().unwrap().text,
            "The computer now stays awake while Herdr runs"
        );
        assert!(screen(&mut a, 160, 30)[0].contains("awake always"));
        // Off is a hollow sign and says what it means for a run.
        press(&mut a, KeyCode::Char('W'));
        press(&mut a, KeyCode::Char('3'));
        assert_eq!(awake::mode(&a.paths), awake::Mode::Off);
        assert!(a.flash.as_ref().unwrap().text.contains("middle of a run"));
        let top = screen(&mut a, 160, 30)[0].clone();
        assert!(top.contains("○ awake off"), "{top}");
        // A narrow header keeps the tabs whole and shortens the chip.
        for w in [60, 80, 100, 120] {
            let top = screen(&mut a, w, 24)[0].clone();
            assert!(top.contains(" Runs"), "{w}: {top}");
        }
        // Esc leaves it as it is.
        press(&mut a, KeyCode::Char('W'));
        press(&mut a, KeyCode::Esc);
        assert_eq!(awake::mode(&a.paths), awake::Mode::Off);
        assert!(screen_at_top(&mut a, 100, 40).contains("awake"));
    }

    #[test]
    fn a_going_run_is_stopped_only_after_y() {
        use std::os::unix::process::ExitStatusExt;
        let mut a = app("stop");
        a.icons.nerd = false;
        // The run of "Morning report" is going, in another Herdr session.
        let mut run = a.all_runs[0].clone();
        run.status = store::Status::Running;
        run.server = "another-session".into();
        run.save(&a.paths).unwrap();
        a.reload();
        a.select_id("morning-report");
        assert!(footer(&mut a, 160, 30).contains("x stop its run"));
        screen(&mut a, 120, 30);
        let row = hit_rect(&a, Hit::Row(0));
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Right),
            row.x + 4,
            row.y,
        );
        let Mode::Menu(m) = a.mode.clone() else {
            panic!("no menu");
        };
        assert!(m
            .items
            .iter()
            .any(|i| i.label == "Stop its run…" && i.danger));
        press(&mut a, KeyCode::Char('x'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("x does not ask: {:?}", a.mode);
        };
        assert_eq!(c.question, "Stop this run?");
        assert!(c.lines[0].starts_with("Morning report, started "), "{c:?}");
        assert_eq!(
            c.lines[1],
            "Its tab is closed; what it wrote so far is kept."
        );
        assert!(c.danger() && !c.enter_confirms());
        let all = screen(&mut a, 100, 30).join("\n");
        assert!(all.contains(" y stop ") && all.contains(" n no "), "{all}");
        // ⏎ is no answer: the run goes on.
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::Normal));
        assert!(a.flash.is_none());
        // y stops it; here it cannot, and says why.
        press(&mut a, KeyCode::Char('x'));
        press(&mut a, KeyCode::Char('y'));
        let flash = a.flash.clone().unwrap();
        assert_eq!(
            flash.text,
            "Not stopped: it runs in another Herdr session; stop it there"
        );
        assert!(flash.sticky);
        // A cron job's command is asked to end; its own process writes
        // down how it ended.
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut cron = add_run(&mut a, "morning-report", 50, store::Status::Running, "");
        cron.cron_pid = child.id();
        cron.cron_started = scheduler::started_at(child.id()).unwrap_or_default();
        cron.save(&a.paths).unwrap();
        a.reload();
        a.view = View::Runs;
        a.sel_run = a.runs.iter().position(|r| r.id == cron.id).unwrap();
        assert!(footer(&mut a, 160, 30).contains("x stop"));
        press(&mut a, KeyCode::Char('x'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("x does not ask: {:?}", a.mode);
        };
        assert_eq!(c.lines[1], "Its command is stopped.");
        press(&mut a, KeyCode::Char('y'));
        let said = a.flash.as_ref().unwrap().text.clone();
        let ended = child.wait().unwrap();
        assert_eq!(said, "Stopping; its run shows it in a moment");
        assert_eq!(ended.signal(), Some(libc::SIGTERM));
        // Its process is gone now: a second try says so.
        press(&mut a, KeyCode::Char('x'));
        press(&mut a, KeyCode::Char('y'));
        assert_eq!(
            a.flash.as_ref().unwrap().text,
            "Not stopped: its process is not there any more"
        );
        // A run that ended has nothing to stop.
        let done = add_run(&mut a, "morning-report", 55, store::Status::Finished, "");
        a.reload();
        a.sel_run = a.runs.iter().position(|r| r.id == done.id).unwrap();
        assert!(!footer(&mut a, 160, 30).contains("x stop"));
        press(&mut a, KeyCode::Char('x'));
        assert!(matches!(a.mode, Mode::Normal));
        assert_eq!(a.flash.as_ref().unwrap().text, "No run is going here");
    }

    #[test]
    fn runs_are_narrowed_to_problems_and_searched() {
        use store::Status;
        let mut a = app("narrow");
        a.icons.nerd = false;
        let other = add_routine(&mut a, "Docs link check", true);
        a.reload();
        add_run(&mut a, &other, 1, Status::Finished, "");
        let failed = add_run(&mut a, &other, 2, Status::Failed, "Ended with exit code 2");
        add_run(&mut a, "morning-report", 3, Status::NeedsYou, "");
        add_run(
            &mut a,
            "morning-report",
            4,
            Status::NoResult,
            "The agent stopped without its summary",
        );
        add_run(
            &mut a,
            &other,
            5,
            Status::Skipped,
            "Missed: more than 2 hours late",
        );
        a.reload();
        a.view = View::Runs;
        let all = a.all_runs.len();
        assert_eq!(a.runs.len(), all);
        // A run without its summary has a sign and a colour of its own,
        // and says why; a skipped time says why too.
        let lines = screen(&mut a, 120, 40);
        let text = lines.join("\n");
        assert!(text.contains("! ") && text.contains("No result"), "{text}");
        assert!(text.contains("The agent stopped without its summary"));
        assert!(text.contains("Missed: more than 2 hours late"), "{text}");
        assert!(footer(&mut a, 160, 40).contains("p problems only"));
        // p: what went wrong, and what needs the owner.
        press(&mut a, KeyCode::Char('p'));
        let statuses: Vec<Status> = a.runs.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [Status::NoResult, Status::NeedsYou, Status::Failed]
        );
        let text = screen(&mut a, 120, 40).join("\n");
        assert!(
            text.contains(&format!("Only  problems only ✕   3 of {all}")),
            "{text}"
        );
        assert!(text.contains("Esc all runs"), "{text}");
        // / asks for a text; it is looked for in name, message and result.
        press(&mut a, KeyCode::Char('/'));
        let Mode::Input(i) = a.mode.clone() else {
            panic!("no search: {:?}", a.mode);
        };
        assert_eq!(i.title, "Search the runs");
        for c in "EXIT code".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.runs.len(), 1);
        assert_eq!(a.runs[0].id, failed.id);
        let text = screen(&mut a, 120, 40).join("\n");
        assert!(
            text.contains(&format!(
                "Only  problems only ✕   “EXIT code” ✕   1 of {all}"
            )),
            "{text}"
        );
        // Together with "only its runs".
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.run_filter.as_deref(), Some(other.as_str()));
        assert_eq!(a.runs.len(), 1);
        for (w, h) in [(160, 50), (100, 30), (80, 24), (60, 20)] {
            let text = screen(&mut a, w, h).join("\n");
            assert!(text.contains("runs of Docs"), "{w}: {text}");
            assert!(text.contains("problems"), "{w}: {text}");
            assert!(text.contains("“EXIT"), "{w}: {text}");
            assert!(text.contains(&format!("1 of {all}")), "{w}: {text}");
        }
        // A ✕ removes its own filter only.
        screen(&mut a, 120, 40);
        let x = hit_rect(&a, Hit::Unfilter(RunFilter::Problems));
        a.click(x.x + 2, x.y);
        assert!(!a.runs_problems);
        assert_eq!(a.runs_search, "EXIT code");
        assert!(a.run_filter.is_some());
        screen(&mut a, 120, 40);
        let x = hit_rect(&a, Hit::Unfilter(RunFilter::Routine));
        a.click(x.x + 2, x.y);
        assert!(a.run_filter.is_none() && a.view == View::Runs);
        assert_eq!(a.runs.len(), 1);
        // A search that finds nothing says so, and Esc shows all again.
        press(&mut a, KeyCode::Char('/'));
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "zebra".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        press(&mut a, KeyCode::Enter);
        assert!(a.runs.is_empty());
        let text = screen(&mut a, 120, 40).join("\n");
        assert!(text.contains("No run matches."), "{text}");
        assert!(text.contains("Esc shows all runs again."), "{text}");
        press(&mut a, KeyCode::Esc);
        assert!(!a.runs_filtered());
        assert_eq!(a.runs.len(), all);
        assert_eq!(a.view, View::Runs);
        // Esc in a search leaves the filters as they were.
        press(&mut a, KeyCode::Char('p'));
        press(&mut a, KeyCode::Char('/'));
        press(&mut a, KeyCode::Char('q'));
        press(&mut a, KeyCode::Esc);
        assert!(a.runs_problems && a.runs_search.is_empty());
        // The filters stay when new runs come in.
        add_run(
            &mut a,
            &other,
            6,
            Status::TimedOut,
            "Stopped after 5 minutes",
        );
        a.reload();
        assert_eq!(a.runs.len(), 4);
        assert_eq!(a.runs[0].status, Status::TimedOut);
        // "Its runs" on a routine shows all of them, whatever narrowed
        // the tab before; p narrows them, and Esc drops both filters and
        // goes back to the routine.
        a.view = View::Routines;
        a.select_id(&other);
        press(&mut a, KeyCode::Char('h'));
        assert!(!a.runs_problems);
        assert_eq!(a.run_filter.as_deref(), Some(other.as_str()));
        assert_eq!(a.runs.len(), a.run_count(&other));
        press(&mut a, KeyCode::Char('p'));
        assert_eq!(a.runs.len(), 2);
        let text = screen(&mut a, 140, 40).join("\n");
        assert!(text.contains("Esc back to the routine"), "{text}");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.view, View::Routines);
        assert!(!a.runs_filtered());
        assert_eq!(a.selected_routine().unwrap().id, other);
    }

    #[test]
    fn enter_opens_a_runs_page() {
        use store::Status;
        let mut a = app("page");
        a.icons.nerd = false;
        let mut run = add_run(
            &mut a,
            "morning-report",
            10,
            Status::Failed,
            "Ended with exit code 2",
        );
        run.trigger = store::Trigger::Retry;
        run.attempt = 1;
        run.result = "3 links are broken.\nSee the list in links.md.".into();
        run.save(&a.paths).unwrap();
        let out: Vec<String> = (1..=60).map(|i| format!("checked page {i}")).collect();
        std::fs::write(run.file(&a.paths, "out"), out.join("\n")).unwrap();
        a.reload();
        a.view = View::Runs;
        a.sel_run = a.runs.iter().position(|r| r.id == run.id).unwrap();
        assert!(footer(&mut a, 160, 30).contains("⏎ open"));
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.mode, Mode::RunPage(_)), "{:?}", a.mode);
        for (w, h) in [(160, 50), (120, 40), (100, 30), (80, 24), (60, 20)] {
            let lines = screen(&mut a, w, h);
            let text = lines.join("\n");
            for part in [
                "Morning report · ",
                "Status      ✗ Failed",
                "Reason      Ended with exit code 2",
                "Started     ",
                "Took        ",
                "Started by  A retry after a run that failed · try 2",
                "Result      3 links are broken.",
                "End of the output",
                "checked page 60",
            ] {
                assert!(text.contains(part), "{w}x{h}: no {part:?} in\n{text}");
            }
            assert!(!text.contains("checked page 1 "), "{w}x{h}: {text}");
            // What can be done with it is in the footer.
            let foot = &lines[h as usize - 1];
            assert!(foot.contains("Esc close"), "{foot}");
            if w >= 100 {
                for key in ["l whole output", "r run again", "f open file", "d delete"] {
                    assert!(foot.contains(key), "{w}: no {key} in {foot}");
                }
            }
            assert!(!foot.contains("x stop") && !foot.contains("go to its tab"));
        }
        // l shows the whole output; d and r ask as they do in the list.
        press(&mut a, KeyCode::Char('l'));
        assert!(matches!(a.mode, Mode::Log(_)));
        press(&mut a, KeyCode::Esc);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char('d'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("d does not ask: {:?}", a.mode);
        };
        assert_eq!(
            c.purpose,
            ConfirmFor::DeleteRun(run.routine_id.clone(), run.id.clone())
        );
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char('r'));
        let Mode::Confirm(c) = a.mode.clone() else {
            panic!("r does not ask: {:?}", a.mode);
        };
        assert_eq!(c.question, "Run Morning report now?");
        assert!(!c.enter_confirms());
        press(&mut a, KeyCode::Char('n'));
        // f opens the file and stays on the page.
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char('f'));
        assert!(matches!(a.mode, Mode::RunPage(_)));
        assert_eq!(a.flash.as_ref().unwrap().text, "Opened the output file");
        press(&mut a, KeyCode::Esc);
        assert!(matches!(a.mode, Mode::Normal));

        // A run that is going: how long, and x stops it (after asking).
        let mut going = add_run(&mut a, "morning-report", 20, Status::Running, "");
        going.asleep_secs = 600;
        going.pane_id = "p-1".into();
        going.save(&a.paths).unwrap();
        a.reload();
        a.sel_run = a.runs.iter().position(|r| r.id == going.id).unwrap();
        press(&mut a, KeyCode::Enter);
        let lines = screen(&mut a, 120, 40);
        let text = lines.join("\n");
        assert!(text.contains("Status      ↻ Running"), "{text}");
        assert!(
            text.contains("Going for   30 min · not counting 10 min the computer slept"),
            "{text}"
        );
        assert!(text.contains("Started by  The schedule"), "{text}");
        assert!(
            text.contains("Nothing is saved yet: o goes to its tab"),
            "{text}"
        );
        let foot = &lines[39];
        assert!(
            foot.contains("o go to its tab") && foot.contains("x stop"),
            "{foot}"
        );
        assert!(!foot.contains("d delete"), "{foot}");
        press(&mut a, KeyCode::Char('d'));
        assert!(matches!(a.mode, Mode::RunPage(_)), "a going run stays");
        press(&mut a, KeyCode::Char('x'));
        assert!(
            matches!(&a.mode, Mode::Confirm(c) if matches!(c.purpose, ConfirmFor::StopRun(..)))
        );
        press(&mut a, KeyCode::Char('n'));
        // Its output shows as soon as there is some.
        press(&mut a, KeyCode::Enter);
        std::fs::write(going.file(&a.paths, "out"), "first step done\n").unwrap();
        a.reload();
        let text = screen(&mut a, 120, 40).join("\n");
        assert!(text.contains("first step done"), "{text}");
        // The page closes when its run is deleted elsewhere.
        store::remove(&a.paths, &going);
        a.reload();
        assert!(matches!(a.mode, Mode::Normal));

        // A time at which nothing ran: when and why, nothing else.
        let skipped = add_run(
            &mut a,
            "morning-report",
            30,
            Status::Skipped,
            "Missed: more than 2 hours late",
        );
        a.reload();
        a.sel_run = a.runs.iter().position(|r| r.id == skipped.id).unwrap();
        press(&mut a, KeyCode::Enter);
        let text = screen(&mut a, 120, 40).join("\n");
        assert!(text.contains("Status      ⊘ Skipped"), "{text}");
        assert!(text.contains("Reason      Missed: more than 2 hours late"));
        assert!(text.contains("When        ") && text.contains(" · Scheduled"));
        assert!(
            !text.contains("Took") && !text.contains("Started by"),
            "{text}"
        );
        assert!(text.contains("Nothing ran at this time."), "{text}");
        // A click beside the page closes it.
        a.click(0, 5);
        assert!(matches!(a.mode, Mode::Normal));
        // In the list, o goes to the tab or, once that is closed, shows
        // the output; the menu names the same keys.
        a.sel_run = a.runs.iter().position(|r| r.id == run.id).unwrap();
        press(&mut a, KeyCode::Char('o'));
        assert!(matches!(a.mode, Mode::Log(_)));
        press(&mut a, KeyCode::Esc);
        screen(&mut a, 120, 40);
        let row = hit_rect(&a, Hit::Row(a.sel_run));
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Right),
            row.x + 4,
            row.y,
        );
        let Mode::Menu(m) = a.mode.clone() else {
            panic!("no menu");
        };
        let labels: Vec<(&str, &str)> = m.items.iter().map(|i| (i.label.as_str(), i.key)).collect();
        assert_eq!(
            labels,
            [
                ("Open", "⏎"),
                ("Its output", "l"),
                ("Open the output file", "f"),
                ("Run its routine again…", "r"),
                ("Only its runs", "h"),
                ("Delete this run…", "d"),
            ]
        );
    }

    #[test]
    fn what_needs_the_owner_comes_first() {
        use store::Status;
        let mut a = app("first");
        a.icons.nerd = false;
        let alpha = add_routine(&mut a, "Alpha backup", true);
        let beta = add_routine(&mut a, "Beta report", true);
        let gamma = add_routine(&mut a, "Gamma review", true);
        let delta = add_routine(&mut a, "Delta digest", false);
        a.reload();
        add_run(&mut a, &alpha, 1, Status::Finished, "");
        add_run(&mut a, &beta, 2, Status::Failed, "Ended with exit code 2");
        add_run(&mut a, &gamma, 3, Status::NeedsYou, "");
        add_run(&mut a, &delta, 4, Status::Finished, "");
        a.reload();
        // By name, as the tests' list starts.
        let names =
            |a: &App| -> Vec<String> { a.routines.iter().map(|r| r.file.name.clone()).collect() };
        assert_eq!(
            names(&a),
            [
                "Alpha backup",
                "Beta report",
                "Delta digest",
                "Gamma review",
                "Morning report",
                "Weekly cleanup"
            ]
        );
        assert!(footer(&mut a, 200, 30).contains("s sort by what needs you"));
        // s: needs you, went wrong, to approve, running, by next time, off.
        a.select_id(&alpha);
        press(&mut a, KeyCode::Char('s'));
        assert_eq!(
            a.flash.as_ref().unwrap().text,
            "Sorted by what needs you first"
        );
        assert_eq!(
            names(&a),
            [
                "Gamma review",
                "Beta report",
                "Weekly cleanup",
                "Morning report",
                "Alpha backup",
                "Delta digest"
            ]
        );
        assert_eq!(
            a.selected_routine().unwrap().id,
            alpha,
            "the cursor follows"
        );
        assert!(footer(&mut a, 200, 30).contains("s sort by name"));
        // The choice is kept for the next time the list opens.
        assert_eq!(
            std::fs::read_to_string(a.paths.state.join("list-sort")).unwrap(),
            "attention"
        );
        assert_eq!(App::new(a.paths.clone()).sort, Sort::Attention);
        // One line on top says it in numbers, and what comes next.
        let line = screen(&mut a, 140, 40)[2].clone();
        assert!(
            line.contains("1 needs you · 1 went wrong · 1 to approve · 1 running · next: "),
            "{line}"
        );
        assert!(line.contains(" in ") || line.contains(":"), "{line}");
        // A narrow window keeps what needs the owner.
        for w in [100, 80, 60] {
            let line = screen(&mut a, w, 24)[2].clone();
            assert!(line.contains("1 needs you · 1 went wrong"), "{w}: {line}");
            assert!(!line.trim_end().ends_with('·'), "{w}: {line}");
        }
        // A click on a part goes to the routine it counts...
        screen(&mut a, 140, 40);
        let part = hit_rect(&a, Hit::Attention(Attention::ToApprove));
        a.click(part.x + 1, part.y);
        assert_eq!(a.selected_routine().unwrap().id, "weekly-cleanup");
        let part = hit_rect(&a, Hit::Attention(Attention::NeedsYou));
        a.click(part.x + 1, part.y);
        assert_eq!(a.selected_routine().unwrap().id, gamma);
        // ...and "went wrong" to the runs that did.
        let part = hit_rect(&a, Hit::Attention(Attention::WentWrong));
        a.click(part.x + 1, part.y);
        assert_eq!(a.view, View::Runs);
        assert!(a.runs_problems);
        assert_eq!(a.runs.len(), 2);
        press(&mut a, KeyCode::Esc);
        press(&mut a, KeyCode::Char('1'));
        // A routine that waits for a free place says so instead of its
        // next time, and one that went wrong again says how often.
        std::fs::write(
            a.paths.state.join("queue.json"),
            format!(
                r#"{{"{alpha}":{{"slot":"{}","trigger":"scheduled"}}}}"#,
                Local::now().to_rfc3339()
            ),
        )
        .unwrap();
        std::fs::write(
            a.paths.state.join("streaks.json"),
            format!(r#"{{"{beta}":3,"{alpha}":4}}"#),
        )
        .unwrap();
        a.reload();
        assert_eq!(a.failing.get(&beta), Some(&3));
        assert!(!a.failing.contains_key(&alpha), "its last run worked");
        a.select_id(&beta);
        for (w, h) in [(160, 50), (120, 40)] {
            let text = screen(&mut a, w, h).join("\n");
            assert!(text.contains("waits for a free place"), "{w}: {text}");
            assert!(text.contains("3 times in a row"), "{w}: {text}");
            assert!(
                text.contains("went wrong 3 times in a row"),
                "the details say it in full: {text}"
            );
        }
        // The line on top is for the routines; the other tabs have none.
        press(&mut a, KeyCode::Char('2'));
        assert!(!screen(&mut a, 140, 40).join("\n").contains("needs you ·"));
        // Back to names.
        press(&mut a, KeyCode::Char('s'));
        assert_eq!(a.flash.as_ref().unwrap().text, "Sorted by name");
        assert_eq!(names(&a)[0], "Alpha backup");
        assert_eq!(
            std::fs::read_to_string(a.paths.state.join("list-sort")).unwrap(),
            "name"
        );
    }

    /// Made-up accounts of the Multiaccount plugin, beside the config.
    fn add_accounts(a: &mut App) {
        let dir = a.paths.config.parent().unwrap().join("multiaccount");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("accounts.json"),
            r#"{"accounts":[{"id":"main","name":"Main","kind":"main"},{"id":"work-2","name":"Work","kind":"subscription"}]}"#,
        )
        .unwrap();
        a.reload();
    }

    #[test]
    fn the_form_has_weekday_switches_and_shows_the_next_times() {
        let mut a = app("days");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        let form = a.form.as_mut().unwrap();
        form.choose(Field::Repeat, crate::form::Repeat::Days.label());
        form.select(Field::Days);
        for (w, h) in [(100, 30), (80, 24), (60, 20)] {
            let all = screen(&mut a, w, h).join("\n");
            assert!(all.contains("When  Mon, Wed, Fri at 09:00"), "{all}");
            assert!(all.contains(" Next times "), "{all}");
            // Three times, each on a day that is switched on.
            let next = all.lines().find(|l| l.contains(" Next times ")).unwrap();
            assert!(
                ["Mon ", "Wed ", "Fri "].iter().any(|d| next.contains(d)),
                "{next}"
            );
            let row = all
                .lines()
                .find(|l| l.contains(" On ") && l.contains("●"))
                .unwrap_or_else(|| panic!("{all}"));
            assert!(row.contains("Mo") && row.contains("○"), "{row}");
            // Only whole times: one less where three do not fit.
            assert!(!next.contains('…'), "{next}");
            assert!(all.contains("⏎ switch the day"), "{all}");
        }
        // → moves to Tuesday and ⏎ (or space) switches it; 7 is Sunday.
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char('7'));
        let form = a.form.as_ref().unwrap();
        assert_eq!(form.days, [true, true, true, false, true, false, true]);
        assert_eq!(form.schedule().unwrap(), "0 9 * * 0,1,2,3,5");
        // A click switches the day under it.
        screen(&mut a, 100, 30);
        let fri = hit_rect(&a, Hit::Day(4));
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Left),
            fri.x + 1,
            fri.y,
        );
        assert!(!a.form.as_ref().unwrap().days[4]);
        // With no day left, the heading says what is missing.
        let form = a.form.as_mut().unwrap();
        form.days = [false; 7];
        let all = screen(&mut a, 100, 30).join("\n");
        assert!(all.contains("switch on at least one day"), "{all}");
        // Every month: on a day, the last day, the first Monday.
        let form = a.form.as_mut().unwrap();
        form.choose(Field::Repeat, crate::form::Repeat::Monthly.label());
        let all = screen(&mut a, 100, 30).join("\n");
        assert!(all.contains(" Day of the month "), "{all}");
        let form = a.form.as_mut().unwrap();
        form.choose(Field::MonthOn, "nth-1");
        let all = screen(&mut a, 100, 30).join("\n");
        assert!(all.contains("The first Monday"), "{all}");
        assert!(all.contains("‹ Monday ›"), "{all}");
        assert!(all.contains("First Monday of the month at 09:00"), "{all}");
        assert!(!all.contains(" Day of the month "), "{all}");
    }

    #[test]
    fn a_claude_routine_can_name_its_account() {
        let mut a = app("account");
        a.select_id("morning-report");
        press(&mut a, KeyCode::Enter);
        // Without the Multiaccount plugin there is no such row.
        assert!(!a
            .form
            .as_ref()
            .unwrap()
            .all_fields()
            .contains(&Field::Account));
        press(&mut a, KeyCode::Esc);
        add_accounts(&mut a);
        press(&mut a, KeyCode::Enter);
        let form = a.form.as_mut().unwrap();
        assert!(form.all_fields().contains(&Field::Account));
        form.select(Field::Account);
        let all = screen(&mut a, 100, 50).join("\n");
        assert!(all.contains("Multiaccount decides"), "{all}");
        press(&mut a, KeyCode::Enter);
        let Mode::Pick(p) = &a.mode else {
            panic!("no choice of accounts");
        };
        let names: Vec<&str> = p.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(names, ["Multiaccount decides", "Main", "Work"]);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.form.as_ref().unwrap().extra.account, "work-2");
        // A shell command has no account.
        let form = a.form.as_mut().unwrap();
        form.choose(Field::Agent, routine::SHELL);
        assert!(!form.all_fields().contains(&Field::Account));
        assert!(form.extra.account.is_empty());
    }

    #[test]
    fn further_settings_show_in_the_details_and_the_approval_view() {
        let mut a = app("further");
        add_accounts(&mut a);
        let mut r = Routine::load(&a.paths, "weekly-cleanup").unwrap();
        r.file.more = routine::More {
            late_hours: 2,
            after_command: "~/bin/tell-me.sh".into(),
            after_always: false,
            off_after_failures: 3,
            account: "work-2".into(),
        };
        r.save_approved(&a.paths).unwrap();
        a.reload();
        a.select_id("weekly-cleanup");
        let all = screen(&mut a, 120, 44).join("\n");
        for part in [
            "If missed       runs once, if under 2 hours late",
            "Switches off    after 3 runs in a row that went wrong",
            "After the run   ~/bin/tell-me.sh · after a run that went wrong",
            "Account         Work",
        ] {
            assert!(all.contains(part), "no {part:?} in\n{all}");
        }
        // The approval view names everything that will run.
        let text = approval_lines(&a.paths, &r).join("\n");
        for part in [
            "When a time is missed: A catch-up run only within 2 hours of its time",
            "Switches itself off: after 3 runs in a row that went wrong",
            "After a run that went wrong (a shell command, as you, without asking, in the folder above): ~/bin/tell-me.sh",
            "Account: Work (Multiaccount id work-2), used also when it is at its limit",
        ] {
            assert!(text.contains(part), "no {part:?} in\n{text}");
        }
        r.file.more.after_always = true;
        r.file.more.account = "gone-1".into();
        let text = approval_lines(&a.paths, &r).join("\n");
        assert!(text.contains("After every run (a shell command"), "{text}");
        assert!(
            text.contains(
                "Account: gone-1 (not in Multiaccount any more) (Multiaccount id gone-1)"
            ),
            "{text}"
        );
        // The form opens and saves them untouched.
        press(&mut a, KeyCode::Enter);
        a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let saved = Routine::load(&a.paths, "weekly-cleanup").unwrap();
        assert_eq!(saved.file.more.late_hours, 2);
        assert_eq!(saved.file.more.after_command, "~/bin/tell-me.sh");
        assert_eq!(saved.file.more.off_after_failures, 3);
        assert_eq!(saved.file.more.account, "work-2");
        // The brief for Claude knows the settings and the schedule forms.
        let brief = agent_brief(&a.paths);
        for word in [
            "late_hours",
            "after_command",
            "after_always",
            "off_after_failures",
            "account = ",
            "@every 2 weeks from",
            "mon#1",
            "LW",
            "HERDR_ROUTINES_STATUS",
        ] {
            assert!(brief.contains(word), "{word}");
        }
    }
}
