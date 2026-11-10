//! The form to create or change a routine.

use crate::agent;
use crate::routine::{Missed, Routine, RoutineFile, AGENTS, SHELL};
use crate::schedule::{Schedule, AT_START};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
    Once,
    AtStart,
    Custom,
}

impl Repeat {
    const ALL: [Repeat; 7] = [
        Repeat::Hourly,
        Repeat::Daily,
        Repeat::Weekdays,
        Repeat::Weekly,
        Repeat::Once,
        Repeat::AtStart,
        Repeat::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Hourly => "Every hour",
            Repeat::Daily => "Every day",
            Repeat::Weekdays => "Weekdays",
            Repeat::Weekly => "Once a week",
            Repeat::Once => "Once",
            Repeat::AtStart => "When Herdr starts",
            Repeat::Custom => "Custom (cron)",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Repeat::Hourly => "At a minute of every hour",
            Repeat::Daily => "At a time every day",
            Repeat::Weekdays => "At a time Monday to Friday",
            Repeat::Weekly => "At a time on one day of the week",
            Repeat::Once => "On one date at a time, then it switches itself off",
            Repeat::AtStart => "Each time the Herdr server starts, such as after a restart",
            Repeat::Custom => "Any cron schedule, such as 15 8-18 * * 1-5",
        }
    }
}

/// The retry choices: `(retries, minutes between)`.
const RETRIES: [(u32, u32); 4] = [(0, 10), (1, 5), (2, 10), (3, 15)];

fn retry_text(retries: u32, delay: u32) -> (String, String) {
    match retries {
        0 => ("Don't retry".into(), "A failed run stays failed".into()),
        1 => (
            "Try once more".into(),
            format!("{delay} minutes after it failed"),
        ),
        n => (
            format!("Try up to {n} more times"),
            format!("{delay} minutes apart"),
        ),
    }
}

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// The model picker's last entry, which asks for a typed name.
pub const OTHER_MODEL: &str = "\u{0}other";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Prompt,
    Agent,
    Mode,
    Model,
    Effort,
    Folder,
    Repeat,
    Date,
    Weekday,
    Time,
    Cron,
    Missed,
    Limit,
    AtLimit,
    Retry,
    Check,
    Remember,
    Args,
    Enabled,
    Save,
}

