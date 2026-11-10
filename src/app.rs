//! The list: routines, their runs, the form and a run's output.

use crate::cron::{short_time, Cron};
use crate::form::{Field, Form};
use crate::paths::{self, Paths};
use crate::routine::{self, Routine, Settings, AGENTS};
use crate::runner;
use crate::scheduler;
use crate::store::{self, Run};
use crate::theme::{self, Icons};
use chrono::{DateTime, Local};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RELOAD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Routines,
    Runs,
}

enum Pending {
    Run(String),
    Delete(String),
    Discard(Box<Form>),
    Quit,
}

/// What waits to be opened in the editor.
enum Edit {
    /// A routine's `prompt.md`.
    Prompt(String),
    /// A routine's `routine.toml`, when it cannot be read.
    File(String),
    /// The prompt in the form.
    Form,
}

enum Mode {
    List,
    Confirm(String, Pending),
    Form(Box<Form>),
    Log {
        title: String,
        lines: Vec<String>,
        scroll: usize,
    },
}

pub struct App {
    paths: Paths,
    icons: Icons,
    view: View,
    routines: Vec<Routine>,
    runs: Vec<Run>,
    sel_routine: usize,
    sel_run: usize,
    scheduler: Option<(u32, String)>,
    mode: Mode,
    flash: Option<(String, bool)>,
    last_load: Instant,
    quit: bool,
    editor: Option<Edit>,
}

fn ago(t: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let m = (*now - *t).num_minutes();
    match m {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{m} min ago"),
        60..=1439 => format!("{} h ago", m / 60),
        _ => format!("{} d ago", m / 1440),
    }
}

fn until(t: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let m = (*t - *now).num_minutes();
    if m < 60 {
        format!("in {} min", m.max(1))
    } else {
        short_time(t, now)
    }
}

