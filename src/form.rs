//! The form to create or change a routine.

use crate::agent;
use crate::cron::Cron;
use crate::routine::{Missed, Routine, RoutineFile, AGENTS, SHELL};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
    Custom,
}

impl Repeat {
    const ALL: [Repeat; 5] = [
        Repeat::Hourly,
        Repeat::Daily,
        Repeat::Weekdays,
        Repeat::Weekly,
        Repeat::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Hourly => "Every hour",
            Repeat::Daily => "Every day",
            Repeat::Weekdays => "Weekdays",
            Repeat::Weekly => "Once a week",
            Repeat::Custom => "Custom (cron)",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Repeat::Hourly => "At a minute of every hour",
            Repeat::Daily => "At a time every day",
            Repeat::Weekdays => "At a time Monday to Friday",
            Repeat::Weekly => "At a time on one day of the week",
            Repeat::Custom => "Any cron schedule, such as 15 8-18 * * 1-5",
        }
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
    Weekday,
    Time,
    Cron,
    Missed,
    Limit,
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
            Field::Missed => "If missed",
            Field::Limit => "Time limit",
            Field::Args => "Extra arguments",
            Field::Enabled => "On",
            Field::Save => "Save",
        }
    }

    /// Typed into, as opposed to picked from a list.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Field::Name | Field::Args | Field::Folder | Field::Time | Field::Cron | Field::Limit
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
        )
    }

    pub fn hint(self) -> &'static str {
        match self {
            Field::Name => "What the routine does, as shown in the list.",
            Field::Prompt => "What the agent is asked to do. ⏎ opens the editor.",
            Field::Agent => "The coding agent Herdr starts for each run.",
            Field::Mode => {
                "What the agent may do without asking. A run waits in its tab when the agent asks something."
            }
            Field::Model => "The model the agent uses. Other… takes any model name.",
            Field::Effort => "How hard the agent thinks before it answers.",
            Field::Folder => "The folder the agent starts in, such as ~/projects/site.",
            Field::Repeat => "How often it runs.",
            Field::Time => "24-hour time, such as 08:15. For every hour: the minute, such as 15.",
            Field::Weekday => "The day of the week it runs.",
            Field::Cron => "minute hour day month weekday, such as 15 8-18 * * 1-5.",
            Field::Missed => {
                "When Herdr or the computer was off at the time: run once afterwards, or skip it."
            }
            Field::Limit => {
                "Minutes until a run counts as timed out. Its tab stays open; the agent is not stopped."
            }
            Field::Args => {
                "More arguments for the agent's command line, for experts. Usually empty."
            }
            Field::Enabled => "Off keeps the routine but never runs it.",
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
    pub enabled: bool,
    pub prompt: String,
    pub field: usize,
    pub dirty: bool,
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
            enabled: true,
            prompt: String::new(),
            field: 0,
            dirty: false,
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
        let (repeat, time, weekday) = preset_of(&r.file.schedule);
        f.repeat = repeat;
        f.time = time;
        f.weekday = weekday;
        f.missed = r.file.missed;
        f.limit = r.file.time_limit_minutes.to_string();
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
            Field::Agent => "A coding agent Herdr starts with the prompt, or a shell command without AI.",
            f => f.hint(),
        }
    }

    /// The fields shown for the current agent and repeat choice.
    pub fn fields(&self) -> Vec<Field> {
        let a = self.agent();
        let mut v = vec![Field::Name, Field::Prompt, Field::Agent];
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
            _ => v.push(Field::Time),
        }
        v.extend([Field::Missed, Field::Limit]);
        if !self.is_shell() {
            v.push(Field::Args);
        }
        v.extend([Field::Enabled, Field::Save]);
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
                    self.cron = c;
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
        Cron::parse(&text)?;
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
}