impl Field {
    pub fn label(self, repeat: Repeat) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Prompt => "Prompt",
            Field::Agent => "Agent",
            Field::Mode => "Permission mode",
            Field::Model => "Model",
            Field::Effort => "Effort",
            Field::Folder => "Folder",
            Field::Repeat => "Repeat",
            Field::Time if repeat == Repeat::Hourly => "Minute",
            Field::Time => "Time",
            Field::Weekday => "Day",
            Field::Cron => "Cron schedule",
            Field::Date => "Date",
            Field::Missed => "If missed",
            Field::Limit => "Time limit",
            Field::AtLimit => "At the time limit",
            Field::Retry => "If it fails",
            Field::Check => "Check first",
            Field::Remember => "Earlier results",
            Field::Args => "Extra options",
            Field::Enabled => "On",
            Field::Save => "Save",
        }
    }

    /// Typed into, as opposed to picked from a list.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Field::Name
                | Field::Args
                | Field::Folder
                | Field::Time
                | Field::Cron
                | Field::Limit
                | Field::Date
                | Field::Check
        )
    }

    /// Picked from a list (⏎ opens it, ← → steps through it).
    pub fn is_choice(self) -> bool {
        matches!(
            self,
            Field::Agent
                | Field::Mode
                | Field::Model
                | Field::Effort
                | Field::Repeat
                | Field::Weekday
                | Field::Missed
                | Field::Enabled
                | Field::AtLimit
                | Field::Retry
                | Field::Remember
        )
    }

    pub fn hint(self) -> &'static str {
        match self {
            Field::Name => {
                "The name shown in the list, such as \"Weekly dependency check\"."
            }
            Field::Prompt => {
                "What the agent should do, written as you would type it to the agent yourself. ⏎ opens the editor."
            }
            Field::Agent => "The coding agent Herdr starts for each run.",
            Field::Mode => {
                "What the agent may do without asking you. Auto: it works on its own and a safety check stops risky steps. When it still asks something, the run waits in its tab for your answer."
            }
            Field::Model => {
                "The AI model that does the work. Bigger models are smarter but slower and use more of your plan. Agent default: what the agent itself is set to. Agents that can list their models show them here (type to narrow a long list); Other…: type any model name."
            }
            Field::Effort => {
                "How long the agent thinks before it acts. More effort gives better results on hard tasks, but takes longer and uses more of your plan."
            }
            Field::Folder => {
                "The folder the run works in, such as ~/projects/site. The agent reads and changes the files there."
            }
            Field::Repeat => {
                "How often it runs: every hour, every day, on weekdays, once a week, once on a date, each time Herdr starts, or Custom for any other schedule."
            }
            Field::Date => "The day of the single run, as YYYY-MM-DD, such as 2026-12-24. The routine switches itself off after it ran.",
            Field::AtLimit => {
                "Keep running: the run is only marked and you get a notification. Stop the run: its tab is closed, which ends the agent or command; the output so far is kept."
            }
            Field::Retry => {
                "When a run fails (a command ends with an error, an agent does not start), it can be tried again a little later."
            }
            Field::Check => {
                "Optional: a shell command run first in the folder. If it ends without an error, the agent starts and gets its output; otherwise the run is recorded as \"Nothing to do\". Example: git fetch -q && git log --oneline HEAD..@{u}"
            }
            Field::Remember => {
                "Given to the agent: each run starts with the results of the last ten runs, so it knows what earlier runs did and found. The plugin keeps them in one short file."
            }
            Field::Time => "24-hour time, such as 08:15. For every hour: the minute, such as 15.",
            Field::Weekday => "The day of the week it runs.",
            Field::Cron => {
                "Any schedule as five values: minute hour day month weekday. 15 8-18 * * 1-5 means at a quarter past every hour from 8 to 18, Monday to Friday."
            }
            Field::Missed => {
                "When Herdr or the computer was off or asleep at the time. Run once: it runs as soon as Herdr runs again. Skip: it waits for the next time."
            }
            Field::Limit => {
                "After this many minutes the run counts as timed out and you get a notification. What happens then is set right below."
            }
            Field::Args => {
                "For experts, leave empty if unsure: options added to the agent's start command, such as --add-dir ~/other to let it work in a second folder too."
            }
            Field::Enabled => {
                "Yes: it runs on its schedule. No: the routine is kept but never runs by itself; r in the list still runs it by hand."
            }
            Field::Save => "Saves the routine (Ctrl+S anywhere in the form).",
        }
    }
}

/// An entry of a choice field's list.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub value: String,
    pub label: String,
    pub detail: String,
}