fn took(run: &Run) -> String {
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

impl App {
    pub fn new(paths: Paths) -> App {
        let settings = Settings::load(&paths);
        let icons = Icons {
            nerd: settings.icons != "plain",
        };
        let mut app = App {
            paths,
            icons,
            view: View::Routines,
            routines: Vec::new(),
            runs: Vec::new(),
            sel_routine: 0,
            sel_run: 0,
            scheduler: None,
            mode: Mode::List,
            flash: None,
            last_load: Instant::now(),
            quit: false,
            editor: None,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.routines = routine::load_all(&self.paths);
        self.runs = store::all_runs(&self.paths);
        self.scheduler = scheduler::running(&self.paths);
        self.sel_routine = self.sel_routine.min(self.routines.len().saturating_sub(1));
        self.sel_run = self.sel_run.min(self.runs.len().saturating_sub(1));
        self.last_load = Instant::now();
    }

    fn say(&mut self, text: impl Into<String>) {
        self.flash = Some((text.into(), false));
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.flash = Some((text.into(), true));
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            if self.last_load.elapsed() >= RELOAD && matches!(self.mode, Mode::List) {
                self.reload();
            }
            terminal.draw(|f| self.render(f))?;
            if self.quit {
                return Ok(());
            }
            if let Some(target) = self.editor.take() {
                ratatui::restore();
                let result = self.edit_prompt(target);
                *terminal = ratatui::init();
                terminal.clear()?;
                match result {
                    Ok(msg) => self.say(msg),
                    Err(e) => self.warn(e),
                }
                self.reload();
                continue;
            }
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.key(k);
                    }
                }
            }
        }
    }

    // ---- actions ------------------------------------------------------------

    fn selected_routine(&self) -> Option<&Routine> {
        self.routines.get(self.sel_routine)
    }

    fn latest_run(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.routine_id == id)
    }

    fn fire(&mut self, id: &str) {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return self.warn(format!("Cannot start: {e}")),
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
            Ok(_) if self.scheduler.is_none() => self.warn(
                "Starting. The scheduler is off, so the run is not followed until it runs (S starts it)",
            ),
            Ok(_) => self.say("Starting, its tab opens in the background"),
            Err(e) => self.warn(format!("Cannot start: {e}")),
        }
        self.reload();
    }

    fn toggle(&mut self) {
        let Some(r) = self.selected_routine().cloned() else {
            return;
        };
        if r.unreadable || (r.problem.is_some() && !r.file.enabled) {
            return self.warn("Fix the routine first (e edits it)");
        }
        let mut r = r;
        r.file.enabled = !r.file.enabled;
        match r.save(&self.paths) {
            Ok(()) => {
                let state = if r.file.enabled { "on" } else { "off" };
                self.say(format!("{} is {state}", r.file.name));
            }
            Err(e) => self.warn(e),
        }
        self.reload();
    }

    fn open_run(&mut self, run: Option<Run>) {
        let Some(run) = run else {
            return self.say("No run yet");
        };
        match runner::show(&self.paths, &run) {
            Ok(()) => self.say("Showing its tab"),
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
                "The run is still going; ⏎ in the list jumps to its tab."
            } else {
                "No output was saved for this run."
            };
            lines.push(why.into());
            if !run.message.is_empty() {
                lines.push(String::new());
                lines.push(run.message.clone());
            }
        }
        let scroll = lines.len().saturating_sub(1);
        self.mode = Mode::Log {
            title: format!(
                "{} · {} · {}",
                run.routine_name,
                run.started.format("%a %-d %b %H:%M"),
                run.status.label()
            ),
            lines,
            scroll,
        };
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
        Ok(format!("Saved {name}"))
    }

    /// Opens a file in the editor: a routine's `prompt.md` or `routine.toml`,
    /// or for the form a scratch file read back into it.
    fn edit_prompt(&mut self, target: Edit) -> Result<String, String> {
        let file = match &target {
            Edit::Prompt(id) => self.paths.routine_dir(id).join("prompt.md"),
            Edit::File(id) => self.paths.routine_dir(id).join("routine.toml"),
            Edit::Form => {
                let f = self.paths.state.join("form-prompt.md");
                let text = match &self.mode {
                    Mode::Form(form) => form.prompt.clone(),
                    _ => String::new(),
                };
                paths::write_atomic(&f, text.as_bytes()).map_err(|e| e.to_string())?;
                f
            }
        };
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|e| !e.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|e| !e.trim().is_empty())
            })
            .unwrap_or_else(|| "nano".into());
        let status = Command::new("sh")
            .args(["-c", &format!("{editor} \"$1\""), "sh"])
            .arg(&file)
            .status()
            .map_err(|e| format!("Cannot open the editor {editor}: {e}"))?;
        if !status.success() {
            return Err(format!("The editor {editor} ended with an error"));
        }
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        match target {
            Edit::Prompt(_) => Ok("Prompt saved".into()),
            Edit::File(_) => Ok("routine.toml saved".into()),
            Edit::Form => {
                let _ = std::fs::remove_file(&file);
                if let Mode::Form(form) = &mut self.mode {
                    if form.prompt != text {
                        form.prompt = text;
                        form.dirty = true;
                    }
                }
                Ok("Prompt updated; save the routine to keep it".into())
            }
        }
    }

    // ---- keys ---------------------------------------------------------------

    fn key(&mut self, k: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, Mode::List);
        self.mode = match mode {
            Mode::List => {
                self.list_key(k);
                std::mem::replace(&mut self.mode, Mode::List)
            }
            Mode::Confirm(text, pending) => self.confirm_key(k, text, pending),
            Mode::Form(form) => self.form_key(k, form),
            Mode::Log {
                title,
                lines,
                scroll,
            } => self.log_key(k, title, lines, scroll),
        };
    }

    fn list_key(&mut self, k: KeyEvent) {
        self.flash = None;
        let (len, sel) = match self.view {
            View::Routines => (self.routines.len(), &mut self.sel_routine),
            View::Runs => (self.runs.len(), &mut self.sel_run),
        };
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => *sel = sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1).min(len.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => *sel = 0,
            KeyCode::End | KeyCode::Char('G') => *sel = len.saturating_sub(1),
            KeyCode::PageUp => *sel = sel.saturating_sub(10),
            KeyCode::PageDown => *sel = (*sel + 10).min(len.saturating_sub(1)),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.view = match self.view {
                    View::Routines => View::Runs,
                    View::Runs => View::Routines,
                }
            }
            KeyCode::Char('q') => {
                self.mode = Mode::Confirm(
                    "Close herdr-routines? (routines keep running)".into(),
                    Pending::Quit,
                )
            }
            KeyCode::Char('S') if self.scheduler.is_some() => self.say("The scheduler is running"),
            KeyCode::Char('S') => match scheduler::spawn(&self.paths) {
                Ok(_) => {
                    std::thread::sleep(Duration::from_millis(300));
                    self.reload();
                    self.say("Scheduler started");
                }
                Err(e) => self.warn(e),
            },
            KeyCode::Char('R') => {
                self.reload();
                self.say("Reloaded");
            }
            _ => match self.view {
                View::Routines => self.routines_key(k),
                View::Runs => self.runs_key(k),
            },
        }
    }

    fn routines_key(&mut self, k: KeyEvent) {
        let selected = self.selected_routine().cloned();
        match k.code {
            KeyCode::Char('n') => {
                let mut form = Form::new(&paths::shorten(
                    &std::env::current_dir().unwrap_or_default(),
                ));
                if let Some(home) = std::env::var_os("HOME") {
                    if std::env::current_dir().ok().as_deref() == Some(std::path::Path::new("/")) {
                        form.folder = paths::shorten(std::path::Path::new(&home));
                    }
                }
                self.mode = Mode::Form(Box::new(form));
            }
            _ if selected.is_none() => {}
            KeyCode::Enter => {
                let run = self.latest_run(&selected.unwrap().id).cloned();
                self.open_run(run);
            }
            KeyCode::Char('l') => match self.latest_run(&selected.unwrap().id).cloned() {
                Some(run) => self.show_log(&run),
                None => self.say("No run yet"),
            },
            KeyCode::Char(' ') => self.toggle(),
            KeyCode::Char('e') => {
                let r = selected.unwrap();
                if r.unreadable {
                    // The form would overwrite the file with defaults.
                    self.editor = Some(Edit::File(r.id));
                } else {
                    self.mode = Mode::Form(Box::new(Form::edit(&r)));
                }
            }
            KeyCode::Char('p') => self.editor = Some(Edit::Prompt(selected.unwrap().id)),
            KeyCode::Char('r') => {
                let r = selected.unwrap();
                if let Some(p) = &r.problem {
                    return self.warn(format!("Cannot run: {p}"));
                }
                self.mode = Mode::Confirm(format!("Run {} now?", r.file.name), Pending::Run(r.id));
            }
            KeyCode::Char('d') => {
                let r = selected.unwrap();
                self.mode = Mode::Confirm(
                    format!("Delete {} and its run history?", r.file.name),
                    Pending::Delete(r.id),
                );
            }
            _ => {}
        }
    }

    fn runs_key(&mut self, k: KeyEvent) {
        let Some(run) = self.runs.get(self.sel_run).cloned() else {
            return;
        };
        match k.code {
            KeyCode::Enter => self.open_run(Some(run)),
            KeyCode::Char('l') => self.show_log(&run),
            _ => {}
        }
    }

    fn confirm_key(&mut self, k: KeyEvent, text: String, pending: Pending) -> Mode {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => match pending {
                Pending::Run(id) => {
                    self.fire(&id);
                    Mode::List
                }
                Pending::Delete(id) => {
                    match Routine::delete(&self.paths, &id) {
                        Ok(()) => self.say("Deleted"),
                        Err(e) => self.warn(e),
                    }
                    self.reload();
                    Mode::List
                }
                Pending::Discard(_) => Mode::List,
                Pending::Quit => {
                    self.quit = true;
                    Mode::List
                }
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => match pending {
                // Back to the form that asked.
                Pending::Discard(form) => Mode::Form(form),
                _ => Mode::List,
            },
            _ => Mode::Confirm(text, pending),
        }
    }

    fn form_key(&mut self, k: KeyEvent, mut form: Box<Form>) -> Mode {
        self.flash = None;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('s') if ctrl => return self.try_save(form),
            KeyCode::Char('u') if ctrl => form.clear(),
            KeyCode::Esc => {
                if form.dirty {
                    // The question keeps the form to return to on "no".
                    return Mode::Confirm("Discard your changes?".into(), Pending::Discard(form));
                }
                return Mode::List;
            }
            KeyCode::Up | KeyCode::BackTab => form.move_by(-1),
            KeyCode::Down | KeyCode::Tab => form.move_by(1),
            KeyCode::Left => form.cycle(-1),
            KeyCode::Right => form.cycle(1),
            KeyCode::Enter => match form.current() {
                Field::Prompt => {
                    self.editor = Some(Edit::Form);
                }
                Field::Save => return self.try_save(form),
                _ => form.move_by(1),
            },
            KeyCode::Backspace => form.backspace(),
            KeyCode::Char(c) if !ctrl => form.type_char(c),
            _ => {}
        }
        Mode::Form(form)
    }

    fn try_save(&mut self, form: Box<Form>) -> Mode {
        match self.save_form(&form) {
            Ok(msg) => {
                self.say(msg);
                Mode::List
            }
            Err(e) => {
                self.warn(format!("Not saved: {e}"));
                Mode::Form(form)
            }
        }
    }

    fn log_key(
        &mut self,
        k: KeyEvent,
        title: String,
        lines: Vec<String>,
        mut scroll: usize,
    ) -> Mode {
        let last = lines.len().saturating_sub(1);
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left => return Mode::List,
            KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => scroll = (scroll + 1).min(last),
            KeyCode::PageUp => scroll = scroll.saturating_sub(20),
            KeyCode::PageDown | KeyCode::Char(' ') => scroll = (scroll + 20).min(last),
            KeyCode::Home | KeyCode::Char('g') => scroll = 0,
            KeyCode::End | KeyCode::Char('G') => scroll = last,
            _ => {}
        }
        Mode::Log {
            title,
            lines,
            scroll,
        }
    }

    // ---- drawing ------------------------------------------------------------

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let [head, body, foot] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);
        self.render_head(f, head);
        match &self.mode {
            Mode::Form(form) => self.render_form(f, body, form),
            Mode::Confirm(_, Pending::Discard(form)) => self.render_form(f, body, form),
            Mode::Log {
                title,
                lines,
                scroll,
            } => render_log(f, body, title, lines, *scroll),
            _ => match self.view {
                View::Routines => self.render_routines(f, body),
                View::Runs => self.render_runs(f, body),
            },
        }
        self.render_foot(f, foot);
    }

    fn render_head(&self, f: &mut Frame, area: Rect) {
        let tab = |label: String, on: bool| {
            if on {
                Span::styled(
                    format!(" {label} "),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {label} "), Style::default().fg(theme::MUTED))
            }
        };
        let list = matches!(self.mode, Mode::List | Mode::Confirm(..));
        let mut left = vec![
            tab(
                format!("Routines ({})", self.routines.len()),
                list && self.view == View::Routines,
            ),
            Span::styled("│", Style::default().fg(theme::BORDER)),
            tab(
                format!("Runs ({})", self.runs.len()),
                list && self.view == View::Runs,
            ),
        ];
        if let Mode::Form(form) = &self.mode {
            left = vec![tab(
                if form.id.is_some() {
                    "Edit routine".into()
                } else {
                    "New routine".into()
                },
                true,
            )];
        }
        if let Mode::Log { .. } = &self.mode {
            left = vec![tab("Run output".into(), true)];
        }
        let right = match self.scheduler {
            Some(_) => Span::styled("● Scheduler running ", Style::default().fg(theme::GREEN)),
            None => Span::styled(
                "○ Scheduler off · S starts it ",
                Style::default().fg(theme::RED),
            ),
        };
        let [l, r] = Layout::horizontal([
            Constraint::Min(10),
            Constraint::Length(right.width() as u16),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(Line::from(left)).block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme::BORDER)),
            ),
            l,
        );
        f.render_widget(
            Paragraph::new(Line::from(right)).block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme::BORDER)),
            ),
            r,
        );
    }

    fn render_routines(&self, f: &mut Frame, area: Rect) {
        if self.routines.is_empty() {
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No routines yet.",
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press n to create one: a prompt, an agent, a folder and when it runs.",
                    Style::default().fg(theme::MUTED),
                )),
                Line::from(Span::styled(
                    format!(
                        "  Routines are plain files in {}",
                        paths::shorten(&self.paths.routines_dir())
                    ),
                    Style::default().fg(theme::FAINT),
                )),
            ];
            f.render_widget(Paragraph::new(text), area);
            return;
        }
        let [top, bottom] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(9)]).areas(area);
        let now = Local::now();
        let longest =
            |f: &dyn Fn(&Routine) -> usize| self.routines.iter().map(f).max().unwrap_or(0);
        let name_w = longest(&|r| r.file.name.chars().count()).clamp(4, 40) as u16;
        let sched_w = longest(&|r| r.schedule_text().chars().count()).clamp(8, 48) as u16;
        let rows: Vec<Row> = self
            .routines
            .iter()
            .map(|r| {
                let (icon, color) = self.icons.routine(r.file.enabled, r.problem.is_some());
                let next = if r.file.enabled && r.problem.is_none() {
                    r.cron()
                        .and_then(|c| c.next_after(&now))
                        .map(|t| until(&t, &now))
                        .unwrap_or_else(|| "never".into())
                } else if r.problem.is_some() {
                    "broken".into()
                } else {
                    "off".into()
                };
                let last = match self.latest_run(&r.id) {
                    Some(run) => {
                        let (ri, rc) = self.icons.status(run.status);
                        Line::from(vec![
                            Span::styled(format!("{ri} "), Style::default().fg(rc)),
                            Span::styled(
                                format!("{} · {}", run.status.label(), ago(&run.started, &now)),
                                Style::default().fg(theme::MUTED),
                            ),
                        ])
                    }
                    None => Line::from(Span::styled("never", Style::default().fg(theme::FAINT))),
                };
                let name_style = if r.file.enabled {
                    Style::default().fg(theme::TEXT)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::default().fg(color))),
                    Cell::from(Span::styled(r.file.name.clone(), name_style)),
                    Cell::from(Span::styled(
                        r.schedule_text(),
                        Style::default().fg(theme::MUTED),
                    )),
                    Cell::from(Span::styled(
                        r.file.agent.clone(),
                        Style::default().fg(theme::MUTED),
                    )),
                    Cell::from(Span::styled(next, Style::default().fg(theme::TEXT))),
                    Cell::from(last),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(2),
                // Name and schedule as wide as their longest entry; the
                // name gives way first when the pane is narrow.
                Constraint::Max(name_w),
                Constraint::Length(sched_w),
                Constraint::Length(10),
                Constraint::Length(13),
                Constraint::Min(24),
            ],
        )
        .header(
            Row::new(["", "Name", "Schedule", "Agent", "Next", "Last run"])
                .style(Style::default().fg(theme::FAINT)),
        )
        .row_highlight_style(Style::default().bg(theme::SELECTED_BG))
        .column_spacing(2);
        let mut state = TableState::default().with_selected(Some(self.sel_routine));
        f.render_stateful_widget(table, top, &mut state);

        if let Some(r) = self.selected_routine() {
            self.render_routine_detail(f, bottom, r);
        }
    }

    fn render_routine_detail(&self, f: &mut Frame, area: Rect, r: &Routine) {
        let label = |s: &str| Span::styled(format!("{s:<10}"), Style::default().fg(theme::FAINT));
        let val = |s: String| Span::styled(s, Style::default().fg(theme::TEXT));
        let mut agent = r.file.agent.clone();
        if !r.file.args.is_empty() {
            agent.push(' ');
            agent.push_str(&crate::form::join_args(&r.file.args));
        }
        let mut lines = vec![
            Line::from(vec![
                label("Schedule"),
                val(r.schedule_text()),
                Span::styled(
                    format!("   {}", r.file.schedule),
                    Style::default().fg(theme::FAINT),
                ),
            ]),
            Line::from(vec![
                label("Agent"),
                val(agent),
                Span::styled("   in ", Style::default().fg(theme::FAINT)),
                val(r.file.folder.clone()),
            ]),
            Line::from(vec![
                label("If missed"),
                val(r.file.missed.label().into()),
                Span::styled("   time limit ", Style::default().fg(theme::FAINT)),
                val(format!("{} min", r.file.time_limit_minutes)),
            ]),
        ];
        if let Some(p) = &r.problem {
            lines.push(Line::from(vec![
                label("Problem"),
                Span::styled(p.clone(), Style::default().fg(theme::RED)),
            ]));
        }
        let prompt: Vec<&str> = r.prompt.lines().filter(|l| !l.trim().is_empty()).collect();
        for (i, l) in prompt.iter().take(4).enumerate() {
            lines.push(Line::from(vec![
                label(if i == 0 { "Prompt" } else { "" }),
                Span::styled(l.to_string(), Style::default().fg(theme::MUTED)),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme::BORDER))
                    .title(Span::styled(
                        format!(" {} ", r.file.name),
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    )),
            ),
            area,
        );
    }

    fn render_runs(&self, f: &mut Frame, area: Rect) {
        if self.runs.is_empty() {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "  No runs yet. r on a routine runs it now.",
                        Style::default().fg(theme::MUTED),
                    )),
                ]),
                area,
            );
            return;
        }
        let [top, bottom] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(4)]).areas(area);
        let now = Local::now();
        let rows: Vec<Row> = self
            .runs
            .iter()
            .map(|run| {
                let (icon, color) = self.icons.status(run.status);
                let open = if !run.tab_closed
                    && !run.pane_id.is_empty()
                    && run.status != crate::store::Status::Closed
                {
                    self.icons.tab_open()
                } else {
                    ""
                };
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::default().fg(color))),
                    Cell::from(Span::styled(
                        short_time(&run.started, &now),
                        Style::default().fg(theme::TEXT),
                    )),
                    Cell::from(Span::styled(
                        run.routine_name.clone(),
                        Style::default().fg(theme::TEXT),
                    )),
                    Cell::from(Span::styled(run.status.label(), Style::default().fg(color))),
                    Cell::from(Span::styled(
                        run.trigger.label(),
                        Style::default().fg(theme::MUTED),
                    )),
                    Cell::from(Span::styled(took(run), Style::default().fg(theme::MUTED))),
                    Cell::from(Span::styled(open, Style::default().fg(theme::ACCENT))),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(2),
                Constraint::Length(13),
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(10),
                Constraint::Length(2),
            ],
        )
        .header(
            Row::new(["", "Started", "Routine", "Status", "Trigger", "Took", ""])
                .style(Style::default().fg(theme::FAINT)),
        )
        .row_highlight_style(Style::default().bg(theme::SELECTED_BG))
        .column_spacing(2);
        let mut state = TableState::default().with_selected(Some(self.sel_run));
        f.render_stateful_widget(table, top, &mut state);

        if let Some(run) = self.runs.get(self.sel_run) {
            let mut lines = Vec::new();
            if !run.message.is_empty() {
                lines.push(Line::from(Span::styled(
                    run.message.clone(),
                    Style::default().fg(theme::TEXT),
                )));
            }
            let mut info = format!("{} in {}", run.agent, run.routine_id);
            if let Some(slot) = run.slot {
                info = format!("{info} · due {}", short_time(&slot, &now));
            }
            if !run.agent_name.is_empty() {
                info = format!("{info} · agent {}", run.agent_name);
            }
            lines.push(Line::from(Span::styled(
                info,
                Style::default().fg(theme::FAINT),
            )));
            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: true }).block(
                    Block::default()
                        .borders(Borders::TOP)
                        .border_style(Style::default().fg(theme::BORDER)),
                ),
                bottom,
            );
        }
    }

    fn render_form(&self, f: &mut Frame, area: Rect, form: &Form) {
        let fields = form.fields();
        let current = form.current();
        let mut lines = vec![Line::from("")];
        for field in &fields {
            let selected = *field == current;
            let value: String = match field {
                Field::Name => form.name.clone(),
                Field::Agent => format!("‹ {} ›", AGENTS[form.agent]),
                Field::Args => form.args.clone(),
                Field::Folder => form.folder.clone(),
                Field::Repeat => format!("‹ {} ›", form.repeat.label()),
                Field::Time => form.time.clone(),
                Field::Weekday => format!("‹ {} ›", crate::form::WEEKDAYS[form.weekday]),
                Field::Cron => form.cron.clone(),
                Field::Missed => format!("‹ {} ›", form.missed.label()),
                Field::Limit => form.limit.clone(),
                Field::Enabled => format!("‹ {} ›", if form.enabled { "Yes" } else { "No" }),
                Field::Prompt => {
                    let first = form
                        .prompt
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("");
                    if first.is_empty() {
                        "(empty, ⏎ to write it)".into()
                    } else {
                        match form.prompt.lines().count() {
                            1 => first.to_string(),
                            n => format!("{first}  ({n} lines)"),
                        }
                    }
                }
                Field::Save => String::new(),
            };
            let marker = if selected { "› " } else { "  " };
            let label_style = if selected {
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            if *field == Field::Save {
                lines.push(Line::from(""));
                let style = if selected {
                    Style::default()
                        .fg(theme::TEXT)
                        .bg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::ACCENT)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {marker}"), label_style),
                    Span::styled(" Save ", style),
                ]));
                continue;
            }
            let mut spans = vec![
                Span::styled(format!("  {marker}"), label_style),
                Span::styled(format!("{:<22}", field.label(form.repeat)), label_style),
                Span::styled(value, Style::default().fg(theme::TEXT)),
            ];
            if selected && field.is_text() {
                spans.push(Span::styled("▏", Style::default().fg(theme::ACCENT)));
            }
            let mut line = Line::from(spans);
            if selected {
                line = line.style(Style::default().bg(theme::SELECTED_BG));
            }
            lines.push(line);
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("    {}", current.hint()),
            Style::default().fg(theme::FAINT),
        )));
        let summary = match form.schedule() {
            Ok(c) => {
                let cron = Cron::parse(&c).expect("checked");
                let next = cron
                    .next_after(&Local::now())
                    .map(|t| format!(" · next {}", short_time(&t, &Local::now())))
                    .unwrap_or_default();
                Span::styled(
                    format!("    Runs: {}{next}", cron.describe()),
                    Style::default().fg(theme::GREEN),
                )
            }
            Err(e) => Span::styled(
                format!("    Schedule: {e}"),
                Style::default().fg(theme::RED),
            ),
        };
        lines.push(Line::from(summary));
        f.render_widget(Paragraph::new(lines), area);
    }

    fn render_foot(&self, f: &mut Frame, area: Rect) {
        let key = |k: &str| {
            Span::styled(
                format!(" {k} "),
                Style::default().fg(theme::ACCENT).bg(theme::BAR_BG),
            )
        };
        let text = |t: &str| {
            Span::styled(
                format!("{t}  "),
                Style::default().fg(theme::MUTED).bg(theme::BAR_BG),
            )
        };
        let line = if let Mode::Confirm(q, _) = &self.mode {
            Line::from(vec![
                Span::styled(
                    format!(" {q} "),
                    Style::default().fg(theme::YELLOW).bg(theme::BAR_BG),
                ),
                key("y"),
                text("yes"),
                key("n"),
                text("no"),
            ])
        } else if let Some((msg, warn)) = &self.flash {
            let color = if *warn { theme::RED } else { theme::GREEN };
            Line::from(Span::styled(
                format!(" {msg}"),
                Style::default().fg(color).bg(theme::BAR_BG),
            ))
        } else {
            let pairs: &[(&str, &str)] = match (&self.mode, self.view) {
                (Mode::Form(_), _) => &[
                    ("↑↓", "field"),
                    ("← →", "choose"),
                    ("⏎", "next / edit prompt"),
                    ("^S", "save"),
                    ("^U", "clear"),
                    ("esc", "cancel"),
                ],
                (Mode::Log { .. }, _) => &[
                    ("↑↓ pgup pgdn", "scroll"),
                    ("g G", "top / end"),
                    ("esc", "back"),
                ],
                (_, View::Routines) => &[
                    ("⏎", "open last run"),
                    ("r", "run now"),
                    ("space", "on/off"),
                    ("e", "edit"),
                    ("p", "prompt"),
                    ("n", "new"),
                    ("d", "delete"),
                    ("l", "output"),
                    ("→", "runs"),
                    ("q", "close"),
                ],
                (_, View::Runs) => &[
                    ("⏎", "open its tab"),
                    ("l", "output"),
                    ("←", "routines"),
                    ("q", "close"),
                ],
            };
            let mut spans = Vec::new();
            for (k, t) in pairs {
                spans.push(key(k));
                spans.push(text(t));
            }
            Line::from(spans)
        };
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::BAR_BG)),
            area,
        );
    }
}

fn render_log(f: &mut Frame, area: Rect, title: &str, lines: &[String], scroll: usize) {
    let height = area.height.saturating_sub(1) as usize;
    // Keep the scroll position as the last visible line.
    let start = (scroll + 1).saturating_sub(height);
    let visible: Vec<Line> = lines
        .iter()
        .skip(start)
        .take(height)
        .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme::TEXT))))
        .collect();
    f.render_widget(
        Paragraph::new(visible).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme::BORDER))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
}
