//! The form to create or change a routine.

use crate::agent;
use crate::routine::{
    Alert, Missed, Routine, RoutineFile, RunsIn, AGENTS, KEEP_DAYS, KEEP_RUNS, SHELL,
};
use crate::schedule::{Every, Schedule, AT_START};
use chrono::{NaiveDate, NaiveTime};

/// How often a routine runs, as the form offers it. Each one stands for
/// a schedule text (`Form::schedule`); anything else is `Custom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// Every N minutes.
    Minutes,
    /// Every hour at a minute.
    Hourly,
    /// Every N hours at a minute.
    Hours,
    Daily,
    Weekdays,
    /// On chosen days of the week.
    Days,
    Weekly,
    /// Every second week, counted from a first date.
    TwoWeeks,
    /// Every month: on a day, the last (working) day or a numbered weekday.
    Monthly,
    Once,
    AtStart,
    Custom,
}

impl Repeat {
    const ALL: [Repeat; 12] = [
        Repeat::Minutes,
        Repeat::Hourly,
        Repeat::Hours,
        Repeat::Daily,
        Repeat::Weekdays,
        Repeat::Days,
        Repeat::Weekly,
        Repeat::TwoWeeks,
        Repeat::Monthly,
        Repeat::Once,
        Repeat::AtStart,
        Repeat::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Minutes => "Every few minutes",
            Repeat::Hourly => "Every hour",
            Repeat::Hours => "Every few hours",
            Repeat::Daily => "Every day",
            Repeat::Weekdays => "Weekdays",
            Repeat::Days => "Chosen weekdays",
            Repeat::Weekly => "Every week",
            Repeat::TwoWeeks => "Every 2 weeks",
            Repeat::Monthly => "Every month",
            Repeat::Once => "Once",
            Repeat::AtStart => "When Herdr starts",
            Repeat::Custom => "Custom (cron)",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Repeat::Minutes => "Every 5, 10, 15, 20 or 30 minutes",
            Repeat::Hourly => "At a minute of every hour",
            Repeat::Hours => "Every 2, 3, 4, 6, 8 or 12 hours",
            Repeat::Daily => "At a time every day",
            Repeat::Weekdays => "At a time Monday to Friday",
            Repeat::Days => "At a time on the days you switch on",
            Repeat::Weekly => "At a time on one day of the week",
            Repeat::TwoWeeks => "Every second week, counted from a first date",
            Repeat::Monthly => "On a day, the last day or the first Monday of the month",
            Repeat::Once => "On one date at a time, then it switches itself off",
            Repeat::AtStart => "Each time the Herdr server starts, such as after a restart",
            Repeat::Custom => "Any cron schedule, such as 15 8-18 * * 1-5",
        }
    }

    /// Whether the system's cron can run it (a cron job's choices).
    fn classic(self) -> bool {
        self != Repeat::TwoWeeks
    }
}

/// Which day of the month a monthly routine runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthOn {
    /// A day by its number (`Form::month_day`).
    Day,
    LastDay,
    LastWorkday,
    /// The first to fourth of a weekday (`Form::weekday`).
    Nth(u32),
    /// The last of a weekday.
    LastWeekday,
}

impl MonthOn {
    const ALL: [MonthOn; 8] = [
        MonthOn::Day,
        MonthOn::LastDay,
        MonthOn::LastWorkday,
        MonthOn::Nth(1),
        MonthOn::Nth(2),
        MonthOn::Nth(3),
        MonthOn::Nth(4),
        MonthOn::LastWeekday,
    ];

    fn value(self) -> String {
        match self {
            MonthOn::Day => "day".into(),
            MonthOn::LastDay => "last-day".into(),
            MonthOn::LastWorkday => "last-workday".into(),
            MonthOn::Nth(n) => format!("nth-{n}"),
            MonthOn::LastWeekday => "last-weekday".into(),
        }
    }

    /// In words, with the weekday the form has now.
    fn label(self, weekday: &str) -> String {
        match self {
            MonthOn::Day => "A day of the month".into(),
            MonthOn::LastDay => "The last day".into(),
            MonthOn::LastWorkday => "The last working day".into(),
            MonthOn::Nth(n) => format!(
                "The {} {weekday}",
                ["first", "second", "third", "fourth", "fifth"][(n as usize).clamp(1, 5) - 1]
            ),
            MonthOn::LastWeekday => format!("The last {weekday}"),
        }
    }

    fn detail(self) -> &'static str {
        match self {
            MonthOn::Day => "The 1st, the 15th or any other day; give its number below",
            MonthOn::LastDay => "The 28th to 31st, whichever the month ends on",
            MonthOn::LastWorkday => "The last day from Monday to Friday",
            MonthOn::Nth(_) | MonthOn::LastWeekday => "Choose the weekday below",
        }
    }

    /// Whether it names a weekday (then the form asks for one).
    fn by_weekday(self) -> bool {
        matches!(self, MonthOn::Nth(_) | MonthOn::LastWeekday)
    }

    /// Whether the system's cron can run it.
    fn classic(self) -> bool {
        self == MonthOn::Day
    }
}

/// The steps "every few minutes" and "every few hours" offer.
const EVERY_MINUTES: [u32; 5] = [5, 10, 15, 20, 30];
const EVERY_HOURS: [u32; 6] = [2, 3, 4, 6, 8, 12];
/// How late a missed time may be caught up, in hours (0: however late).
const LATE_HOURS: [u32; 6] = [0, 1, 2, 4, 8, 24];
/// After how many runs in a row that went wrong a routine is switched off
/// (0: never).
const OFF_AFTER: [u32; 4] = [0, 3, 5, 10];

pub fn late_text(hours: u32) -> (String, String) {
    match hours {
        0 => (
            Missed::RunOnce.label().into(),
            Missed::RunOnce.detail().into(),
        ),
        1 => (
            "Run once, if less than 1 hour late".into(),
            "A catch-up run only within an hour of the missed time".into(),
        ),
        n => (
            format!("Run once, if less than {n} hours late"),
            format!("A catch-up run only within {n} hours of the missed time"),
        ),
    }
}