fn item(value: &str, label: &str, detail: &str) -> Item {
    Item {
        value: value.into(),
        label: label.into(),
        detail: detail.into(),
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    /// `None` for a new routine.
    pub id: Option<String>,
    pub name: String,
    pub agent: usize,
    pub mode: String,
    pub model: String,
    pub effort: String,
    pub args: String,
    pub folder: String,
    pub repeat: Repeat,
    pub time: String,
    pub weekday: usize,
    pub cron: String,
    pub missed: Missed,
    pub limit: String,
    /// The day of a single run, `YYYY-MM-DD`.
    pub date: String,
    pub stop_at_limit: bool,
    pub retries: u32,
    pub retry_delay: u32,
    pub check: String,
    pub remember: bool,
    pub enabled: bool,
    pub prompt: String,
    pub field: usize,
    pub dirty: bool,
    /// Models offered besides the agent's own suggestions: `(name, where
    /// it comes from)`, set by the list before the model list opens.
    pub more_models: Vec<(String, String)>,
}

impl Form {
    pub fn new(folder: &str) -> Form {
        Form {
            id: None,
            name: String::new(),
            agent: 0,
            mode: agent::default_mode(AGENTS[0]).into(),
            model: String::new(),
            effort: String::new(),
            args: String::new(),
            folder: folder.to_string(),
            repeat: Repeat::Weekdays,
            time: "09:00".into(),
            weekday: 0,
            cron: "0 9 * * 1-5".into(),
            missed: Missed::RunOnce,
            limit: "120".into(),
            date: (chrono::Local::now() + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string(),
            stop_at_limit: false,
            retries: 0,
            retry_delay: 10,
            check: String::new(),
            remember: false,
            enabled: true,
            prompt: String::new(),
            field: 0,
            dirty: false,
            more_models: Vec::new(),
        }
    }

    pub fn edit(r: &Routine) -> Form {
        let mut f = Form::new(&r.file.folder);
        f.id = Some(r.id.clone());
        f.name = r.file.name.clone();
        f.agent = AGENTS.iter().position(|a| *a == r.file.agent).unwrap_or(0);
        f.mode = r.file.permission_mode.clone();
        f.model = r.file.model.clone();
        f.effort = r.file.effort.clone();
        f.args = join_args(&r.file.args);
        f.cron = r.file.schedule.clone();
        match Schedule::parse(&r.file.schedule) {
            Ok(Schedule::Once(t)) => {
                f.repeat = Repeat::Once;
                f.date = t.format("%Y-%m-%d").to_string();
                f.time = t.format("%H:%M").to_string();
                f.cron = "0 9 * * 1-5".into();
            }
            Ok(Schedule::AtStart) => {
                f.repeat = Repeat::AtStart;
                f.cron = "0 9 * * 1-5".into();
            }
            _ => {
                let (repeat, time, weekday) = preset_of(&r.file.schedule);
                f.repeat = repeat;
                f.time = time;
                f.weekday = weekday;
            }
        }
        f.missed = r.file.missed;
        f.limit = r.file.time_limit_minutes.to_string();
        f.stop_at_limit = r.file.stop_at_limit;
        f.retries = r.file.retries;
        f.retry_delay = r.file.retry_delay_minutes;
        f.check = r.file.check_command.clone();
        f.remember = r.file.remember;
        f.enabled = r.file.enabled;
        f.prompt = r.prompt.clone();
        f
    }

    pub fn agent(&self) -> &'static str {
        AGENTS[self.agent]
    }

    pub fn is_shell(&self) -> bool {
        self.agent() == SHELL
    }

    /// A field's name: the prompt is the command for a shell routine.
    pub fn label(&self, f: Field) -> &'static str {
        match f {
            Field::Prompt if self.is_shell() => "Command",
            f => f.label(self.repeat),
        }
    }

    pub fn hint(&self, f: Field) -> &'static str {
        match f {
            Field::Prompt if self.is_shell() => {
                "The command or script to run in the folder, such as ./backup.sh. Several lines run one after the other. ⏎ opens the editor."
            }
            Field::Agent => {
                "Which AI does the work: Claude Code, Codex, Gemini or another agent Herdr knows. Shell command runs a command or script instead, without AI."
            }
            f => f.hint(),
        }
    }

    /// The fields shown for the current agent and repeat choice.
    pub fn fields(&self) -> Vec<Field> {
        let a = self.agent();
        let mut v = vec![Field::Name, Field::Agent];
        if !agent::modes(a).is_empty() {
            v.push(Field::Mode);
        }
        if !agent::models(a).is_empty() {
            v.push(Field::Model);
        }
        if !agent::efforts(a).is_empty() {
            v.push(Field::Effort);
        }
        v.extend([Field::Folder, Field::Repeat]);
        match self.repeat {
            Repeat::Custom => v.push(Field::Cron),
            Repeat::Weekly => {
                v.push(Field::Weekday);
                v.push(Field::Time);
            }
            Repeat::Once => {
                v.push(Field::Date);
                v.push(Field::Time);
            }
            Repeat::AtStart => {}
            _ => v.push(Field::Time),
        }
        if self.repeat != Repeat::AtStart {
            v.push(Field::Missed);
        }
        v.extend([Field::Limit, Field::AtLimit, Field::Retry]);
        if !self.is_shell() {
            v.extend([Field::Check, Field::Remember, Field::Args]);
        }
        // The prompt last, right above the save button: it is the longest
        // field and the one people write after choosing everything else.
        v.extend([Field::Enabled, Field::Prompt, Field::Save]);
        v
    }

    pub fn current(&self) -> Field {
        let fields = self.fields();
        fields[self.field.min(fields.len() - 1)]
    }

    pub fn move_by(&mut self, delta: i32) {
        let n = self.fields().len() as i32;
        self.field = (self.field as i32 + delta).clamp(0, n - 1) as usize;
    }

    pub fn select(&mut self, f: Field) {
        if let Some(i) = self.fields().iter().position(|x| *x == f) {
            self.field = i;
        }
    }

    fn text_mut(&mut self, f: Field) -> Option<&mut String> {
        match f {
            Field::Name => Some(&mut self.name),
            Field::Args => Some(&mut self.args),
            Field::Folder => Some(&mut self.folder),
            Field::Time => Some(&mut self.time),
            Field::Cron => Some(&mut self.cron),
            Field::Limit => Some(&mut self.limit),
            Field::Date => Some(&mut self.date),
            Field::Check => Some(&mut self.check),
            _ => None,
        }
    }

    pub fn type_char(&mut self, c: char) {
        let f = self.current();
        if let Some(t) = self.text_mut(f) {
            t.push(c);
            self.dirty = true;
        }
    }

    pub fn paste(&mut self, text: &str) {
        let f = self.current();
        if let Some(t) = self.text_mut(f) {
            t.push_str(text);
            self.dirty = true;
        }
    }

    pub fn backspace(&mut self) {
        let f = self.current();
        if let Some(t) = self.text_mut(f) {
            t.pop();
            self.dirty = true;
        }
    }

    pub fn clear(&mut self) {
        let f = self.current();
        if let Some(t) = self.text_mut(f) {
            t.clear();
            self.dirty = true;
        }
    }

    /// The value a choice field has now.
    pub fn value(&self, f: Field) -> String {
        match f {
            Field::Agent => self.agent().into(),
            Field::Mode => self.mode.clone(),
            Field::Model => self.model.clone(),
            Field::Effort => self.effort.clone(),
            Field::Repeat => self.repeat.label().into(),
            Field::Weekday => WEEKDAYS[self.weekday].into(),
            Field::Missed => self.missed.label().into(),
            Field::Enabled => if self.enabled { "Yes" } else { "No" }.into(),
            Field::AtLimit => if self.stop_at_limit { "stop" } else { "keep" }.into(),
            Field::Retry => format!("{}:{}", self.retries, self.retry_delay),
            Field::Remember => if self.remember { "Yes" } else { "No" }.into(),
            _ => String::new(),
        }
    }

    /// How a choice field's value shows.
    pub fn shown(&self, f: Field) -> String {
        let a = self.agent();
        match f {
            Field::Agent if self.is_shell() => "Shell command".into(),
            Field::Mode => agent::label(agent::modes(a), &self.mode),
            Field::Model => agent::label(agent::models(a), &self.model),
            Field::Effort => agent::label(agent::efforts(a), &self.effort),
            Field::AtLimit | Field::Retry | Field::Remember => {
                let v = self.value(f);
                self.items(f)
                    .into_iter()
                    .find(|i| i.value == v)
                    .map(|i| i.label)
                    .unwrap_or(v)
            }
            _ => self.value(f),
        }
    }

    /// The line under a choice's value: what it means.
    pub fn detail(&self, f: Field) -> String {
        let v = self.value(f);
        self.items(f)
            .into_iter()
            .find(|i| i.value == v)
            .map(|i| i.detail)
            .unwrap_or_default()
    }

    /// A choice field's list.
    pub fn items(&self, f: Field) -> Vec<Item> {
        let a = self.agent();
        let from = |list: &[agent::Choice]| -> Vec<Item> {
            list.iter()
                .map(|c| item(c.value, c.label, c.detail))
                .collect()
        };
        match f {
            Field::Agent => AGENTS
                .iter()
                .map(|a| match *a {
                    SHELL => item(a, "Shell command", "A command or script, no AI"),
                    "claude" => item(a, a, "Claude Code"),
                    _ => item(a, a, ""),
                })
                .collect(),
            Field::Mode => from(agent::modes(a)),
            Field::Model => {
                let mut v = from(agent::models(a));
                for (name, source) in &self.more_models {
                    if !v.iter().any(|i| i.value == *name) {
                        v.push(item(name, name, source));
                    }
                }
                if !self.model.is_empty() && !v.iter().any(|i| i.value == self.model) {
                    v.push(item(&self.model, &self.model, "Typed in"));
                }
                v.push(item(OTHER_MODEL, "Other…", "Type any model name"));
                v
            }
            Field::Effort => from(agent::efforts(a)),
            Field::Repeat => Repeat::ALL
                .iter()
                .map(|r| item(r.label(), r.label(), r.detail()))
                .collect(),
            Field::Weekday => WEEKDAYS.iter().map(|d| item(d, d, "")).collect(),
            Field::Missed => [Missed::RunOnce, Missed::Skip]
                .iter()
                .map(|m| item(m.label(), m.label(), m.detail()))
                .collect(),
            Field::Enabled => vec![
                item("Yes", "Yes", "Runs on its schedule"),
                item("No", "No", "Kept, but never runs"),
            ],
            Field::AtLimit => vec![
                item("keep", "Keep running", "Only marked; the tab stays open"),
                item("stop", "Stop the run", "Its tab is closed, which ends it"),
            ],
            Field::Retry => {
                let mut v: Vec<Item> = RETRIES
                    .iter()
                    .map(|(r, d)| {
                        let (label, detail) = retry_text(*r, *d);
                        item(&format!("{r}:{d}"), &label, &detail)
                    })
                    .collect();
                // A value set by hand in routine.toml stays choosable.
                let now = self.value(Field::Retry);
                if !v.iter().any(|i| i.value == now) {
                    let (label, detail) = retry_text(self.retries, self.retry_delay);
                    v.push(item(&now, &label, &detail));
                }
                v
            }
            Field::Remember => vec![
                item("No", "Not used", "Each run starts fresh"),
                item(
                    "Yes",
                    "Given to the agent",
                    "Each run gets the results of the last ten runs",
                ),
            ],
            _ => Vec::new(),
        }
    }

    /// Sets a choice field to a value from its list (or a typed model).
    pub fn choose(&mut self, f: Field, value: &str) {
        if value == OTHER_MODEL {
            return;
        }
        match f {
            Field::Agent => {
                let Some(i) = AGENTS.iter().position(|a| *a == value) else {
                    return;
                };
                if i == self.agent {
                    return;
                }
                self.agent = i;
                let a = self.agent();
                if agent::find(agent::modes(a), &self.mode).is_none() || self.mode.is_empty() {
                    self.mode = agent::default_mode(a).into();
                }
                // Model names belong to one agent.
                self.model.clear();
                if self.is_shell() {
                    // Agent flags mean nothing to a shell command.
                    self.args.clear();
                }
                if agent::find(agent::efforts(a), &self.effort).is_none() {
                    self.effort.clear();
                }
                // The list of fields changed: stay on the agent.
                self.select(Field::Agent);
            }
            Field::Mode => self.mode = value.into(),
            Field::Model => self.model = value.trim().into(),
            Field::Effort => self.effort = value.into(),
            Field::Repeat => {
                // Switching to custom starts from the schedule so far.
                if let Ok(c) = self.schedule() {
                    if !c.starts_with('@') {
                        self.cron = c;
                    }
                }
                if let Some(r) = Repeat::ALL.iter().find(|r| r.label() == value) {
                    self.repeat = *r;
                }
                self.select(Field::Repeat);
            }
            Field::Weekday => {
                if let Some(i) = WEEKDAYS.iter().position(|d| *d == value) {
                    self.weekday = i;
                }
            }
            Field::Missed => {
                self.missed = if value == Missed::Skip.label() {
                    Missed::Skip
                } else {
                    Missed::RunOnce
                }
            }
            Field::Enabled => self.enabled = value == "Yes",
            Field::AtLimit => self.stop_at_limit = value == "stop",
            Field::Retry => {
                if let Some((r, d)) = value.split_once(':') {
                    self.retries = r.parse().unwrap_or(0);
                    self.retry_delay = d.parse().unwrap_or(10);
                }
            }
            Field::Remember => self.remember = value == "Yes",
            _ => return,
        }
        self.dirty = true;
    }

    /// ← → on a choice field: the next or previous entry of its list.
    pub fn cycle(&mut self, delta: i32) {
        let f = self.current();
        let items: Vec<Item> = self
            .items(f)
            .into_iter()
            .filter(|i| i.value != OTHER_MODEL)
            .collect();
        if items.is_empty() {
            return;
        }
        let now = self.value(f);
        let i = items.iter().position(|i| i.value == now).unwrap_or(0) as i32;
        let n = items.len() as i32;
        let next = items[(i + delta).rem_euclid(n) as usize].value.clone();
        self.choose(f, &next);
    }

    /// The cron text the choices stand for.
    pub fn schedule(&self) -> Result<String, String> {
        let text = match self.repeat {
            Repeat::AtStart => AT_START.to_string(),
            Repeat::Once => {
                let (h, m) = parse_time(&self.time)?;
                let text = Schedule::once_text(&self.date, &format!("{h:02}:{m:02}"));
                Schedule::parse(&text).map_err(|_| "the date is YYYY-MM-DD, such as 2026-12-24")?;
                return Ok(text);
            }
            Repeat::Custom => self.cron.trim().to_string(),
            Repeat::Hourly => {
                let m = self.time.trim().trim_start_matches(':');
                let m: u32 = m
                    .parse()
                    .ok()
                    .filter(|m| *m < 60)
                    .ok_or("the minute is a number from 0 to 59")?;
                format!("{m} * * * *")
            }
            r => {
                let (h, m) = parse_time(&self.time)?;
                let day = match r {
                    Repeat::Weekdays => "1-5".to_string(),
                    Repeat::Weekly => ((self.weekday + 1) % 7).to_string(),
                    _ => "*".to_string(),
                };
                format!("{m} {h} * * {day}")
            }
        };
        Schedule::parse(&text)?;
        Ok(text)
    }

    /// The routine as it would be saved, or why it cannot be.
    pub fn build(&self) -> Result<(RoutineFile, String), String> {
        let schedule = self.schedule()?;
        let time_limit_minutes: u32 = self
            .limit
            .trim()
            .parse()
            .map_err(|_| "the time limit is a number of minutes")?;
        let file = RoutineFile {
            name: self.name.trim().to_string(),
            enabled: self.enabled,
            schedule,
            agent: self.agent().to_string(),
            permission_mode: self.mode.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            args: split_args(&self.args)?,
            folder: self.folder.trim().to_string(),
            missed: self.missed,
            time_limit_minutes,
            stop_at_limit: self.stop_at_limit,
            retries: self.retries,
            retry_delay_minutes: self.retry_delay,
            check_command: if self.is_shell() {
                String::new()
            } else {
                self.check.trim().to_string()
            },
            remember: self.remember && !self.is_shell(),
        };
        if let Some(problem) = Routine::check(&file, &self.prompt) {
            return Err(problem);
        }
        Ok((file, self.prompt.clone()))
    }
}

