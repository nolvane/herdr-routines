//! The list's state and what the keys and the mouse do. Drawing is in
//! `ui.rs`.

use crate::editor::TextArea;
use crate::form::{Field, Form, OTHER_MODEL};
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
    pub field: Field,
}

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
    pub runs: Vec<Run>,
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
        };
        app.reload();
        app
    }

    pub fn reload(&mut self) {
        self.routines = routine::load_all(&self.paths);
        self.runs = store::all_runs(&self.paths);
        self.scheduler = scheduler::running(&self.paths);
        self.sel_routine = self.sel_routine.min(self.routines.len().saturating_sub(1));
        self.sel_run = self.sel_run.min(self.runs.len().saturating_sub(1));
        self.last_load = Instant::now();
    }

    pub fn flash(&mut self, text: impl Into<String>, color: Color) {
        self.flash = Some((text.into(), color, Instant::now() + FLASH));
    }

    pub fn selected_routine(&self) -> Option<&Routine> {
        self.routines.get(self.sel_routine)
    }

    pub fn latest_run(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.routine_id == id)
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
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        if r.unreadable || (r.problem.is_some() && !r.file.enabled) {
            return self.flash("Fix the routine first (⏎ edits it)", YELLOW);
        }
        let mut r = r;
        r.file.enabled = !r.file.enabled;
        match r.save(&self.paths) {
            Ok(()) => {
                let state = if r.file.enabled { "on" } else { "off" };
                let color = if r.file.enabled { GREEN } else { RED };
                self.flash(format!("{} is {state}", r.file.name), color);
            }
            Err(e) => self.flash(e, RED),
        }
        self.reload();
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

    fn show_log(&mut self, run: &Run) {
        let text = std::fs::read_to_string(run.log_path(&self.paths)).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
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
        // Open at the end, where the agent's answer is.
        self.mode = Mode::Log(LogView {
            title: format!(
                "{} · {} · {}",
                run.routine_name,
                run.started.format("%a %-d %b %H:%M"),
                run.status.label()
            ),
            lines,
            scroll: u16::MAX,
            max: 0,
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
        let r = Routine {
            id: id.clone(),
            file,
            prompt,
            problem: None,
            unreadable: false,
        };
        r.save(&self.paths)?;
        self.reload();
        if let Some(i) = self.routines.iter().position(|r| r.id == id) {
            self.sel_routine = i;
        }
        self.view = View::Routines;
        Ok(format!("Saved {name}"))
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
            EditTarget::Prompt(id, name) => {
                paths::write_atomic(&self.paths.routine_dir(id).join(name), text.as_bytes())
                    .map(|_| "Saved")
            }
            EditTarget::File(id) => paths::write_atomic(
                &self.paths.routine_dir(id).join("routine.toml"),
                text.as_bytes(),
            )
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
        self.mode = Mode::Confirm(Confirm {
            question: format!("Run {} now?", r.file.name),
            lines: vec![if r.is_shell() {
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
            }],
            purpose: ConfirmFor::Run(r.id),
        });
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

    fn confirmed(&mut self, c: Confirm) {
        self.mode = Mode::Normal;
        match c.purpose {
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
        self.mode = Mode::Pick(Picker {
            title: form.label(field).into(),
            items,
            sel,
            field,
        });
    }

    fn picked(&mut self, p: Picker) {
        self.mode = Mode::Normal;
        let Some(item) = p.items.get(p.sel) else {
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
            form.choose(p.field, &item.value);
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
        if ctrl && k.code == KeyCode::Char('c') {
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
                }
            }
            Mode::Log(mut log) => {
                let page = 10;
                match k.code {
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
            KeyCode::Esc => {}
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
            KeyCode::Char('i') => self.toggle_icons(),
            KeyCode::Char('n') => self.new_form(),
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
                KeyCode::Char(' ') => self.toggle(),
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
            return;
        }
        if paths::write_atomic(&seen, b"").is_ok() {
            self.mode = Mode::Message {
                title: "How routines work".into(),
                lines: INTRO.iter().map(|l| l.to_string()).collect(),
            };
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
            Routine {
                id: routine::new_id(&paths, name),
                file,
                prompt,
                problem: None,
                unreadable: false,
            }
            .save(&paths)
            .unwrap();
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
    fn the_selected_field_explains_itself() {
        let mut a = app("explain");
        press(&mut a, KeyCode::Char('n'));
        a.form.as_mut().unwrap().select(Field::Mode);
        for (w, h) in [(90, 34), (80, 24)] {
            let lines = screen(&mut a, w, h);
            assert!(lines.iter().any(|l| l.contains("▎ Permission mode")));
            assert!(lines.iter().any(|l| l.contains("▎ What the agent may do")));
        }
    }

    #[test]
    fn the_form_saves_mode_and_model() {
        let mut a = app("form");
        press(&mut a, KeyCode::Char('n'));
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