pub fn off_after_text(runs: u32) -> (String, String) {
    match runs {
        0 => (
            "Never".into(),
            "It stays on, however often it goes wrong".into(),
        ),
        n => (
            format!("{n} runs in a row that went wrong"),
            "Then it is switched off and you are told".into(),
        ),
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

/// The choices for how many runs are kept: `(runs, days)`, 0 is no limit.
const KEEPS: [(u32, u32); 6] = [
    (KEEP_RUNS, KEEP_DAYS),
    (10, 30),
    (100, 365),
    (KEEP_RUNS, 0),
    (0, KEEP_DAYS),
    (0, 0),
];

/// How a choice of kept runs reads: short, and in words.
pub fn keep_text(runs: u32, days: u32) -> (String, String) {
    let runs_text = |n: u32| {
        if n == 1 {
            "1 run".into()
        } else {
            format!("{n} runs")
        }
    };
    let days_text = |n: u32| {
        if n == 1 {
            "1 day".into()
        } else {
            format!("{n} days")
        }
    };
    match (runs, days) {
        (0, 0) => ("All runs".into(), "Nothing is deleted by itself".into()),
        (r, 0) => (
            runs_text(r),
            format!("The last {}, however old", runs_text(r)),
        ),
        (0, d) => (
            days_text(d),
            format!("All runs of the last {}", days_text(d)),
        ),
        (r, d) => (
            format!("{}, {}", runs_text(r), days_text(d)),
            format!(
                "The last {}, none older than {}",
                runs_text(r),
                days_text(d)
            ),
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
    /// How many minutes or hours between runs.
    Every,
    /// The minute of the hour.
    Minute,
    /// The days of the week, as seven switches.
    Days,
    /// Which day of the month.
    MonthOn,
    /// The number of a day of the month.
    MonthDay,
    Date,
    Weekday,
    Time,
    Cron,
    /// What happens with a missed time, and how late a catch-up may be.
    Missed,
    Limit,
    AtLimit,
    Retry,
    /// Switch off after runs in a row that went wrong.
    OffAfter,
    Alert,
    /// The command after a run, and after which runs.
    After,
    AfterWhen,
    RunsIn,
    Keep,
    /// The Multiaccount account a Claude Code routine runs with.
    Account,
    Check,
    Remember,
    Args,
    Enabled,
    /// The row that shows or hides the settings most routines never change.
    More,
    Save,
}

impl Field {
    pub fn label(self, repeat: Repeat) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Prompt => "Prompt",
            Field::Agent => "Runs with",
            Field::Mode => "Permission mode",
            Field::Model => "Model",
            Field::Effort => "Effort",
            Field::Folder => "Folder",
            Field::Repeat => "Repeat",
            Field::Every => "Every",
            Field::Minute => "At minute",
            Field::Days | Field::MonthOn => "On",
            Field::MonthDay => "Day of the month",
            Field::Time => "Time",
            Field::Weekday => "Day",
            Field::Cron => "Cron schedule",
            Field::Date if repeat == Repeat::TwoWeeks => "First date",
            Field::Date => "Date",
            Field::OffAfter => "Switch off after",
            Field::After => "After the run",
            Field::AfterWhen => "After which runs",
            Field::Account => "Account",
            Field::Missed => "If missed",
            Field::Limit => "Time limit",
            Field::AtLimit => "At the time limit",
            Field::Retry => "If it fails",
            Field::Alert => "Alerts",
            Field::RunsIn => "Runs in",
            Field::Keep => "Run history",
            Field::Check => "Check first",
            Field::Remember => "Earlier results",
            Field::Args => "Extra options",
            Field::Enabled => "Switched",
            Field::More => "More settings",
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
                | Field::Minute
                | Field::MonthDay
                | Field::After
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
                | Field::Alert
                | Field::RunsIn
                | Field::Keep
                | Field::Remember
                | Field::Every
                | Field::MonthOn
                | Field::OffAfter
                | Field::AfterWhen
                | Field::Account
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
                "How often it runs: every few minutes or hours, every day, on weekdays or the days you choose, every week or second week, every month, once on a date, each time Herdr starts, or Custom for any other schedule. The next times show below."
            }
            Field::Every => "How long between two runs.",
            Field::Minute => "The minute of the hour it runs at, from 0 to 59, such as 15 for a quarter past.",
            Field::Days => {
                "The days of the week it runs on. ← → move, Space switches a day on or off; 1 to 7 switch Monday to Sunday."
            }
            Field::MonthOn => {
                "Which day of the month: a day by its number, the last day, the last working day (Monday to Friday), or the first, second, third, fourth or last of a weekday."
            }
            Field::MonthDay => {
                "The number of the day, from 1 to 31. In a month that is shorter, a day it does not have is left out: use \"The last day\" for the end of every month."
            }
            Field::Date => "The day of the single run, as YYYY-MM-DD, such as 2026-12-24 (24.12.2026 works too). The routine switches itself off after it ran.",
            Field::OffAfter => {
                "Switches the routine off by itself when this many runs in a row went wrong (failed, timed out, closed early or without a result), and tells you. A run that works starts the count again."
            }
            Field::After => {
                "Optional: a one-line command that runs in the routine's folder when a run has ended, for at most 5 minutes, such as ~/bin/tell-me.sh. It gets HERDR_ROUTINES_STATUS (finished, failed, timed-out, closed, no-result), _NAME, _ID, _RUN, _MESSAGE, _RESULT, _OUTPUT (the output file) and _STARTED."
            }
            Field::AfterWhen => {
                "After which runs the command above runs: only runs that went wrong (failed, timed out, closed early, without a result), or every run."
            }
            Field::Account => {
                "The Claude account the run uses, from the Multiaccount plugin. Multiaccount decides: the space's or the active account, moving on to the next when one is at its limit. An account you name here is used also when it is at its limit."
            }
            Field::AtLimit => {
                "Keep running: the run is only marked and you get a notification. Stop the run: its tab is closed, which ends the agent or command; the output so far is kept."
            }
            Field::Retry => {
                "When a run fails (a command ends with an error, an agent does not start), it can be tried again a little later."
            }
            Field::RunsIn => {
                "Herdr: Herdr starts it in a tab, as long as Herdr runs, and you can watch it live. Cron: a cron job in your computer's crontab, which also runs when Herdr is closed; its output and result still show in Runs. On macOS, a cron job that works in Documents, Desktop or Downloads needs Full Disk Access for cron."
            }
            Field::Alert => {
                "How you hear about a run. Notification: Herdr's usual notification when it fails, runs too long or needs you. Popup on problems: a window in Herdr that stays in front until you press OK. Popup always: also when a run finishes."
            }
            Field::Keep => {
                "How long its runs and their output stay in Runs. When a run ends, older ones are deleted; whichever limit comes first counts. A run that is still going is never deleted."
            }
            Field::Check => {
                "Optional: a shell command run first in the folder. If it ends without an error, the agent starts and gets its output; otherwise the run is recorded as \"Nothing to do\". Example: git fetch -q && git log --oneline HEAD..@{u}"
            }
            Field::Remember => {
                "Given to the agent: each run starts with the results of the last ten runs, so it knows what earlier runs did and found. The plugin keeps them in one short file."
            }
            Field::Time => "24-hour time, such as 08:15. Shorter works too: 9, 930, 0930 or 9.30.",
            Field::Weekday => "The day of the week it runs.",
            Field::Cron => {
                "Any schedule as five values: minute hour day month weekday. 15 8-18 * * 1-5 means at a quarter past every hour from 8 to 18, Monday to Friday. Also: L or LW as the day (the last day, the last working day), mon#1 or friL as the weekday (the first Monday, the last Friday), and @every 3 days from 2026-11-09 09:00."
            }
            Field::Missed => {
                "When Herdr or the computer was off or asleep at the time. Run once: it runs as soon as Herdr runs again, however late, or only when that is within a few hours. Skip: it waits for the next time."
            }
            Field::Limit => {
                "After this many minutes the run counts as timed out and you get a notification. What happens then is set right below."
            }
            Field::Args => {
                "For experts, leave empty if unsure: options added to the agent's start command, such as --add-dir ~/other to let it work in a second folder too."
            }
            Field::Enabled => {
                "On: it runs on its schedule. Off: the routine is kept but never runs by itself; r in the list still runs it by hand."
            }
            Field::More => {
                "Permission mode, model, missed times, time limit, retries, alerts, a command after the run, run history and more. They start with values that suit most routines; ⏎ shows or hides them."
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
    /// The time of day, for the choices that have one.
    pub time: String,
    /// The minute of the hour, for "every hour" and "every few hours".
    pub minute: String,
    pub every_minutes: u32,
    pub every_hours: u32,
    /// Monday is 0.
    pub weekday: usize,
    /// The chosen weekdays, Monday first, and the one the cursor is on.
    pub days: [bool; 7],
    pub day_cursor: usize,
    pub month_on: MonthOn,
    /// The day of the month, as typed.
    pub month_day: String,
    /// The text of "Custom".
    pub cron: String,
    /// The custom text is the owner's own (read from the routine or typed):
    /// going to "Custom" then shows it again instead of the schedule the
    /// other choices stand for.
    pub cron_own: bool,
    /// The schedule as the routine has it, kept as long as nothing under
    /// "When" was changed, so that saving does not rewrite it.
    pub loaded_schedule: Option<String>,
    pub when_dirty: bool,
    /// The accounts of the Multiaccount plugin, when it is installed; set
    /// by the list when the form opens.
    pub accounts: Option<Vec<crate::accounts::Account>>,
    pub missed: Missed,
    pub limit: String,
    /// The day of a single run, `YYYY-MM-DD`.
    pub date: String,
    pub stop_at_limit: bool,
    pub retries: u32,
    pub retry_delay: u32,
    pub alert: Alert,
    pub runs_in: RunsIn,
    pub keep_runs: u32,
    pub keep_days: u32,
    /// The routine's further settings (`routine::More`).
    pub extra: crate::routine::More,
    pub check: String,
    pub remember: bool,
    pub enabled: bool,
    pub prompt: String,
    pub field: usize,
    /// Where typing goes in the current text field, in characters.
    pub cursor: usize,
    /// "More settings" is open.
    pub more: bool,
    /// The first row the form shows, kept between frames so it only
    /// scrolls when the current field would leave the window.
    pub top: usize,
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
            minute: "00".into(),
            every_minutes: 15,
            every_hours: 2,
            weekday: 0,
            days: [true, false, true, false, true, false, false],
            day_cursor: 0,
            month_on: MonthOn::Day,
            month_day: "1".into(),
            cron: "0 9 * * 1-5".into(),
            cron_own: false,
            loaded_schedule: None,
            when_dirty: false,
            accounts: None,
            missed: Missed::RunOnce,
            limit: "120".into(),
            date: (chrono::Local::now() + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string(),
            stop_at_limit: false,
            retries: 0,
            retry_delay: 10,
            alert: Alert::default(),
            runs_in: RunsIn::default(),
            keep_runs: KEEP_RUNS,
            keep_days: KEEP_DAYS,
            extra: Default::default(),
            check: String::new(),
            remember: false,
            enabled: true,
            prompt: String::new(),
            field: 0,
            cursor: 0,
            more: false,
            top: 0,
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
        f.load_schedule(&r.file.schedule);
        f.missed = r.file.missed;
        f.limit = r.file.time_limit_minutes.to_string();
        f.stop_at_limit = r.file.stop_at_limit;
        f.retries = r.file.retries;
        f.retry_delay = r.file.retry_delay_minutes;
        f.alert = r.file.alert;
        f.runs_in = r.file.runs_in;
        f.keep_runs = r.file.keep_runs;
        f.keep_days = r.file.keep_days;
        f.extra = r.file.more.clone();
        f.check = r.file.check_command.clone();
        f.remember = r.file.remember;
        f.enabled = r.file.enabled;
        f.prompt = r.prompt.clone();
        f.cursor_end();
        f
    }

    pub fn agent(&self) -> &'static str {
        AGENTS[self.agent]
    }

    pub fn is_shell(&self) -> bool {
        self.agent() == SHELL
    }

    /// A classic cron job (only shell commands can be one).
    pub fn is_cron(&self) -> bool {
        self.is_shell() && self.runs_in == RunsIn::Cron
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
            Field::Prompt if self.is_cron() => {
                "The crontab line's command, one line, such as ~/bin/backup.sh >> ~/backup.log. It starts in your home folder; for more steps, call a script. ⏎ opens the editor."
            }
            Field::Prompt if self.is_shell() => {
                "The command or script to run in the folder, such as ./backup.sh. Several lines run one after the other. ⏎ opens the editor."
            }
            Field::Repeat if self.is_cron() => {
                "How often it runs: every few minutes or hours, every day, on weekdays or the days you choose, every week, every month on a day, once on a date, each time the computer starts, or Custom for any other schedule cron knows. The next times show below."
            }
            Field::Cron if self.is_cron() => {
                "Any schedule as five values: minute hour day month weekday. 15 8-18 * * 1-5 means at a quarter past every hour from 8 to 18, Monday to Friday."
            }
            Field::Limit if self.is_cron() => {
                "After this many minutes the command is stopped and the run counts as timed out."
            }
            Field::Agent => {
                "Which AI does the work: Claude Code, Codex, Gemini or another agent Herdr knows. Shell command runs a command or script instead, without AI."
            }
            f => f.hint(),
        }
    }

    /// The fields shown now: what the routine does, when and where it
    /// runs, and, when "More settings" is open, everything else.
    pub fn fields(&self) -> Vec<Field> {
        self.fields_with(self.more)
    }

    /// Every field this kind of routine has, also those "More settings"
    /// hides.
    pub fn all_fields(&self) -> Vec<Field> {
        self.fields_with(true)
    }

    fn fields_with(&self, more: bool) -> Vec<Field> {
        let a = self.agent();
        let cron = self.is_cron();
        let mut v = vec![Field::Name, Field::Agent, Field::Prompt, Field::Repeat];
        match self.repeat {
            Repeat::Custom => v.push(Field::Cron),
            Repeat::Minutes => v.push(Field::Every),
            Repeat::Hourly => v.push(Field::Minute),
            Repeat::Hours => v.extend([Field::Every, Field::Minute]),
            Repeat::Days => v.extend([Field::Days, Field::Time]),
            Repeat::Weekly => v.extend([Field::Weekday, Field::Time]),
            Repeat::Monthly => {
                v.push(Field::MonthOn);
                match self.month_on {
                    MonthOn::Day => v.push(Field::MonthDay),
                    m if m.by_weekday() => v.push(Field::Weekday),
                    _ => {}
                }
                v.push(Field::Time);
            }
            Repeat::Once | Repeat::TwoWeeks => v.extend([Field::Date, Field::Time]),
            Repeat::AtStart => {}
            Repeat::Daily | Repeat::Weekdays => v.push(Field::Time),
        }
        if self.is_shell() {
            v.push(Field::RunsIn);
        }
        // Cron starts its jobs in the home folder.
        if !cron {
            v.push(Field::Folder);
        }
        v.push(Field::More);
        if more {
            if !agent::modes(a).is_empty() {
                v.push(Field::Mode);
            }
            if !agent::models(a).is_empty() {
                v.push(Field::Model);
            }
            if !agent::efforts(a).is_empty() {
                v.push(Field::Effort);
            }
            if self.offers_account() {
                v.push(Field::Account);
            }
            if !self.is_shell() {
                v.extend([Field::Check, Field::Remember, Field::Args]);
            }
            v.push(Field::Enabled);
            // Cron skips missed times and always stops a run at its limit.
            if self.repeat != Repeat::AtStart && !cron {
                v.push(Field::Missed);
            }
            v.push(Field::Limit);
            if !cron {
                v.push(Field::AtLimit);
            }
            v.extend([
                Field::Retry,
                Field::OffAfter,
                Field::Alert,
                Field::After,
                Field::AfterWhen,
                Field::Keep,
            ]);
        }
        v.push(Field::Save);
        v
    }

    /// The part of the form a field is in, for the headings.
    pub fn section(&self, f: Field) -> &'static str {
        match f {
            Field::Name | Field::Agent | Field::Prompt => "Routine",
            Field::Repeat
            | Field::Every
            | Field::Minute
            | Field::Days
            | Field::MonthOn
            | Field::MonthDay
            | Field::Date
            | Field::Weekday
            | Field::Time
            | Field::Cron => "When",
            Field::RunsIn | Field::Folder => "Where",
            Field::Mode
            | Field::Model
            | Field::Effort
            | Field::Account
            | Field::Check
            | Field::Remember
            | Field::Args => "Agent settings",
            Field::Enabled | Field::Missed => "Schedule details",
            Field::Limit
            | Field::AtLimit
            | Field::Retry
            | Field::OffAfter
            | Field::Alert
            | Field::After
            | Field::AfterWhen
            | Field::Keep => "Time limit, retries, alerts and run history",
            Field::More | Field::Save => "",
        }
    }

    /// What "More settings" holds, in a few words, for its row while it is
    /// closed: the permission mode and whatever differs from the defaults.
    pub fn more_summary(&self) -> String {
        let a = self.agent();
        let mut v: Vec<String> = Vec::new();
        if !agent::modes(a).is_empty() {
            v.push(agent::label(agent::modes(a), &self.mode));
        }
        if !self.model.is_empty() {
            v.push(agent::label(agent::models(a), &self.model));
        }
        if !self.effort.is_empty() {
            v.push(format!(
                "{} effort",
                agent::label(agent::efforts(a), &self.effort).to_lowercase()
            ));
        }
        if self.offers_account() && !self.extra.account.is_empty() {
            v.push(format!("account {}", self.account_name()));
        }
        if !self.is_shell() {
            if !self.check.trim().is_empty() {
                v.push("checks first".into());
            }
            if self.remember {
                v.push("gets earlier results".into());
            }
            if !self.args.trim().is_empty() {
                v.push("extra options".into());
            }
        }
        if !self.enabled {
            v.push("switched off".into());
        }
        if self.all_fields().contains(&Field::Missed) {
            match (self.missed, self.extra.late_hours) {
                (Missed::Skip, _) => v.push("skips missed times".into()),
                (Missed::RunOnce, 0) => {}
                (Missed::RunOnce, h) => v.push(format!("catches up within {h} h")),
            }
        }
        if self.limit.trim() != "120" {
            v.push(format!("{} min limit", self.limit.trim()));
        }
        if self.stop_at_limit && !self.is_cron() {
            v.push("stops at the limit".into());
        }
        if self.retries > 0 {
            v.push(retry_text(self.retries, self.retry_delay).0.to_lowercase());
        }
        if self.extra.off_after_failures > 0 {
            v.push(format!(
                "off after {} that went wrong",
                self.extra.off_after_failures
            ));
        }
        if !self.extra.after_command.trim().is_empty() {
            v.push(
                if self.extra.after_always {
                    "a command after every run"
                } else {
                    "a command after a run that went wrong"
                }
                .into(),
            );
        }
        if self.alert != Alert::default() {
            v.push(self.alert.label().to_lowercase());
        }
        if (self.keep_runs, self.keep_days) != (KEEP_RUNS, KEEP_DAYS) {
            v.push(format!(
                "keeps {}",
                keep_text(self.keep_runs, self.keep_days).0.to_lowercase()
            ));
        }
        if v.is_empty() {
            "the usual settings".into()
        } else {
            v.join(" · ")
        }
    }

    pub fn current(&self) -> Field {
        let fields = self.fields();
        fields[self.field.min(fields.len() - 1)]
    }

    /// Whether the routine can name an account: the Multiaccount plugin is
    /// there, and it runs with Claude Code.
    pub fn offers_account(&self) -> bool {
        self.accounts.is_some() && self.agent() == "claude"
    }

    /// The named account as the list shows it.
    pub fn account_name(&self) -> String {
        let id = &self.extra.account;
        self.accounts
            .iter()
            .flatten()
            .find(|a| &a.id == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("{id} (not in Multiaccount any more)"))
    }

    /// Leaving a field shows what was typed there the usual way: a time as
    /// HH:MM, a date as YYYY-MM-DD, a minute with two digits.
    fn tidy(&mut self) {
        match self.current() {
            Field::Time => {
                if let Ok((h, m)) = parse_time(&self.time) {
                    self.time = format!("{h:02}:{m:02}");
                }
            }
            Field::Date => {
                if let Ok(d) = parse_date(&self.date) {
                    self.date = d.format("%Y-%m-%d").to_string();
                }
            }
            Field::Minute => {
                if let Ok(m) = parse_minute(&self.minute) {
                    self.minute = format!("{m:02}");
                }
            }
            _ => {}
        }
    }

    pub fn move_by(&mut self, delta: i32) {
        self.tidy();
        let n = self.fields().len() as i32;
        self.field = (self.field as i32 + delta).clamp(0, n - 1) as usize;
        self.cursor_end();
    }

    /// Goes to the field at this place in [`Form::fields`].
    pub fn go(&mut self, index: usize) {
        self.tidy();
        self.field = index.min(self.fields().len().saturating_sub(1));
        self.cursor_end();
    }

    /// Goes to a field; one that "More settings" hides opens them first.
    pub fn select(&mut self, f: Field) {
        self.tidy();
        if !self.fields().contains(&f) && self.all_fields().contains(&f) {
            self.more = true;
        }
        if let Some(i) = self.fields().iter().position(|x| *x == f) {
            self.field = i;
        }
        self.cursor_end();
    }

    /// Shows or hides the settings behind "More settings", staying on its
    /// row.
    pub fn toggle_more(&mut self) {
        self.more = !self.more;
        self.select(Field::More);
    }

    /// The text of a typed field.
    pub fn text(&self, f: Field) -> Option<&str> {
        Some(match f {
            Field::Name => &self.name,
            Field::Args => &self.args,
            Field::Folder => &self.folder,
            Field::Time => &self.time,
            Field::Cron => &self.cron,
            Field::Limit => &self.limit,
            Field::Date => &self.date,
            Field::Check => &self.check,
            Field::Minute => &self.minute,
            Field::MonthDay => &self.month_day,
            Field::After => &self.extra.after_command,
            _ => return None,
        })
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
            Field::Minute => Some(&mut self.minute),
            Field::MonthDay => Some(&mut self.month_day),
            Field::After => Some(&mut self.extra.after_command),
            _ => None,
        }
    }

    /// Something in the field was changed by hand.
    fn touched(&mut self, f: Field) {
        self.dirty = true;
        if self.section(f) == "When" {
            self.when_dirty = true;
        }
        if f == Field::Cron {
            self.cron_own = true;
        }
    }

    /// Space on the weekday switches: the day under the cursor on or off.
    pub fn toggle_day(&mut self, day: usize) {
        if let Some(on) = self.days.get_mut(day) {
            *on = !*on;
            self.day_cursor = day;
            self.touched(Field::Days);
        }
    }

    /// ← → on the weekday switches.
    pub fn day_by(&mut self, delta: i32) {
        self.day_cursor = (self.day_cursor as i32 + delta).clamp(0, 6) as usize;
    }

    /// How many characters the current text field has.
    fn text_len(&self) -> usize {
        self.text(self.current()).map_or(0, |t| t.chars().count())
    }

    /// Puts the typing cursor behind the current field's text.
    pub fn cursor_end(&mut self) {
        self.cursor = self.text_len();
    }

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the typing cursor left or right inside the text.
    pub fn cursor_by(&mut self, delta: i32) {
        let len = self.text_len() as i32;
        self.cursor = (self.cursor as i32 + delta).clamp(0, len) as usize;
    }

    pub fn type_char(&mut self, c: char) {
        self.paste(c.encode_utf8(&mut [0; 4]));
    }

    /// Puts text in at the typing cursor.
    pub fn paste(&mut self, text: &str) {
        let f = self.current();
        let at = self.cursor.min(self.text_len());
        if let Some(t) = self.text_mut(f) {
            t.insert_str(byte_at(t, at), text);
            self.cursor = at + text.chars().count();
            self.touched(f);
        }
    }

    /// Removes the character before the typing cursor.
    pub fn backspace(&mut self) {
        let at = self.cursor.min(self.text_len());
        if at == 0 {
            return;
        }
        self.cursor = at - 1;
        self.delete();
    }

    /// Removes the character at the typing cursor.
    pub fn delete(&mut self) {
        let f = self.current();
        let at = self.cursor;
        if at >= self.text_len() {
            return;
        }
        if let Some(t) = self.text_mut(f) {
            t.remove(byte_at(t, at));
            self.touched(f);
        }
    }

    pub fn clear(&mut self) {
        let f = self.current();
        if let Some(t) = self.text_mut(f) {
            t.clear();
            self.cursor = 0;
            self.touched(f);
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
            Field::Every if self.repeat == Repeat::Hours => self.every_hours.to_string(),
            Field::Every => self.every_minutes.to_string(),
            Field::MonthOn => self.month_on.value(),
            Field::Missed if self.missed == Missed::Skip => "skip".into(),
            Field::Missed => format!("once:{}", self.extra.late_hours),
            Field::OffAfter => self.extra.off_after_failures.to_string(),
            Field::AfterWhen => if self.extra.after_always {
                "always"
            } else {
                "problem"
            }
            .into(),
            Field::Account => self.extra.account.clone(),
            Field::Enabled => if self.enabled { "Yes" } else { "No" }.into(),
            Field::AtLimit => if self.stop_at_limit { "stop" } else { "keep" }.into(),
            Field::Retry => format!("{}:{}", self.retries, self.retry_delay),
            Field::Keep => format!("{}:{}", self.keep_runs, self.keep_days),
            Field::Remember => if self.remember { "Yes" } else { "No" }.into(),
            Field::Alert => self.alert.label().into(),
            Field::RunsIn => self.runs_in.label().into(),
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
            Field::AtLimit
            | Field::Retry
            | Field::Keep
            | Field::Remember
            | Field::Enabled
            | Field::Every
            | Field::MonthOn
            | Field::Missed
            | Field::OffAfter
            | Field::AfterWhen
            | Field::Account => {
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
            // A cron job gets the choices the system's cron can run.
            Field::Repeat => Repeat::ALL
                .iter()
                .filter(|r| !self.is_cron() || r.classic())
                .map(|r| item(r.label(), r.label(), r.detail()))
                .collect(),
            Field::Weekday => WEEKDAYS.iter().map(|d| item(d, d, "")).collect(),
            Field::Every => {
                let (steps, now, unit): (&[u32], u32, &str) = if self.repeat == Repeat::Hours {
                    (&EVERY_HOURS, self.every_hours, "hours")
                } else {
                    (&EVERY_MINUTES, self.every_minutes, "minutes")
                };
                let mut all: Vec<u32> = steps.to_vec();
                // A step from routine.toml stays choosable.
                if !all.contains(&now) {
                    all.push(now);
                    all.sort_unstable();
                }
                all.iter()
                    .map(|n| item(&n.to_string(), &format!("{n} {unit}"), ""))
                    .collect()
            }
            Field::MonthOn => MonthOn::ALL
                .iter()
                .filter(|m| !self.is_cron() || m.classic())
                .map(|m| item(&m.value(), &m.label(WEEKDAYS[self.weekday]), m.detail()))
                .collect(),
            Field::Missed => {
                let mut hours = LATE_HOURS.to_vec();
                // A window from routine.toml stays choosable.
                if !hours.contains(&self.extra.late_hours) {
                    hours.push(self.extra.late_hours);
                    hours.sort_unstable();
                }
                let mut v: Vec<Item> = hours
                    .iter()
                    .map(|h| {
                        let (label, detail) = late_text(*h);
                        item(&format!("once:{h}"), &label, &detail)
                    })
                    .collect();
                v.push(item("skip", Missed::Skip.label(), Missed::Skip.detail()));
                v
            }
            Field::OffAfter => {
                let mut runs = OFF_AFTER.to_vec();
                if !runs.contains(&self.extra.off_after_failures) {
                    runs.push(self.extra.off_after_failures);
                    runs.sort_unstable();
                }
                runs.iter()
                    .map(|n| {
                        let (label, detail) = off_after_text(*n);
                        item(&n.to_string(), &label, &detail)
                    })
                    .collect()
            }
            Field::AfterWhen => vec![
                item(
                    "problem",
                    "Runs that went wrong",
                    "Failed, timed out, closed early or without a result",
                ),
                item("always", "Every run", "Also runs that finished"),
            ],
            Field::Account => {
                let mut v = vec![item(
                    "",
                    "Multiaccount decides",
                    "The space's or the active account; moves on when one is at its limit",
                )];
                for a in self.accounts.iter().flatten() {
                    v.push(item(&a.id, &a.name, a.kind_text()));
                }
                // An account Multiaccount no longer has stays choosable.
                let now = &self.extra.account;
                if !v.iter().any(|i| &i.value == now) {
                    v.push(item(now, &self.account_name(), "Used as it is"));
                }
                v
            }
            Field::Alert => Alert::ALL
                .iter()
                .map(|a| item(a.label(), a.label(), a.detail()))
                .collect(),
            Field::RunsIn => RunsIn::ALL
                .iter()
                .map(|r| item(r.label(), r.label(), r.detail()))
                .collect(),
            Field::Enabled => vec![
                item("Yes", "On", "Runs on its schedule"),
                item("No", "Off", "Kept, but never runs by itself"),
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
            Field::Keep => {
                let mut v: Vec<Item> = KEEPS
                    .iter()
                    .map(|(r, d)| {
                        let (label, detail) = keep_text(*r, *d);
                        item(&format!("{r}:{d}"), &label, &detail)
                    })
                    .collect();
                // A value set by hand in routine.toml stays choosable.
                let now = self.value(Field::Keep);
                if !v.iter().any(|i| i.value == now) {
                    let (label, detail) = keep_text(self.keep_runs, self.keep_days);
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
                // An account belongs to Claude Code.
                if a != "claude" {
                    self.extra.account.clear();
                }
                // The list of fields changed: stay on the agent.
                self.select(Field::Agent);
            }
            Field::Mode => self.mode = value.into(),
            Field::Model => self.model = value.trim().into(),
            Field::Effort => self.effort = value.into(),
            Field::Repeat => {
                let Some(r) = Repeat::ALL.iter().find(|r| r.label() == value) else {
                    return;
                };
                // "Custom" starts from the schedule so far, unless its text
                // is the owner's own: that one comes back as it was.
                if *r == Repeat::Custom && !self.cron_own {
                    if let Ok(text) = self.schedule() {
                        self.cron = text;
                    }
                }
                self.repeat = *r;
                // Each choice keeps its own values; a monthly cron job has
                // only days by number.
                if self.is_cron() && !self.month_on.classic() {
                    self.month_on = MonthOn::Day;
                }
                self.when_dirty = true;
                self.select(Field::Repeat);
            }
            Field::Weekday => {
                if let Some(i) = WEEKDAYS.iter().position(|d| *d == value) {
                    self.weekday = i;
                }
            }
            Field::Every => {
                let Ok(n) = value.parse::<u32>() else { return };
                if self.repeat == Repeat::Hours {
                    self.every_hours = n;
                } else {
                    self.every_minutes = n;
                }
            }
            Field::MonthOn => {
                let Some(m) = MonthOn::ALL.iter().find(|m| m.value() == value) else {
                    return;
                };
                self.month_on = *m;
                // The fields below it changed: stay on this one.
                self.when_dirty = true;
                self.select(Field::MonthOn);
            }
            Field::Missed => match value.strip_prefix("once:") {
                Some(hours) => {
                    self.missed = Missed::RunOnce;
                    self.extra.late_hours = hours.parse().unwrap_or(0);
                }
                // The window stays for when "run once" comes back.
                None => self.missed = Missed::Skip,
            },
            Field::OffAfter => self.extra.off_after_failures = value.parse().unwrap_or(0),
            Field::AfterWhen => self.extra.after_always = value == "always",
            Field::Account => self.extra.account = value.into(),
            Field::Enabled => self.enabled = value == "Yes",
            Field::AtLimit => self.stop_at_limit = value == "stop",
            Field::Retry => {
                if let Some((r, d)) = value.split_once(':') {
                    self.retries = r.parse().unwrap_or(0);
                    self.retry_delay = d.parse().unwrap_or(10);
                }
            }
            Field::Keep => {
                if let Some((r, d)) = value.split_once(':') {
                    self.keep_runs = r.parse().unwrap_or(KEEP_RUNS);
                    self.keep_days = d.parse().unwrap_or(KEEP_DAYS);
                }
            }
            Field::Remember => self.remember = value == "Yes",
            Field::Alert => {
                if let Some(a) = Alert::ALL.iter().find(|a| a.label() == value) {
                    self.alert = *a;
                }
            }
            Field::RunsIn => {
                if let Some(r) = RunsIn::ALL.iter().find(|r| r.label() == value) {
                    self.runs_in = *r;
                }
            }
            _ => return,
        }
        self.touched(f);
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

    /// The schedule text the form stands for. A schedule that was read
    /// from the routine and not changed stays as it is written there.
    pub fn schedule(&self) -> Result<String, String> {
        if let (false, Some(text)) = (self.when_dirty, &self.loaded_schedule) {
            Schedule::parse(text)?;
            return Ok(text.clone());
        }
        let text = self.schedule_of_choices()?;
        Schedule::parse(&text)?;
        Ok(text)
    }

    fn schedule_of_choices(&self) -> Result<String, String> {
        let at = || parse_time(&self.time);
        let day = || parse_date(&self.date);
        let minute = || parse_minute(&self.minute);
        // Cron counts the weekdays from Sunday = 0, the form from Monday.
        let cron_day = |d: usize| (d + 1) % 7;
        Ok(match self.repeat {
            Repeat::AtStart => AT_START.to_string(),
            Repeat::Custom => self.cron.trim().to_string(),
            Repeat::Once => {
                let (h, m) = at()?;
                Schedule::once_text(
                    &day()?.format("%Y-%m-%d").to_string(),
                    &format!("{h:02}:{m:02}"),
                )
            }
            Repeat::TwoWeeks => {
                let (h, m) = at()?;
                let start = day()?.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default());
                Every { days: 14, start }.text()
            }
            Repeat::Minutes => {
                if !(1..=59).contains(&self.every_minutes) {
                    return Err("every 1 to 59 minutes".into());
                }
                format!("*/{} * * * *", self.every_minutes)
            }
            Repeat::Hourly => format!("{} * * * *", minute()?),
            Repeat::Hours => {
                if !(1..=23).contains(&self.every_hours) {
                    return Err("every 1 to 23 hours".into());
                }
                format!("{} */{} * * *", minute()?, self.every_hours)
            }
            Repeat::Daily | Repeat::Weekdays | Repeat::Weekly | Repeat::Days => {
                let (h, m) = at()?;
                let days = match self.repeat {
                    Repeat::Weekdays => "1-5".to_string(),
                    Repeat::Weekly => cron_day(self.weekday).to_string(),
                    Repeat::Days => {
                        let mut on: Vec<usize> =
                            (0..7).filter(|d| self.days[*d]).map(cron_day).collect();
                        if on.is_empty() {
                            return Err("switch on at least one day".into());
                        }
                        on.sort_unstable();
                        on.iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    }
                    _ => "*".to_string(),
                };
                format!("{m} {h} * * {days}")
            }
            Repeat::Monthly => {
                let (h, m) = at()?;
                let name = &WEEKDAYS[self.weekday].to_lowercase()[..3];
                match self.month_on {
                    MonthOn::Day => {
                        let d: u32 = self
                            .month_day
                            .trim()
                            .trim_end_matches('.')
                            .parse()
                            .ok()
                            .filter(|d| (1..=31).contains(d))
                            .ok_or("the day of the month is a number from 1 to 31")?;
                        format!("{m} {h} {d} * *")
                    }
                    MonthOn::LastDay => format!("{m} {h} L * *"),
                    MonthOn::LastWorkday => format!("{m} {h} LW * *"),
                    MonthOn::Nth(n) => format!("{m} {h} * * {name}#{n}"),
                    MonthOn::LastWeekday => format!("{m} {h} * * {name}L"),
                }
            }
        })
    }

    /// Sets the choices to the ones a schedule text matches, or to "Custom"
    /// with the text as it is.
    pub fn load_schedule(&mut self, text: &str) {
        self.loaded_schedule = Some(text.to_string());
        self.when_dirty = false;
        if !self.match_schedule(text.trim()) {
            self.repeat = Repeat::Custom;
            self.cron = text.to_string();
            self.cron_own = true;
        }
    }

    /// Whether one of the form's choices says exactly this schedule; sets
    /// it then.
    fn match_schedule(&mut self, text: &str) -> bool {
        let hhmm = |h: u32, m: u32| format!("{h:02}:{m:02}");
        match Schedule::parse(text) {
            Ok(Schedule::AtStart) => {
                self.repeat = Repeat::AtStart;
                return true;
            }
            Ok(Schedule::Once(t)) => {
                self.repeat = Repeat::Once;
                self.date = t.format("%Y-%m-%d").to_string();
                self.time = t.format("%H:%M").to_string();
                return true;
            }
            Ok(Schedule::Every(e)) if e.days == 14 => {
                self.repeat = Repeat::TwoWeeks;
                self.date = e.start.format("%Y-%m-%d").to_string();
                self.time = e.start.format("%H:%M").to_string();
                return true;
            }
            Ok(Schedule::Cron(_)) => {}
            _ => return false,
        }
        // Cron's own short forms read as the fields they stand for.
        let text = match text {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            t => t,
        };
        let p: Vec<&str> = text.split_whitespace().collect();
        let num = |s: &str, max: u32| s.parse::<u32>().ok().filter(|n| *n <= max);
        if p.len() != 5 || p[3] != "*" {
            return false;
        }
        let (min, hour, dom, dow) = (p[0], p[1], p[2], p[4]);
        let every_day = dom == "*" && dow == "*";
        // Every few minutes, every hour, every few hours.
        if let (Some(step), "*", true) = (min.strip_prefix("*/"), hour, every_day) {
            let Some(n) = num(step, 59).filter(|n| *n > 0) else {
                return false;
            };
            self.repeat = Repeat::Minutes;
            self.every_minutes = n;
            return true;
        }
        let Some(m) = num(min, 59) else { return false };
        if hour == "*" && every_day {
            self.repeat = Repeat::Hourly;
            self.minute = format!("{m:02}");
            return true;
        }
        if let (Some(step), true) = (hour.strip_prefix("*/"), every_day) {
            let Some(n) = num(step, 23).filter(|n| *n > 0) else {
                return false;
            };
            self.repeat = Repeat::Hours;
            self.every_hours = n;
            self.minute = format!("{m:02}");
            return true;
        }
        let Some(h) = num(hour, 23) else { return false };
        // A weekday as cron writes it (0 to 7 or a name), counted from
        // Monday as the form does.
        let weekday = |s: &str| -> Option<usize> {
            let lower = s.to_ascii_lowercase();
            let from_sunday = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
                .iter()
                .position(|d| *d == lower)
                .or_else(|| num(s, 7).map(|d| d as usize % 7))?;
            Some((from_sunday + 6) % 7)
        };
        let lower = dow.to_ascii_lowercase();
        let found = match (dom, lower.as_str()) {
            ("*", "*") => Some(Repeat::Daily),
            ("*", "1-5" | "mon-fri") => Some(Repeat::Weekdays),
            ("L" | "l", "*") => {
                self.month_on = MonthOn::LastDay;
                Some(Repeat::Monthly)
            }
            ("LW" | "lw", "*") => {
                self.month_on = MonthOn::LastWorkday;
                Some(Repeat::Monthly)
            }
            (d, "*") => num(d, 31).filter(|d| *d > 0).map(|d| {
                self.month_on = MonthOn::Day;
                self.month_day = d.to_string();
                Repeat::Monthly
            }),
            ("*", days) => {
                if let Some((day, n)) = days.split_once('#') {
                    match (weekday(day), num(n, 4).filter(|n| *n > 0)) {
                        (Some(day), Some(n)) => {
                            self.weekday = day;
                            self.month_on = MonthOn::Nth(n);
                            Some(Repeat::Monthly)
                        }
                        _ => None,
                    }
                } else if let Some(day) = days.strip_suffix('l').and_then(weekday) {
                    self.weekday = day;
                    self.month_on = MonthOn::LastWeekday;
                    Some(Repeat::Monthly)
                } else if let Some(day) = weekday(days) {
                    self.weekday = day;
                    Some(Repeat::Weekly)
                } else {
                    let all: Option<Vec<usize>> = days.split(',').map(weekday).collect();
                    all.filter(|d| d.len() > 1).map(|chosen| {
                        self.days = [false; 7];
                        for d in chosen {
                            self.days[d] = true;
                        }
                        Repeat::Days
                    })
                }
            }
            _ => None,
        };
        let Some(repeat) = found else { return false };
        self.repeat = repeat;
        self.time = hhmm(h, m);
        true
    }

    /// The next times the schedule comes to, for the form to show, or why
    /// there are none.
    pub fn upcoming(&self, count: usize) -> Result<Vec<chrono::DateTime<chrono::Local>>, String> {
        let schedule = Schedule::parse(&self.schedule()?)?;
        Ok(schedule.upcoming(&chrono::Local::now(), count))
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
            // Cron starts its jobs in the home folder.
            folder: if self.is_cron() {
                "~".to_string()
            } else {
                self.folder.trim().to_string()
            },
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
            alert: self.alert,
            runs_in: if self.is_shell() {
                self.runs_in
            } else {
                RunsIn::Herdr
            },
            keep_runs: self.keep_runs,
            keep_days: self.keep_days,
            more: self.extra.clone(),
        };
        if let Some(problem) = Routine::check(&file, &self.prompt) {
            return Err(problem);
        }
        Ok((file, self.prompt.clone()))
    }
}

/// The byte where the character with this number starts.
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

/// A time of day as people type it: `9`, `9:00`, `0900`, `930`, `9.30`.
pub fn parse_time(t: &str) -> Result<(u32, u32), String> {
    let err = || "the time is HH:MM, such as 08:15".to_string();
    let t = t.trim();
    let number = |s: &str| -> Result<u32, String> {
        if s.is_empty() || s.len() > 2 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err());
        }
        s.parse().map_err(|_| err())
    };
    let (h, m) = match t.split_once([':', '.']) {
        // "9:" is nine o'clock.
        Some((h, "")) => (number(h)?, 0),
        Some((h, m)) => (number(h)?, number(m)?),
        // Digits alone: the last two are the minutes from three on.
        None if t.len() >= 3 => {
            if t.len() > 4 || !t.is_char_boundary(t.len() - 2) {
                return Err(err());
            }
            let (h, m) = t.split_at(t.len() - 2);
            (number(h)?, number(m)?)
        }
        None => (number(t)?, 0),
    };
    if h > 23 || m > 59 {
        return Err(err());
    }
    Ok((h, m))
}

