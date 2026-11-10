//! The form to create or change a routine.

use crate::cron::Cron;
use crate::routine::{Missed, Routine, RoutineFile, AGENTS};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Agent,
    Args,
    Folder,
    Repeat,
    Time,
    Weekday,
    Cron,
    Missed,
    Limit,
    Enabled,
    Prompt,
    Save,
}

impl Field {
    pub fn label(self, repeat: Repeat) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Agent => "Agent",
            Field::Args => "Agent arguments",
            Field::Folder => "Folder",
            Field::Repeat => "Repeat",
            Field::Time if repeat == Repeat::Hourly => "Minute",
            Field::Time => "Time",
            Field::Weekday => "Day",
            Field::Cron => "Cron schedule",
            Field::Missed => "If missed",
            Field::Limit => "Time limit (minutes)",
            Field::Enabled => "On",
            Field::Prompt => "Prompt",
            Field::Save => "Save",
        }
    }

    /// Typed into, as opposed to picked with ← →.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Field::Name | Field::Args | Field::Folder | Field::Time | Field::Cron | Field::Limit
        )
    }

    pub fn hint(self) -> &'static str {
        match self {
            Field::Name => "What the routine does, as shown in the list",
            Field::Agent => "← → picks the coding agent Herdr starts",
            Field::Args => "Passed to the agent, such as --model opus (optional)",
            Field::Folder => "The folder the agent starts in",
            Field::Repeat => "← → picks how often it runs",
            Field::Time => "24-hour time, such as 08:15 (for every hour: the minute, such as 15)",
            Field::Weekday => "← → picks the day",
            Field::Cron => "minute hour day month weekday, such as 15 8-18 * * 1-5",
            Field::Missed => {
                "When Herdr or the computer was off at the time: run once afterwards, or skip"
            }
            Field::Limit => "After this the run counts as timed out; its tab stays open",
            Field::Enabled => "Space or ← → switches it on or off",
            Field::Prompt => "⏎ opens the prompt in your editor ($VISUAL, $EDITOR or nano)",
            Field::Save => "⏎ saves the routine (or Ctrl+S anywhere)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    /// `None` for a new routine.
    pub id: Option<String>,
    pub name: String,
    pub agent: usize,
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

    /// The fields shown for the current repeat choice.
    pub fn fields(&self) -> Vec<Field> {
        let mut v = vec![
            Field::Name,
            Field::Agent,
            Field::Args,
            Field::Folder,
            Field::Repeat,
        ];
        match self.repeat {
            Repeat::Custom => v.push(Field::Cron),
            Repeat::Weekly => {
                v.push(Field::Weekday);
                v.push(Field::Time);
            }
            _ => v.push(Field::Time),
        }
        v.extend([
            Field::Missed,
            Field::Limit,
            Field::Enabled,
            Field::Prompt,
            Field::Save,
        ]);
        v
    }

    pub fn current(&self) -> Field {
        let fields = self.fields();
        fields[self.field.min(fields.len() - 1)]
    }

    pub fn move_by(&mut self, delta: i32) {
        let n = self.fields().len() as i32;
        self.field = ((self.field as i32 + delta).rem_euclid(n)) as usize;
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
        } else if c == ' ' {
            self.cycle(1);
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

    /// ← → on a choice field.
    pub fn cycle(&mut self, delta: i32) {
        let step = |i: usize, n: usize| ((i as i32 + delta).rem_euclid(n as i32)) as usize;
        match self.current() {
            Field::Agent => self.agent = step(self.agent, AGENTS.len()),
            Field::Repeat => {
                // Switching to custom starts from the schedule so far.
                if let Ok(c) = self.schedule() {
                    self.cron = c;
                }
                let i = Repeat::ALL
                    .iter()
                    .position(|r| *r == self.repeat)
                    .unwrap_or(0);
                self.repeat = Repeat::ALL[step(i, Repeat::ALL.len())];
            }
            Field::Weekday => self.weekday = step(self.weekday, 7),
            Field::Missed => self.missed = self.missed.toggled(),
            Field::Enabled => self.enabled = !self.enabled,
            _ => return,
        }
        self.dirty = true;
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
            agent: AGENTS[self.agent].to_string(),
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
                None => return Err("agent arguments: a backslash at the end".into()),
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
        return Err("agent arguments: a quote is not closed".into());
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
