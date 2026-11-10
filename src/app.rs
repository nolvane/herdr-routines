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
use crate::theme::{Icons, ACCENT, GREEN, MUTED, RED, YELLOW};
use crate::ui;
use chrono::{DateTime, Local};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::DefaultTerminal;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RELOAD: Duration = Duration::from_secs(2);
const FLASH: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Routines,
    Runs,
}

impl View {
    pub const ORDER: [View; 2] = [View::Routines, View::Runs];

    pub fn name(self) -> &'static str {
        match self {
            View::Routines => "Routines",
            View::Runs => "Runs",
        }
    }

    fn other(self) -> View {
        match self {
            View::Routines => View::Runs,
            View::Runs => View::Routines,
        }
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
    /// The form field it fills; None for "how to create a routine".
    pub field: Option<Field>,
    /// Typed to narrow a long list (such as the models an agent lists).
    pub filter: String,
    /// Every entry; `items` is what the filter leaves.
    pub all: Vec<crate::form::Item>,
}

impl Picker {
    pub fn new(
        title: String,
        items: Vec<crate::form::Item>,
        sel: usize,
        field: Option<Field>,
    ) -> Picker {
        Picker {
            title,
            all: items.clone(),
            items,
            sel,
            field,
            filter: String::new(),
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
    }
}

/// The choices behind `n`.
const BY_FORM: &str = "form";
const BY_AGENT: &str = "agent";

#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub title: String,
    pub hint: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmFor {
    Run(String),
    Delete(String),
    DiscardForm,
    Autostart,
    AutostartOff,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    pub question: String,
    pub lines: Vec<String>,
    pub purpose: ConfirmFor,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Help,
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
    AskQuit,
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
}

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

pub fn took(run: &Run) -> String {
    let end = run.ended.unwrap_or_else(Local::now);
    let s = (end - run.started).num_seconds().max(0);
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
    pub sel_routine: usize,
    pub sel_run: usize,
    pub scheduler: Option<(u32, String)>,
    pub mode: Mode,
    pub form: Option<Box<Form>>,
    pub editor: Option<Editor>,
    pub help_scroll: u16,
    pub help_max: u16,
    pub flash: Option<(String, Color, Instant)>,
    pub quit: bool,
    pub hits: Vec<(Rect, Hit)>,
    pub opener: Opener,
    /// The editor's text should go to the person's own editor.
    external: bool,
    last_load: Instant,
    last_click: Option<(Instant, Hit)>,
    /// What was copied last, for pasting when there is no system clipboard.
    clip: String,
    /// Ask about starting at login once the intro is closed.
    autostart_after_intro: bool,
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
            sel_routine: 0,
            sel_run: 0,
            scheduler: None,
            mode: Mode::Normal,
            form: None,
            editor: None,
            help_scroll: 0,
            help_max: 0,
            flash: None,
            quit: false,
            hits: Vec::new(),
            opener: open_in_browser,
            external: false,
            last_load: Instant::now(),
            last_click: None,
            clip: String::new(),
            autostart_after_intro: false,
        };
        app.reload();
        app
    }

    pub fn reload(&mut self) {
        self.routines = routine::load_all(&self.paths);
        self.all_runs = store::all_runs(&self.paths);
        if let Some(id) = &self.run_filter {
            if !self.routines.iter().any(|r| &r.id == id) {
                self.run_filter = None;
            }
        }
        self.runs = match &self.run_filter {
            Some(id) => self
                .all_runs
                .iter()
                .filter(|r| &r.routine_id == id)
                .cloned()
                .collect(),
            None => self.all_runs.clone(),
        };
        self.scheduler = scheduler::running(&self.paths);
        self.sel_routine = self.sel_routine.min(self.routines.len().saturating_sub(1));
        self.sel_run = self.sel_run.min(self.runs.len().saturating_sub(1));
        self.last_load = Instant::now();
        self.fill_models();
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

    pub fn flash(&mut self, text: impl Into<String>, color: Color) {
        self.flash = Some((text.into(), color, Instant::now() + FLASH));
    }

    pub fn selected_routine(&self) -> Option<&Routine> {
        self.routines.get(self.sel_routine)
    }

    pub fn latest_run(&self, id: &str) -> Option<&Run> {
        self.all_runs.iter().find(|r| r.routine_id == id)
    }

    /// How many runs of a routine are kept.
    pub fn run_count(&self, id: &str) -> usize {
        self.all_runs.iter().filter(|r| r.routine_id == id).count()
    }

    /// Shows the runs of one routine (or all with None).
    pub fn filter_runs(&mut self, routine_id: Option<String>) {
        self.run_filter = routine_id;
        self.sel_run = 0;
        self.view = View::Runs;
        self.reload();
    }

    /// The name of the routine the runs are filtered to.
    pub fn filter_name(&self) -> Option<&str> {
        let id = self.run_filter.as_ref()?;
        self.routines
            .iter()
            .find(|r| &r.id == id)
            .map(|r| r.file.name.as_str())
    }

    // ---- actions ----------------------------------------------------------

    fn fire(&mut self, id: &str) {
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
                "Starting; the scheduler is off, so the run is not followed (S starts it)",
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
        self.reload();
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
            Err(e) => return self.flash(format!("Cannot copy: {e}"), RED),
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
                self.flash(format!("Copied as {}, switched off", copy.file.name), GREEN);
                self.reload();
                if let Some(i) = self.routines.iter().position(|r| r.id == copy.id) {
                    self.sel_routine = i;
                }
            }
            Err(e) => self.flash(format!("Cannot copy: {e}"), RED),
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
        let text = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(visible_text).collect();
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
                "The run is still going: ⏎ on it in Runs goes to its tab."
            } else {
                "No output was saved for this run."
            };
            lines.push(why.into());
            if !run.message.is_empty() {
                lines.push(String::new());
                lines.push(run.message.clone());
            }
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
            "New routine".into(),
            vec![
                item(BY_FORM, "Fill in the form", "Choose every setting yourself"),
                item(
                    BY_AGENT,
                    "Describe it to Claude",
                    "Claude asks what you need in a new tab and creates the routine",
                ),
            ],
            0,
            None,
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
        self.form = Some(Box::new(Form::new(&folder)));
    }

    fn edit_selected(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        if r.unreadable {
            // The form would overwrite the file with stand-ins.
            let text = std::fs::read_to_string(self.paths.routine_dir(&r.id).join("routine.toml"))
                .unwrap_or_default();
            self.open_editor(&text, EditTarget::File(r.id.clone()), "routine.toml");
            self.flash(
                r.problem
                    .unwrap_or_else(|| "routine.toml cannot be read".into()),
                YELLOW,
            );
        } else {
            self.form = Some(Box::new(Form::edit(&r)));
        }
    }

    fn open_editor(&mut self, text: &str, target: EditTarget, title: &str) {
        let mut t = TextArea::new(text);
        t.go_to_end();
        self.editor = Some(Editor {
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
        let waiting = form.id.as_ref().is_some_and(|id| !self.is_approved(id));
        let r = Routine {
            id: id.clone(),
            file,
            prompt,
            problem: None,
            unreadable: false,
            approved: !waiting,
        };
        if waiting {
            r.save(&self.paths)?;
        } else {
            // The owner wrote it in the form: what it saves is approved.
            r.save_approved(&self.paths)?;
        }
        self.reload();
        if let Some(i) = self.routines.iter().position(|r| r.id == id) {
            self.sel_routine = i;
        }
        self.view = View::Routines;
        Ok(if waiting {
            format!("Saved {name}; it still waits for approval (a)")
        } else {
            format!("Saved {name}")
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
            Ok(msg) => self.flash(msg, GREEN),
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
        let was = self.is_approved(id);
        paths::write_atomic(&self.paths.routine_dir(id).join(name), text.as_bytes())
            .map_err(|e| e.to_string())?;
        crate::approval::approve_file(&self.paths, id, was, name, text)
    }

    fn close_form(&mut self) {
        match &self.form {
            Some(f) if f.dirty => {
                self.mode = Mode::Confirm(Confirm {
                    question: "Discard the changes?".into(),
                    lines: vec!["The routine stays as it was.".into()],
                    purpose: ConfirmFor::DiscardForm,
                })
            }
            _ => self.form = None,
        }
    }

    fn ask_run(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
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
        let mut lines = vec![if r.is_shell() {
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

    /// Whether the routine's files are the approved ones right now.
    fn is_approved(&self, id: &str) -> bool {
        Routine::load(&self.paths, id).is_ok_and(|r| r.approved)
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
            lines: approval_lines(&now),
            scroll: 0,
            // Unknown until drawn, so `y` cannot approve unseen text.
            max: u16::MAX,
            file: None,
            approve: Some((r.id, files)),
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
        self.reload();
    }

    fn ask_delete(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        self.mode = Mode::Confirm(Confirm {
            question: format!("Delete {}?", r.file.name),
            lines: vec![
                "The routine, its prompt and its run history go. Tabs of runs still open stay."
                    .into(),
            ],
            purpose: ConfirmFor::Delete(r.id),
        });
    }

    /// `C`: the jobs in the crontab.
    fn show_cron(&mut self) {
        match crate::crontab::read() {
            Ok(text) => {
                let jobs = crate::crontab::parse(&text);
                if jobs.is_empty() {
                    self.flash("No cron jobs in your crontab", MUTED);
                } else {
                    self.mode = Mode::Cron(jobs);
                }
            }
            Err(e) => self.flash(format!("Could not read the crontab: {e}"), RED),
        }
    }

    fn import_cron(&mut self, jobs: Vec<crate::crontab::CronJob>, moving: bool) {
        self.mode = Mode::Normal;
        match crate::crontab::import(&self.paths, &jobs, moving) {
            Ok(n) => {
                let what = if n == 1 { "1 cron job" } else { "cron jobs" };
                let text = match (moving, n) {
                    (true, 1) => "Moved 1 cron job here".to_string(),
                    (true, _) => format!("Moved {n} {what} here"),
                    (false, 1) => "Copied 1 cron job, switched off".to_string(),
                    (false, _) => format!("Copied {n} {what}, switched off"),
                };
                self.flash(text, GREEN);
            }
            Err(e) => self.flash(e, RED),
        }
        self.view = View::Routines;
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
                    "Herdr then no longer starts at login: after a restart, routines run only once you open Herdr. Herdr running now keeps running.".into(),
                ],
                purpose: ConfirmFor::AutostartOff,
            });
        } else {
            self.ask_autostart();
        }
    }

    fn ask_autostart(&mut self) {
        self.mode = Mode::Confirm(Confirm {
            question: "Start Herdr at login?".into(),
            lines: vec![
                "Herdr then runs in the background from the moment you log in, so routines also run after a restart without opening it. Opening Herdr connects to it as usual.".into(),
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
            ConfirmFor::Delete(id) => {
                match Routine::delete(&self.paths, &id) {
                    Ok(()) => self.flash("Deleted", ACCENT),
                    Err(e) => self.flash(e, RED),
                }
                self.reload();
            }
            ConfirmFor::DiscardForm => self.form = None,
        }
    }

    fn start_scheduler(&mut self) {
        if self.scheduler.is_some() {
            return self.flash("The scheduler is running", GREEN);
        }
        match scheduler::spawn(&self.paths) {
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(300));
                self.reload();
                self.flash("Scheduler started", GREEN);
            }
            Err(e) => self.flash(e, RED),
        }
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
            Some(field),
        ));
    }

    fn picked(&mut self, p: Picker) {
        self.mode = Mode::Normal;
        let Some(item) = p.items.get(p.sel) else {
            return;
        };
        let Some(field) = p.field else {
            match item.value.as_str() {
                BY_AGENT => self.create_with_agent(),
                _ => self.new_form(),
            }
            return;
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
            });
            return;
        }
        if let Some(form) = self.form.as_mut() {
            form.choose(field, &item.value);
        }
    }

    fn entered(&mut self, input: Input) {
        self.mode = Mode::Normal;
        if let Some(form) = self.form.as_mut() {
            form.choose(Field::Model, input.text.trim());
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
            f if f.is_choice() => self.open_picker(),
            _ => form.move_by(1),
        }
    }

    // ---- keys -------------------------------------------------------------

    fn rows_len(&self) -> usize {
        match self.view {
            View::Routines => self.routines.len(),
            View::Runs => self.runs.len(),
        }
    }

    fn cursor(&mut self) -> &mut usize {
        match self.view {
            View::Routines => &mut self.sel_routine,
            View::Runs => &mut self.sel_run,
        }
    }

    fn step(&mut self, by: i32) {
        let len = self.rows_len();
        if len == 0 {
            return;
        }
        let c = self.cursor();
        *c = (*c as i32 + by).clamp(0, len as i32 - 1) as usize;
    }

    fn enter(&mut self) {
        match self.view {
            View::Routines => self.edit_selected(),
            View::Runs => {
                let run = self.runs.get(self.sel_run).cloned();
                self.open_run(run);
            }
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
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
            Mode::Confirm(c) => match k.code {
                KeyCode::Char('y') => self.confirmed(c),
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
            Mode::Message { .. } => {
                if matches!(
                    k.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ')
                ) {
                    self.mode = Mode::Normal;
                    if std::mem::take(&mut self.autostart_after_intro) {
                        self.ask_autostart_once();
                    }
                }
            }
            Mode::Cron(jobs) => match k.code {
                KeyCode::Char('m') => self.import_cron(jobs, true),
                KeyCode::Char('c') => self.import_cron(jobs, false),
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
            KeyCode::Home if !field.is_text() => form.field = 0,
            KeyCode::End if !field.is_text() => form.move_by(99),
            KeyCode::Left => form.cycle(-1),
            KeyCode::Right => form.cycle(1),
            KeyCode::Enter => self.form_enter(),
            KeyCode::Backspace => form.backspace(),
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
            // Esc in a filtered Runs view shows all runs again.
            KeyCode::Esc if self.run_filter.is_some() && self.view == View::Runs => {
                self.filter_runs(None)
            }
            KeyCode::Esc => {}
            KeyCode::Char('h') => {
                let id = match self.view {
                    View::Routines => self.selected_routine().map(|r| r.id.clone()),
                    View::Runs => self.runs.get(self.sel_run).map(|r| r.routine_id.clone()),
                };
                if id.is_some() && id == self.run_filter && self.view == View::Runs {
                    self.filter_runs(None);
                } else if id.is_some() {
                    self.filter_runs(id);
                }
            }
            KeyCode::Char('f') if self.view == View::Runs => {
                if let Some(run) = self.runs.get(self.sel_run).cloned() {
                    self.open_output_file(&run);
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
            KeyCode::Char('g') | KeyCode::Home => *self.cursor() = 0,
            KeyCode::Char('G') | KeyCode::End => {
                let last = self.rows_len().saturating_sub(1);
                *self.cursor() = last;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::BackTab | KeyCode::Left => {
                self.view = self.view.other()
            }
            KeyCode::Char('1') => self.view = View::Routines,
            KeyCode::Char('2') => self.view = View::Runs,
            KeyCode::Char('S') => self.start_scheduler(),
            KeyCode::Char('A') => self.toggle_autostart(),
            KeyCode::Char('C') => self.show_cron(),
            KeyCode::Char('i') => self.toggle_icons(),
            KeyCode::Char('n') => self.new_routine(),
            KeyCode::Enter => self.enter(),
            KeyCode::Char('l') => {
                let run = match self.view {
                    View::Routines => self
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
            _ if self.view == View::Routines && self.selected_routine().is_some() => match k.code {
                KeyCode::Char('e') => self.edit_selected(),
                KeyCode::Char('p') => {
                    let r = self.selected_routine().cloned().unwrap();
                    let what = if r.is_shell() { "Command" } else { "Prompt" };
                    let title = format!("{what} of {}", r.file.name);
                    let name = Routine::text_file(&r.file.agent);
                    let text = std::fs::read_to_string(self.paths.routine_dir(&r.id).join(name))
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

    fn click(&mut self, x: u16, y: u16) {
        let Some((rect, hit)) = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .copied()
        else {
            return;
        };
        let double = self
            .last_click
            .is_some_and(|(at, h)| h == hit && at.elapsed() < Duration::from_millis(400));
        self.last_click = Some((Instant::now(), hit));
        let normal = matches!(self.mode, Mode::Normal);
        match hit {
            Hit::View(v) => {
                if normal {
                    self.view = v;
                }
            }
            Hit::Row(i) => {
                if normal {
                    *self.cursor() = i;
                    if double {
                        self.enter();
                    }
                }
            }
            Hit::RunsOf(i) => {
                if normal {
                    self.sel_routine = i;
                    let id = self.routines.get(i).map(|r| r.id.clone());
                    self.filter_runs(id);
                }
            }
            Hit::PickRow(i) => {
                if let Mode::Pick(mut p) = self.mode.clone() {
                    p.sel = i;
                    if double {
                        self.picked(p);
                    } else {
                        self.mode = Mode::Pick(p);
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
                form.field = i;
                let f = form.current();
                // A second click (or one on a button) acts.
                if (again || f == Field::Save || f == Field::Prompt) && !f.is_text() {
                    self.form_enter();
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
            Mode::Help => self.scroll_help(by * 2),
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
                    self.step(by);
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
                MouseEventKind::Drag(MouseButton::Left) => self.drag(m.column, m.row),
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
        let file = self.paths.state.join("editing.md");
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
                .is_some_and(|(_, _, until)| Instant::now() >= *until)
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
fn approval_lines(r: &Routine) -> Vec<String> {
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
        format!("When a time is missed: {}", f.missed.detail()),
        format!("Folder: {}", f.folder),
        format!(
            "Time limit: {} minutes{}",
            f.time_limit_minutes,
            if f.stop_at_limit { ", then it is stopped" } else { "" }
        ),
    ];
    if f.retries > 0 {
        lines.push(format!(
            "Retries: {} after a failure, {} minutes apart",
            f.retries, f.retry_delay_minutes
        ));
    }
    if r.is_shell() {
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
        let flags = agent::flags(f);
        if !flags.is_empty() {
            lines.push(format!("Options: {}", flags.join(" ")));
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
time_limit_minutes = 120
stop_at_limit = false             # true: close the run's tab at the time limit
retries = 0                       # try a failed run again this many times (0-3)...
retry_delay_minutes = 10          # ...this many minutes apart
check_command = ""                # agents only: a shell command run first; the agent starts only if it exits 0, and gets its output
remember = false                  # agents only: each run gets the results of the last ten runs

Instead of cron, schedule can be "@once YYYY-MM-DD HH:MM" (runs once, then the routine switches off) or "@start" (each time Herdr starts).

What to do:
1. Ask me what the routine should do, how often and when, and in which folder, unless I already said it. Ask everything you need in one short message. Suggest sensible choices (a cheaper model for simple jobs, "auto" so nobody has to approve steps, a plain shell command when no AI is needed).
2. For an agent routine, write prompt.md as clear instructions for an agent working alone: what to do, where, what to report at the end, and what it must not do. For a shell routine, write command.sh.
3. Show me a short summary (name, schedule in words, agent, model, folder) and create the files once I agree. Do not change or delete other routines unless I ask.
4. Run `{bin} routines` and check that the new routine shows "problem": null; fix the files if not. It shows "approved": false, which is expected: a routine that was not saved in the list runs only after I approve it there. Do not try to change that. Then tell me it is ready and that I approve it in the list (Ctrl+B, U, then a on the routine)."#
    )
}

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

    #[test]
    fn cron_jobs_move_here() {
        let mut a = app("cron");
        let dir = a.paths.state.clone();
        let tab = dir.join("fake-crontab.txt");
        std::fs::write(
            &tab,
            "PATH=/usr/bin:/bin\n0 2 * * * /opt/jobs/backup.sh --all\n@reboot ~/start.sh\n",
        )
        .unwrap();
        // A stand-in for crontab: -l prints the file, a path replaces it.
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
        std::env::set_var("HERDR_ROUTINES_CRONTAB", &fake);
        press(&mut a, KeyCode::Char('C'));
        let Mode::Cron(jobs) = a.mode.clone() else {
            panic!("no cron list: {:?}", a.flash);
        };
        assert_eq!(jobs.len(), 2);
        let lines = screen(&mut a, 100, 30);
        assert!(lines
            .iter()
            .any(|l| l.contains("2 cron jobs in your crontab")));
        assert!(lines.iter().any(|l| l.contains("When Herdr starts")));
        press(&mut a, KeyCode::Char('m'));
        std::env::remove_var("HERDR_ROUTINES_CRONTAB");
        let moved: Vec<&Routine> = a
            .routines
            .iter()
            .filter(|r| r.file.agent == routine::SHELL)
            .collect();
        assert_eq!(moved.len(), 2, "{:?}", a.flash);
        assert!(moved.iter().all(|r| r.file.enabled && r.problem.is_none()));
        assert!(moved.iter().any(|r| r.file.name == "backup.sh --all"));
        let now = std::fs::read_to_string(&tab).unwrap();
        assert!(now.contains("# moved to herdr-routines (routine "), "{now}");
        assert!(crate::crontab::parse(&now).is_empty());
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e
            .file_name()
            .to_string_lossy()
            .starts_with("crontab-backup-")));
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
        assert!(at(" Schedule       Weekdays").is_some_and(|i| i > head));
        assert!(at("Do task").is_some_and(|i| i > head), "the prompt shows");
        // The routines follow each other without empty rows.
        let first = at(&name).unwrap();
        let other = &a.routines[1].file.name;
        assert_eq!(at(other).unwrap(), first + 2);
        // A short window shows only the list.
        let lines = screen(&mut a, 110, 20);
        assert!(!lines.iter().any(|l| l.contains(&format!(" {name} ─"))));
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
        assert!(lines
            .iter()
            .any(|l| l.contains(&format!("Runs of {}", ids[1].1))));
        assert!(lines.iter().any(|l| l.contains("Result 1.1")));
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.runs.len(), a.all_runs.len());
        assert!(a.run_filter.is_none());
    }

    #[test]
    fn a_routine_written_outside_the_list_waits_for_approval() {
        let mut a = app("approve");
        let all = screen(&mut a, 120, 30).join("\n");
        assert!(all.contains("a approves"), "{all}");
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
    fn the_prompt_sits_above_the_save_button() {
        let mut a = app("layout");
        press(&mut a, KeyCode::Char('n'));
        press(&mut a, KeyCode::Enter);
        let lines = screen(&mut a, 90, 30);
        let at = |s: &str| lines.iter().position(|l| l.contains(s)).unwrap();
        let prompt = at(" Prompt ");
        let save = at("⏎ save");
        assert!(
            at(" On ") < prompt,
            "the prompt comes after the other fields"
        );
        assert_eq!(save, prompt + 2, "one empty row before the button");
        assert!(lines[prompt + 1].trim().is_empty());
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
        let lines = screen(&mut a, 90, 30);
        assert!(lines.iter().any(|l| l.contains("Step 1 of the task. …")));
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
}