fn parse_time(t: &str) -> Result<(u32, u32), String> {
    let err = || "the time is HH:MM, such as 08:15".to_string();
    let (h, m) = t.trim().split_once(':').ok_or_else(err)?;
    let h: u32 = h.parse().map_err(|_| err())?;
    let m: u32 = m.parse().map_err(|_| err())?;
    if h > 23 || m > 59 {
        return Err(err());
    }
    Ok((h, m))
}

/// The form choices that give this cron text, or custom.
fn preset_of(cron: &str) -> (Repeat, String, usize) {
    let p: Vec<&str> = cron.split_whitespace().collect();
    let num = |s: &str| s.parse::<u32>().ok();
    let custom = (Repeat::Custom, "09:00".to_string(), 0);
    if p.len() != 5 || p[2] != "*" || p[3] != "*" {
        return custom;
    }
    let Some(m) = num(p[0]).filter(|m| *m < 60) else {
        return custom;
    };
    if p[1] == "*" && p[4] == "*" {
        return (Repeat::Hourly, format!("{m:02}"), 0);
    }
    let Some(h) = num(p[1]).filter(|h| *h < 24) else {
        return custom;
    };
    let time = format!("{h:02}:{m:02}");
    match p[4] {
        "*" => (Repeat::Daily, time, 0),
        "1-5" => (Repeat::Weekdays, time, 0),
        d => match num(d).filter(|d| *d <= 7) {
            // Cron counts from Sunday = 0, the form from Monday = 0.
            Some(d) => (Repeat::Weekly, time, ((d + 6) % 7) as usize),
            None => custom,
        },
    }
}