/// A date as `2026-12-24`, or the way it is written day first:
/// `24.12.2026`.
fn parse_date(d: &str) -> Result<NaiveDate, String> {
    let d = d.trim();
    ["%Y-%m-%d", "%d.%m.%Y", "%d/%m/%Y"]
        .iter()
        .find_map(|shape| NaiveDate::parse_from_str(d, shape).ok())
        .filter(|date| (2000..=2999).contains(&chrono::Datelike::year(date)))
        .ok_or_else(|| "the date is YYYY-MM-DD, such as 2026-12-24".to_string())
}

/// The minute of an hour: `5`, `05` or `:05`.
fn parse_minute(m: &str) -> Result<u32, String> {
    m.trim()
        .trim_start_matches(':')
        .parse()
        .ok()
        .filter(|m| *m < 60)
        .ok_or_else(|| "the minute is a number from 0 to 59".to_string())
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
                None => return Err("extra options: a backslash at the end".into()),
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
        return Err("extra options: a quote is not closed".into());
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

    /// The times a schedule text comes to, from a fixed moment.
    fn times(text: &str) -> Vec<chrono::DateTime<chrono::Local>> {
        use chrono::TimeZone;
        let from = chrono::Local
            .with_ymd_and_hms(2026, 11, 11, 7, 3, 0)
            .unwrap();
        Schedule::parse(text)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .upcoming(&from, 6)
    }

    #[test]
    fn a_schedule_loads_into_the_choice_it_matches_and_back() {
        for (text, repeat) in [
            ("*/15 * * * *", Repeat::Minutes),
            ("*/7 * * * *", Repeat::Minutes),
            ("15 * * * *", Repeat::Hourly),
            ("@hourly", Repeat::Hourly),
            ("30 */4 * * *", Repeat::Hours),
            ("30 7 * * *", Repeat::Daily),
            ("@daily", Repeat::Daily),
            ("15 8 * * 1-5", Repeat::Weekdays),
            ("0 9 * * mon-fri", Repeat::Weekdays),
            ("0 9 * * 1,3,5", Repeat::Days),
            ("0 9 * * sat,sun", Repeat::Days),
            ("0 9 * * 1", Repeat::Weekly),
            ("0 9 * * 0", Repeat::Weekly),
            ("0 9 * * SUN", Repeat::Weekly),
            ("@weekly", Repeat::Weekly),
            ("@every 2 weeks from 2026-11-09 09:00", Repeat::TwoWeeks),
            ("0 9 1 * *", Repeat::Monthly),
            ("@monthly", Repeat::Monthly),
            ("30 17 L * *", Repeat::Monthly),
            ("30 17 LW * *", Repeat::Monthly),
            ("0 9 * * mon#1", Repeat::Monthly),
            ("0 9 * * 5#3", Repeat::Monthly),
            ("0 9 * * friL", Repeat::Monthly),
            ("@once 2026-12-24 08:15", Repeat::Once),
            ("@start", Repeat::AtStart),
            // What no choice says stays as it is, under "Custom".
            ("15 8-18 * * 1-5", Repeat::Custom),
            ("0 9 1,15 * *", Repeat::Custom),
            ("0 9 1 1 *", Repeat::Custom),
            ("0 9 1 * 1", Repeat::Custom),
            ("0 9 * * mon#5", Repeat::Custom),
            ("0 9 L * 1", Repeat::Custom),
            ("@every 3 days from 2026-11-09 07:30", Repeat::Custom),
            ("*/15 9-17 * * *", Repeat::Custom),
            ("@yearly", Repeat::Custom),
        ] {
            let mut f = Form::new("~");
            f.load_schedule(text);
            assert_eq!(f.repeat, repeat, "{text}");
            // Untouched, the text stays as the routine has it.
            assert_eq!(f.schedule().unwrap(), text, "{text}");
            // Built from the choices, it comes to the same times.
            f.when_dirty = true;
            let back = f.schedule().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(times(&back), times(text), "{text} came back as {back}");
            assert!(
                !times(text).is_empty() || repeat == Repeat::AtStart,
                "{text}"
            );
        }
        // A schedule that cannot be read shows under "Custom", with why.
        let mut f = Form::new("~");
        f.load_schedule("every monday");
        assert_eq!(
            (f.repeat, f.cron.as_str()),
            (Repeat::Custom, "every monday")
        );
        assert!(f.schedule().is_err());
    }

    #[test]
    fn every_choice_writes_its_own_schedule() {
        let mut f = Form::new("~");
        let text = |f: &mut Form, r: Repeat| {
            f.choose(Field::Repeat, r.label());
            f.schedule().unwrap_or_else(|e| panic!("{r:?}: {e}"))
        };
        assert_eq!(text(&mut f, Repeat::Minutes), "*/15 * * * *");
        // "Every hour" has a minute of its own, not the time of day.
        assert_eq!(text(&mut f, Repeat::Hourly), "0 * * * *");
        assert_eq!(f.fields()[4], Field::Minute);
        assert!(!f.fields().contains(&Field::Time));
        f.minute = "15".into();
        assert_eq!(text(&mut f, Repeat::Hours), "15 */2 * * *");
        f.select(Field::Every);
        f.cycle(1);
        assert_eq!(f.schedule().unwrap(), "15 */3 * * *");
        assert_eq!(f.shown(Field::Every), "3 hours");
        assert_eq!(text(&mut f, Repeat::Daily), "0 9 * * *");
        assert_eq!(f.time, "09:00", "the time of day is still its own");
        assert_eq!(text(&mut f, Repeat::Weekdays), "0 9 * * 1-5");
        assert_eq!(text(&mut f, Repeat::Days), "0 9 * * 1,3,5");
        // Sunday is cron's 0, and comes first in its list.
        f.toggle_day(6);
        f.toggle_day(0);
        assert_eq!(f.schedule().unwrap(), "0 9 * * 0,3,5");
        for d in [2, 4, 6] {
            f.toggle_day(d);
        }
        assert_eq!(f.schedule().unwrap_err(), "switch on at least one day");
        f.weekday = 3;
        assert_eq!(text(&mut f, Repeat::Weekly), "0 9 * * 4");
        f.date = "2026-11-09".into();
        assert_eq!(
            text(&mut f, Repeat::TwoWeeks),
            "@every 2 weeks from 2026-11-09 09:00"
        );
        assert_eq!(f.label(Field::Date), "First date");
        assert_eq!(text(&mut f, Repeat::Monthly), "0 9 1 * *");
        f.month_day = "31".into();
        assert_eq!(f.schedule().unwrap(), "0 9 31 * *");
        f.month_day = "32".into();
        assert!(f.schedule().unwrap_err().contains("1 to 31"));
        f.choose(Field::MonthOn, &MonthOn::LastDay.value());
        assert_eq!(f.schedule().unwrap(), "0 9 L * *");
        assert!(!f.fields().contains(&Field::MonthDay));
        assert!(!f.fields().contains(&Field::Weekday));
        f.choose(Field::MonthOn, &MonthOn::LastWorkday.value());
        assert_eq!(f.schedule().unwrap(), "0 9 LW * *");
        f.choose(Field::MonthOn, &MonthOn::Nth(1).value());
        assert_eq!(f.schedule().unwrap(), "0 9 * * thu#1");
        assert_eq!(f.shown(Field::MonthOn), "The first Thursday");
        assert!(f.fields().contains(&Field::Weekday));
        f.choose(Field::Weekday, "Monday");
        f.choose(Field::MonthOn, &MonthOn::LastWeekday.value());
        assert_eq!(f.schedule().unwrap(), "0 9 * * monL");
        assert_eq!(f.current(), Field::MonthOn, "the cursor stays on its row");
        f.date = "24.12.2026".into();
        f.time = "815".into();
        assert_eq!(text(&mut f, Repeat::Once), "@once 2026-12-24 08:15");
        assert_eq!(f.label(Field::Date), "Date");
        assert_eq!(text(&mut f, Repeat::AtStart), "@start");
        // Every schedule the choices write is one the scheduler reads.
        f.days[0] = true;
        for r in Repeat::ALL {
            let s = text(&mut f, r);
            assert!(Schedule::parse(&s).is_ok(), "{r:?}: {s}");
        }
    }

    #[test]
    fn a_time_can_be_typed_the_short_way() {
        for (typed, time) in [
            ("9", (9, 0)),
            ("09", (9, 0)),
            ("9:00", (9, 0)),
            ("9:5", (9, 5)),
            ("0900", (9, 0)),
            ("930", (9, 30)),
            ("9.30", (9, 30)),
            ("17:45", (17, 45)),
            (" 2359 ", (23, 59)),
            ("0", (0, 0)),
            ("9:", (9, 0)),
        ] {
            assert_eq!(parse_time(typed), Ok(time), "{typed}");
        }
        for bad in [
            "", "24", "25:00", "9:60", "960", "12345", "9am", "nine", ":30", "9:1:2", "-1", "1 30",
        ] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
        // Leaving the field shows it as HH:MM; so do a date and a minute.
        let mut f = Form::new("~");
        f.select(Field::Time);
        f.clear();
        for c in "930".chars() {
            f.type_char(c);
        }
        assert_eq!(f.schedule().unwrap(), "30 9 * * 1-5");
        f.move_by(1);
        assert_eq!(f.time, "09:30");
        f.choose(Field::Repeat, Repeat::Once.label());
        f.select(Field::Date);
        f.clear();
        f.paste("24.12.2026");
        f.select(Field::Time);
        assert_eq!(f.date, "2026-12-24");
        f.choose(Field::Repeat, Repeat::Hourly.label());
        f.select(Field::Minute);
        f.clear();
        f.paste(":5");
        f.select(Field::Name);
        assert_eq!(f.minute, "05");
        assert_eq!(f.schedule().unwrap(), "5 * * * *");
        // What cannot be read stays as typed, and says what is wrong.
        f.minute = "75".into();
        assert!(f.schedule().unwrap_err().contains("0 to 59"));
        f.choose(Field::Repeat, Repeat::Once.label());
        f.date = "2026-02-30".into();
        assert!(f.schedule().unwrap_err().contains("YYYY-MM-DD"));
    }

    #[test]
    fn the_custom_text_comes_back_and_an_untouched_schedule_stays() {
        // Read from the routine: the owner's own text.
        let mut f = Form::new("~");
        f.load_schedule("15 8-18 * * 1-5");
        assert!(f.cron_own);
        f.choose(Field::Repeat, Repeat::Daily.label());
        assert_eq!(f.schedule().unwrap(), "0 9 * * *");
        f.choose(Field::Repeat, Repeat::Hourly.label());
        f.choose(Field::Repeat, Repeat::Custom.label());
        assert_eq!(f.cron, "15 8-18 * * 1-5", "back as it was");
        assert_eq!(f.schedule().unwrap(), "15 8-18 * * 1-5");
        // Never typed: "Custom" starts from the schedule so far.
        let mut f = Form::new("~");
        f.choose(Field::Repeat, Repeat::Weekly.label());
        f.choose(Field::Repeat, Repeat::Custom.label());
        assert_eq!(f.cron, "0 9 * * 1");
        f.choose(Field::Repeat, Repeat::TwoWeeks.label());
        f.date = "2026-11-09".into();
        f.choose(Field::Repeat, Repeat::Custom.label());
        assert_eq!(f.cron, "@every 2 weeks from 2026-11-09 09:00");
        // Typed: it is kept from then on.
        f.select(Field::Cron);
        f.clear();
        f.paste("5 4 * * *");
        f.choose(Field::Repeat, Repeat::Monthly.label());
        f.choose(Field::Repeat, Repeat::Custom.label());
        assert_eq!(f.cron, "5 4 * * *");

        // Saving a form in which nothing under "When" changed keeps the
        // schedule as written, also where a choice would write it shorter.
        for text in ["0 9 * * MON", "@daily", "00 09 * * 1-5", "0 9 * * 5,1,3"] {
            let mut f = Form::new("/tmp");
            f.load_schedule(text);
            f.name = "Report".into();
            f.prompt = "Do it".into();
            f.select(Field::Name);
            f.type_char('!');
            assert_ne!(f.repeat, Repeat::Custom, "{text}");
            assert_eq!(f.build().unwrap().0.schedule, text);
            // A change under "When" writes it from the choices.
            f.select(Field::Time);
            f.backspace();
            f.type_char('5');
            assert_ne!(f.build().unwrap().0.schedule, text);
        }
    }

    #[test]
    fn a_cron_job_gets_the_schedules_cron_knows() {
        let mut f = Form::new("~");
        f.choose(Field::Agent, SHELL);
        let labels = |f: &Form, field| -> Vec<String> {
            f.items(field).into_iter().map(|i| i.label).collect()
        };
        assert!(labels(&f, Field::Repeat).contains(&"Every 2 weeks".to_string()));
        f.choose(Field::Repeat, Repeat::Monthly.label());
        f.choose(Field::MonthOn, &MonthOn::LastDay.value());
        assert_eq!(f.items(Field::MonthOn).len(), 8);
        f.choose(Field::RunsIn, RunsIn::Cron.label());
        assert!(f.is_cron());
        assert!(!labels(&f, Field::Repeat).contains(&"Every 2 weeks".to_string()));
        assert_eq!(labels(&f, Field::MonthOn), ["A day of the month"]);
        // What it had before says why it cannot be saved.
        f.name = "Stamp".into();
        f.prompt = "date >> ~/stamp.log".into();
        assert!(f.build().unwrap_err().contains("cron itself knows"));
        f.choose(Field::Repeat, Repeat::Monthly.label());
        assert_eq!(f.month_on, MonthOn::Day);
        assert_eq!(f.build().unwrap().0.schedule, "0 9 1 * *");
        for r in [Repeat::Minutes, Repeat::Days, Repeat::Once, Repeat::AtStart] {
            f.choose(Field::Repeat, r.label());
            assert!(f.build().is_ok(), "{r:?}");
        }
    }

    #[test]
    fn further_settings_are_chosen_and_kept() {
        use crate::accounts::Account;
        let mut f = Form::new("/tmp");
        f.name = "Report".into();
        f.prompt = "Do it".into();
        // Hidden until "More settings" opens; no account without the plugin.
        for field in [Field::OffAfter, Field::After, Field::AfterWhen] {
            assert!(!f.fields().contains(&field) && f.all_fields().contains(&field));
        }
        assert!(!f.all_fields().contains(&Field::Account));
        // A missed time: however late, within some hours, or skipped.
        assert_eq!(f.shown(Field::Missed), "Run once");
        f.choose(Field::Missed, "once:2");
        assert_eq!(
            f.shown(Field::Missed),
            "Run once, if less than 2 hours late"
        );
        assert_eq!((f.missed, f.extra.late_hours), (Missed::RunOnce, 2));
        f.choose(Field::Missed, "skip");
        assert_eq!(
            (f.shown(Field::Missed).as_str(), f.missed),
            ("Skip", Missed::Skip)
        );
        f.choose(Field::Missed, "once:1");
        assert_eq!(f.shown(Field::Missed), "Run once, if less than 1 hour late");
        f.choose(Field::OffAfter, "3");
        assert_eq!(f.shown(Field::OffAfter), "3 runs in a row that went wrong");
        f.select(Field::After);
        f.paste("~/bin/tell-me.sh");
        assert_eq!(f.shown(Field::AfterWhen), "Runs that went wrong");
        f.choose(Field::AfterWhen, "always");
        let (file, _) = f.build().unwrap();
        assert_eq!(
            file.more,
            crate::routine::More {
                late_hours: 1,
                after_command: "~/bin/tell-me.sh".into(),
                after_always: true,
                off_after_failures: 3,
                account: String::new(),
            }
        );
        assert_eq!(
            f.more_summary(),
            "Auto · catches up within 1 h · off after 3 that went wrong · a command after every run"
        );
        // With the Multiaccount plugin, a Claude Code routine names its
        // account.
        f.accounts = Some(vec![
            Account {
                id: "main".into(),
                name: "Main login".into(),
                kind: "main".into(),
            },
            Account {
                id: "team-two".into(),
                name: "Team two".into(),
                kind: "subscription".into(),
            },
        ]);
        assert!(f.all_fields().contains(&Field::Account));
        assert_eq!(f.shown(Field::Account), "Multiaccount decides");
        f.choose(Field::Account, "team-two");
        assert_eq!(f.shown(Field::Account), "Team two");
        assert_eq!(f.detail(Field::Account), "A subscription");
        assert!(f.more_summary().contains("account Team two"));
        assert_eq!(f.build().unwrap().0.more.account, "team-two");

        // Opened and saved untouched, every value stays, also ones the
        // lists do not offer and an account that is gone.
        let mut file = f.build().unwrap().0;
        file.more.late_hours = 36;
        file.more.off_after_failures = 7;
        file.more.account = "old-one".into();
        let r = Routine {
            id: "report".into(),
            file: file.clone(),
            prompt: "Do it".into(),
            problem: None,
            unreadable: false,
            approved: true,
        };
        let mut back = Form::edit(&r);
        back.accounts = f.accounts.clone();
        assert_eq!(back.build().unwrap().0, file);
        assert_eq!(
            back.shown(Field::Missed),
            "Run once, if less than 36 hours late"
        );
        assert_eq!(
            back.shown(Field::OffAfter),
            "7 runs in a row that went wrong"
        );
        assert_eq!(
            back.shown(Field::Account),
            "old-one (not in Multiaccount any more)"
        );
        assert!(back
            .items(Field::Account)
            .iter()
            .any(|i| i.value == "old-one"));
        assert!(back
            .items(Field::Missed)
            .iter()
            .any(|i| i.value == "once:36"));
        // Without the plugin the account is not shown, and still kept.
        back.accounts = None;
        assert!(!back.all_fields().contains(&Field::Account));
        assert_eq!(back.build().unwrap().0.more.account, "old-one");
        // Another agent has no account: it goes with the agent.
        back.choose(Field::Agent, "codex");
        assert_eq!(back.build().unwrap().0.more.account, "");
    }

    #[test]
    fn rejects_bad_input() {
        let mut f = Form::new("~");
        f.time = "25:00".into();
        assert!(f.schedule().is_err());
        f.choose(Field::Repeat, Repeat::Hourly.label());
        f.minute = ":75".into();
        assert!(f.schedule().is_err());
        f.minute = ":05".into();
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
        assert!(f.all_fields().contains(&Field::Effort));
        assert!(!f.fields().contains(&Field::Effort), "behind More settings");
        f.choose(Field::Model, "opus");
        f.choose(Field::Effort, "high");
        f.choose(Field::Agent, "amp");
        assert_eq!(
            (f.mode.as_str(), f.model.as_str(), f.effort.as_str()),
            ("", "", "")
        );
        assert!(!f.all_fields().contains(&Field::Mode));
        assert_eq!(f.current(), Field::Agent);
        f.choose(Field::Agent, "codex");
        assert_eq!(f.mode, "auto");
        assert!(f.all_fields().contains(&Field::Model));
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
        let fields = f.all_fields();
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
        f.choose(Field::Alert, Alert::PopupOnProblems.label());
        let (file, _) = f.build().unwrap();
        assert_eq!(file.alert, Alert::PopupOnProblems);
        let text = toml::to_string(&file).unwrap();
        assert!(text.contains("alert = \"popup-on-problems\""));
        assert!(file.alert.popup(true) && !file.alert.popup(false));
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
        assert_eq!(back.shown(Field::Alert), "Popup on problems");
        // "When Herdr starts" needs no time and no missed setting.
        f.choose(Field::Repeat, Repeat::AtStart.label());
        assert!(!f.all_fields().contains(&Field::Time));
        assert!(!f.all_fields().contains(&Field::Missed));
        assert_eq!(f.build().unwrap().0.schedule, "@start");
        // Shell commands have no check and no memory.
        f.choose(Field::Agent, SHELL);
        assert!(!f.all_fields().contains(&Field::Check));
        assert!(!f.build().unwrap().0.remember);
        assert!(f.build().unwrap().0.check_command.is_empty());
    }

    #[test]
    fn the_form_starts_with_what_when_and_where() {
        let mut f = Form::new("~/code/app");
        assert_eq!(
            f.fields(),
            vec![
                Field::Name,
                Field::Agent,
                Field::Prompt,
                Field::Repeat,
                Field::Time,
                Field::Folder,
                Field::More,
                Field::Save
            ]
        );
        assert_eq!(f.more_summary(), "Auto");
        // The rest shows under "More settings", with today's defaults.
        f.select(Field::More);
        f.toggle_more();
        assert_eq!(f.current(), Field::More);
        for hidden in [Field::Mode, Field::Limit, Field::Retry, Field::Keep] {
            assert!(f.fields().contains(&hidden), "{hidden:?}");
        }
        assert_eq!(f.fields().last(), Some(&Field::Save));
        f.toggle_more();
        // Going to a hidden field opens them.
        f.select(Field::Limit);
        assert!(f.more);
        assert_eq!(f.current(), Field::Limit);
        f.limit = "30".into();
        f.retries = 1;
        f.enabled = false;
        assert_eq!(
            f.more_summary(),
            "Auto · switched off · 30 min limit · try once more"
        );
    }

    #[test]
    fn text_fields_have_a_cursor_that_moves() {
        let mut f = Form::new("~");
        for c in "Wekly chek".chars() {
            f.type_char(c);
        }
        assert_eq!(f.cursor, 10);
        // Left to behind "che", then the missing letter.
        f.cursor_by(-1);
        f.type_char('c');
        assert_eq!(f.name, "Wekly check");
        f.cursor_home();
        f.cursor_by(2);
        f.type_char('e');
        assert_eq!(f.name, "Weekly check");
        // Delete removes at the cursor, Backspace before it.
        f.delete();
        assert_eq!(f.name, "Weely check");
        f.backspace();
        assert_eq!(f.name, "Wely check");
        f.cursor_home();
        f.backspace();
        f.cursor_by(-5);
        assert_eq!((f.name.as_str(), f.cursor), ("Wely check", 0));
        f.cursor_end();
        f.delete();
        f.paste(" ü!");
        assert_eq!(f.name, "Wely check ü!");
        f.cursor_by(-1);
        f.backspace();
        assert_eq!(f.name, "Wely check !");
        // Each field starts with the cursor behind its text.
        f.select(Field::Folder);
        assert_eq!(f.cursor, 1);
        f.clear();
        assert_eq!((f.folder.as_str(), f.cursor), ("", 0));
    }
}