/// Splits agent arguments like a shell: spaces separate, single and double
/// quotes group, a backslash takes the next character as it is (except
/// inside single quotes).
pub fn split_args(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), c) => cur.push(c),
            (_, '\\') => match chars.next() {
                Some(n) => {
                    cur.push(n);
                    has = true;
                }
                None => return Err("extra arguments: a backslash at the end".into()),
            },
            (Some('"'), '"') => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quote.is_some() {
        return Err("extra arguments: a quote is not closed".into());
    }
    if has {
        out.push(cur);
    }
    Ok(out)
}

/// The arguments as one line that [`split_args`] turns back into the same
/// list.
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            let plain = !a.is_empty()
                && !a
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
            if plain {
                a.clone()
            } else {
                let escaped = a.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::Cron;

    #[test]
    fn presets_round_trip() {
        for cron in [
            "15 * * * *",
            "30 7 * * *",
            "15 8 * * 1-5",
            "0 9 * * 1",
            "0 9 * * 0",
            "15 8-18 * * 1-5",
        ] {
            let (repeat, time, weekday) = preset_of(cron);
            let mut f = Form::new("~");
            f.repeat = repeat;
            f.time = time;
            f.weekday = weekday;
            f.cron = cron.into();
            let back = f.schedule().unwrap();
            assert_eq!(
                Cron::parse(&back).unwrap(),
                Cron::parse(cron).unwrap(),
                "{cron}"
            );
        }
        assert_eq!(preset_of("15 8-18 * * 1-5").0, Repeat::Custom);
        assert_eq!(preset_of("0 9 * * 0").2, 6);
    }

    #[test]
    fn rejects_bad_input() {
        let mut f = Form::new("~");
        f.time = "25:00".into();
        assert!(f.schedule().is_err());
        f.repeat = Repeat::Hourly;
        f.time = ":75".into();
        assert!(f.schedule().is_err());
        f.time = ":05".into();
        assert_eq!(f.schedule().unwrap(), "5 * * * *");
        f.name = "Report".into();
        f.prompt = "Do it".into();
        f.limit = "abc".into();
        assert!(f.build().is_err());
        f.limit = "30".into();
        assert!(f.build().is_ok());
    }

    #[test]
    fn agent_choices_follow_the_agent() {
        let mut f = Form::new("~");
        assert_eq!(f.mode, "auto");
        assert!(f.fields().contains(&Field::Effort));
        f.choose(Field::Model, "opus");
        f.choose(Field::Effort, "high");
        f.choose(Field::Agent, "amp");
        assert_eq!(
            (f.mode.as_str(), f.model.as_str(), f.effort.as_str()),
            ("", "", "")
        );
        assert!(!f.fields().contains(&Field::Mode));
        assert_eq!(f.current(), Field::Agent);
        f.choose(Field::Agent, "codex");
        assert_eq!(f.mode, "auto");
        assert!(f.fields().contains(&Field::Model));
        f.choose(Field::Model, "some-model");
        assert!(f
            .items(Field::Model)
            .iter()
            .any(|i| i.value == "some-model"));
        f.name = "R".into();
        f.prompt = "go".into();
        let (file, _) = f.build().unwrap();
        assert_eq!(file.model, "some-model");
    }

    #[test]
    fn shell_commands_have_their_own_fields() {
        let mut f = Form::new("~");
        f.args = "--verbose".into();
        f.choose(Field::Agent, SHELL);
        assert!(f.is_shell());
        assert_eq!(f.label(Field::Prompt), "Command");
        let fields = f.fields();
        for gone in [Field::Mode, Field::Model, Field::Effort, Field::Args] {
            assert!(!fields.contains(&gone), "{gone:?}");
        }
        f.name = "Backup".into();
        f.prompt = "./backup.sh".into();
        let (file, text) = f.build().unwrap();
        assert_eq!(file.agent, "shell");
        assert!(file.args.is_empty() && file.permission_mode.is_empty());
        assert_eq!(text, "./backup.sh");
    }

    #[test]
    fn cycling_steps_through_the_list() {
        let mut f = Form::new("~");
        f.select(Field::Mode);
        f.cycle(1);
        assert_eq!(f.mode, "acceptEdits");
        f.cycle(-2);
        assert_eq!(f.mode, "");
        f.select(Field::Model);
        f.cycle(-1);
        assert_eq!(f.model, "haiku", "Other… is skipped");
    }

    #[test]
    fn args_split_and_join() {
        assert_eq!(split_args("--model opus").unwrap(), vec!["--model", "opus"]);
        assert_eq!(
            split_args("--append-system-prompt \"be brief\" -x").unwrap(),
            vec!["--append-system-prompt", "be brief", "-x"]
        );
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
        assert!(split_args("\"open").is_err());
        let a: Vec<String> = [
            "--x",
            "two words",
            "Use \"plain\" words",
            "don't",
            "a\\b",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(split_args(&join_args(&a)).unwrap(), a);
        assert_eq!(split_args("it\\'s").unwrap(), vec!["it's"]);
    }

    #[test]
    fn single_runs_retries_checks_and_memory_round_trip() {
        let mut f = Form::new("/tmp");
        f.name = "Year end".into();
        f.prompt = "Sum up the year.".into();
        f.choose(Field::Repeat, Repeat::Once.label());
        assert!(f.fields().contains(&Field::Date));
        f.date = "2026-12-24".into();
        f.time = "08:15".into();
        f.choose(Field::Retry, "2:10");
        f.choose(Field::AtLimit, "stop");
        f.choose(Field::Remember, "Yes");
        f.check = "test -f new.txt".into();
        let (file, _) = f.build().unwrap();
        assert_eq!(file.schedule, "@once 2026-12-24 08:15");
        assert_eq!((file.retries, file.retry_delay_minutes), (2, 10));
        assert!(file.stop_at_limit && file.remember);
        assert_eq!(file.check_command, "test -f new.txt");
        let r = Routine {
            id: "year-end".into(),
            file,
            prompt: "Sum up the year.".into(),
            problem: None,
            unreadable: false,
            approved: true,
        };
        let back = Form::edit(&r);
        assert_eq!(back.repeat, Repeat::Once);
        assert_eq!(
            (back.date.as_str(), back.time.as_str()),
            ("2026-12-24", "08:15")
        );
        assert_eq!(back.shown(Field::Retry), "Try up to 2 more times");
        assert_eq!(back.shown(Field::AtLimit), "Stop the run");
        // "When Herdr starts" needs no time and no missed setting.
        f.choose(Field::Repeat, Repeat::AtStart.label());
        assert!(!f.fields().contains(&Field::Time));
        assert!(!f.fields().contains(&Field::Missed));
        assert_eq!(f.build().unwrap().0.schedule, "@start");
        // Shell commands have no check and no memory.
        f.choose(Field::Agent, SHELL);
        assert!(!f.fields().contains(&Field::Check));
        assert!(!f.build().unwrap().0.remember);
        assert!(f.build().unwrap().0.check_command.is_empty());
    }
}
