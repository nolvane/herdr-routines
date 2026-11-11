//! The person's crontab (`crontab -l`).
//!
//! Its jobs show in the list. A job can be taken over: moved to Herdr (its
//! line is commented out, so nothing runs twice), copied to Herdr switched
//! off, or kept as a cron job that this plugin manages.
//!
//! A managed cron job stays an ordinary line in the crontab, where it was,
//! with its own command. The plugin only puts a marker above it and sets
//! `SHELL` around it to a small wrapper (a link to the plugin binary, named
//! after the job), so cron starts the command through the plugin, which
//! records the run (output, result, exit code) and then hands the output on
//! as cron would. Name, alerts, retries and the like live in the plugin;
//! schedule and command live in the crontab, and edits on either side win
//! (the list writes the crontab when you save, and reads it back when it
//! opens and every few seconds).

//!
//! The crontab is the person's own file: it is handled as bytes, so lines
//! this plugin does not touch go back exactly as they were (also text that
//! is not UTF-8), and every change is made under a lock, on a fresh read.

use crate::approval::Approvals;
use crate::paths::{self, Paths};
use crate::routine::{Missed, Routine, RoutineFile, RunsIn, SHELL};
use crate::schedule::{Schedule, AT_START};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long `crontab` may take before a call is given up.
const LIMIT: Duration = Duration::from_secs(20);

/// The program to call. Tests never reach the real crontab: they set a
/// stand-in for their own thread with `use_fake`, and without one there is
/// no crontab at all.
fn program() -> Option<String> {
    #[cfg(test)]
    {
        FAKE.with(|f| f.borrow().clone())
    }
    #[cfg(not(test))]
    {
        Some("crontab".into())
    }
}

#[cfg(test)]
thread_local! {
    static FAKE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Tests: a stand-in for `crontab` on this thread.
#[cfg(test)]
pub fn use_fake(program: Option<&std::path::Path>) {
    FAKE.with(|f| *f.borrow_mut() = program.map(|p| p.to_string_lossy().into_owned()));
}

/// The start of a line this plugin commented out.
const MOVED: &str = "# moved to herdr-routines";
/// The marker above a managed cron job: `# herdr-routines: <id> …`.
const MARK: &str = "# herdr-routines:";
/// A managed cron job that is switched off.
const OFF: &str = "#off ";

#[derive(Debug, Clone, PartialEq)]
pub struct CronJob {
    /// The line as it is in the crontab.
    pub line: String,
    /// The schedule as a routine has it (`@start` for `@reboot`).
    pub schedule: String,
    /// The command, with cron's `\%` turned into `%`.
    pub command: String,
    /// Variables set above it in the crontab (`PATH=…`), MAILTO left out.
    pub env: Vec<(String, String)>,
    /// Something to look at before relying on it.
    pub note: Option<String>,
}

/// The crontab as its lines, byte for byte. Reading and showing it goes
/// through [`Tab::text`]; a change replaces single lines and leaves the
/// bytes of all others alone.
#[derive(Debug, Clone, PartialEq, Default)]
struct Tab {
    lines: Vec<Vec<u8>>,
}

/// A line as text: bytes that are not UTF-8 show as the replacement
/// character, and a carriage return at the end is left off.
fn shown(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

impl Tab {
    fn from_bytes(bytes: &[u8]) -> Tab {
        let mut lines: Vec<Vec<u8>> = bytes.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
        // What follows the last line break is a line only when it has text.
        if lines.last().is_some_and(Vec::is_empty) {
            lines.pop();
        }
        Tab { lines }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = self.lines.join(&b'\n');
        if !self.lines.is_empty() {
            out.push(b'\n');
        }
        out
    }

    /// The crontab as text, line for line (see [`shown`]): line `n` of the
    /// text is line `n` here.
    fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&shown(line));
            out.push('\n');
        }
        out
    }

    /// Puts `new` in place of the lines in `range`. A new line that reads
    /// like the line it replaces keeps that line's bytes (its line end).
    fn put(&mut self, range: std::ops::Range<usize>, new: Vec<Vec<u8>>) {
        let old: Vec<Vec<u8>> = self.lines[range.clone()].to_vec();
        let new = new
            .into_iter()
            .enumerate()
            .map(|(i, line)| match old.get(i) {
                Some(bytes) if shown(bytes) == shown(&line) => bytes.clone(),
                _ => line,
            });
        self.lines.splice(range, new.collect::<Vec<_>>());
    }
}

/// The crontab as it is now, empty when there is none.
fn read_tab() -> Result<Tab, String> {
    let Some(program) = program() else {
        return Ok(Tab::default());
    };
    let mut command = Command::new(program);
    command.arg("-l");
    let out = crate::shell::tool(command, LIMIT).map_err(|e| format!("cannot run crontab: {e}"))?;
    if out.success {
        return Ok(Tab::from_bytes(&out.stdout));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("no crontab") {
        Ok(Tab::default())
    } else {
        Err(err.trim().to_string())
    }
}

/// The crontab as text, empty when there is none.
pub fn read() -> Result<String, String> {
    read_tab().map(|t| t.text())
}

fn env_assignment(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    let ok = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if !ok {
        return None;
    }
    let v = value.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v);
    Some((name.to_string(), v.to_string()))
}

/// A job line's schedule (as cron has it, and as a routine has it) and its
/// command as written. None for anything that is not a job.
fn split_job(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    let (when, schedule, command) = if let Some(rest) = line.strip_prefix('@') {
        let (word, command) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let schedule = match word {
            "reboot" => AT_START,
            "yearly" | "annually" => "0 0 1 1 *",
            "monthly" => "0 0 1 * *",
            "weekly" => "0 0 * * 0",
            "daily" | "midnight" => "0 0 * * *",
            "hourly" => "0 * * * *",
            _ => return None,
        };
        (
            format!("@{word}"),
            schedule.to_string(),
            command.trim().to_string(),
        )
    } else {
        if !line.starts_with(|c: char| c.is_ascii_digit() || c == '*') {
            return None;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            return None;
        }
        // The command keeps its own spacing: cut after the fifth field.
        let mut rest = line;
        for _ in 0..5 {
            rest = rest.trim_start();
            rest = &rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..];
        }
        let fields = parts[..5].join(" ");
        (fields.clone(), fields, rest.trim().to_string())
    };
    if command.is_empty() || Schedule::parse(&schedule).is_err() {
        return None;
    }
    Some((when, schedule, command))
}

/// A cron job this plugin manages: a group of lines in the crontab
/// (marker, `SHELL=` wrapper, the job, `SHELL=` back to before).
#[derive(Debug, Clone, PartialEq)]
pub struct Managed {
    pub id: String,
    pub enabled: bool,
    /// When, as the crontab says it (`*/5 * * * *`, `@reboot`).
    pub when: String,
    /// When, as a routine has it.
    pub schedule: String,
    /// The command, with cron's `\%` turned into `%`.
    pub command: String,
    /// The group's lines: from `start`, `len` of them.
    pub start: usize,
    pub len: usize,
    /// The shell the crontab had before the group; the group ends with it.
    pub shell: String,
}

/// The managed cron jobs in a crontab, in order.
pub fn managed(text: &str) -> Vec<Managed> {
    let lines: Vec<&str> = text.lines().collect();
    let mut shell = "/bin/sh".to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(rest) = t.strip_prefix(MARK) {
            let id = rest.split_whitespace().next().unwrap_or("").to_string();
            let mut j = i + 1;
            if lines.get(j).is_some_and(|l| l.trim().starts_with("SHELL=")) {
                j += 1;
            }
            if let Some(job) = lines.get(j) {
                let job = job.trim();
                let (enabled, body) = match job.strip_prefix(OFF) {
                    Some(b) => (false, b),
                    None => (true, job),
                };
                if let Some((when, schedule, command)) = split_job(body) {
                    j += 1;
                    if lines.get(j).is_some_and(|l| l.trim().starts_with("SHELL=")) {
                        j += 1;
                    }
                    out.push(Managed {
                        id,
                        enabled,
                        when,
                        schedule,
                        command: command.replace("\\%", "%"),
                        start: i,
                        len: j - i,
                        shell: shell.clone(),
                    });
                    i = j;
                    continue;
                }
            }
        } else if let Some((k, v)) = env_assignment(t) {
            if k == "SHELL" && !t.starts_with('#') {
                shell = v;
            }
        }
        i += 1;
    }
    out
}

/// The jobs in a crontab that this plugin does not manage, in order. Lines
/// it cannot read are left out.
pub fn parse(text: &str) -> Vec<CronJob> {
    let skip: Vec<std::ops::Range<usize>> = managed(text)
        .iter()
        .map(|m| m.start..m.start + m.len)
        .collect();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut jobs = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        if skip.iter().any(|r| r.contains(&n)) {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((_, schedule, command)) = split_job(line) else {
            if let Some((k, v)) = env_assignment(line) {
                env.retain(|(name, _)| *name != k);
                if k != "MAILTO" {
                    env.push((k, v));
                }
            }
            continue;
        };
        // In cron an unescaped % ends the command and feeds the rest in.
        let bare_percent = command
            .char_indices()
            .any(|(i, c)| c == '%' && (i == 0 || command.as_bytes()[i - 1] != b'\\'));
        jobs.push(CronJob {
            line: raw.to_string(),
            schedule,
            command: command.replace("\\%", "%"),
            env: env.clone(),
            note: bare_percent
                .then(|| "uses % (in cron: input for the command); check the command".to_string()),
        });
    }
    jobs
}

/// A short name from the command, such as "backup.sh --all".
pub fn name_for(command: &str) -> String {
    let main = command
        .split(['>', '|', ';', '&'])
        .next()
        .unwrap_or(command)
        .trim();
    let mut words = main.split_whitespace();
    let first = words.next().unwrap_or("command");
    let first = first.rsplit('/').next().unwrap_or(first);
    let mut name: String = std::iter::once(first)
        .chain(words)
        .collect::<Vec<_>>()
        .join(" ");
    if name.chars().count() > 40 {
        name = name.chars().take(39).collect::<String>() + "…";
    }
    if name.is_empty() {
        "Cron job".into()
    } else {
        name
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A cron job's routine file: a shell command in the home folder (where
/// cron runs it).
fn cron_file(name: String, schedule: String, enabled: bool) -> RoutineFile {
    RoutineFile {
        name,
        enabled,
        schedule,
        agent: SHELL.into(),
        permission_mode: String::new(),
        model: String::new(),
        effort: String::new(),
        args: Vec::new(),
        folder: "~".into(),
        missed: Missed::Skip,
        time_limit_minutes: 120,
        stop_at_limit: false,
        retries: 0,
        retry_delay_minutes: 10,
        check_command: String::new(),
        remember: false,
        alert: Default::default(),
        runs_in: RunsIn::Herdr,
        keep_runs: crate::routine::KEEP_RUNS,
        keep_days: crate::routine::KEEP_DAYS,
    }
}

/// The Herdr routine a job becomes: its command with its crontab variables,
/// skipping missed times as cron does.
pub fn routine_for(job: &CronJob, enabled: bool) -> (RoutineFile, String) {
    let mut script = format!("# From the crontab: {}\n", job.line.trim());
    for (k, v) in &job.env {
        script.push_str(&format!("export {k}={}\n", quote(v)));
    }
    script.push_str(&job.command);
    script.push('\n');
    let file = cron_file(name_for(&job.command), job.schedule.clone(), enabled);
    (file, script)
}

/// What goes in front of a moved job's line to comment it out.
fn moved_mark(line: &str, moved: &[(String, String)]) -> Option<String> {
    moved
        .iter()
        .find(|(l, _)| l == line)
        .map(|(_, id)| format!("{MOVED} (routine {id}): "))
}

/// The crontab with the moved jobs' lines commented out.
#[cfg(test)]
pub fn without(text: &str, moved: &[(String, String)]) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match moved_mark(line, moved) {
            Some(mark) => out.push(format!("{mark}{line}")),
            None => out.push(line.to_string()),
        }
    }
    out.join("\n") + "\n"
}

/// [`without`] on the crontab's own bytes: a commented line keeps them.
fn without_moved(tab: &Tab, moved: &[(String, String)]) -> Tab {
    let lines = tab.lines.iter().map(|line| {
        let text = shown(line);
        match moved_mark(&text, moved) {
            Some(mark) => [mark.as_bytes(), line].concat(),
            None => line.clone(),
        }
    });
    Tab {
        lines: lines.collect(),
    }
}

/// What becomes of a cron job taken over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Take {
    /// A routine that runs in Herdr, on; the crontab line is commented out.
    ToHerdr,
    /// Stays a cron job, where it is, now managed here (name, runs).
    KeepCron,
    /// A routine that runs in Herdr, switched off; the crontab stays.
    Copy,
}

/// Takes over the jobs (see `Take`). Changing the crontab keeps a backup of
/// it in the state folder. Returns how many were taken over.
pub fn import(paths: &Paths, jobs: &[CronJob], how: Take) -> Result<usize, String> {
    if how == Take::KeepCron {
        for job in jobs {
            keep(paths, job)?;
        }
        return Ok(jobs.len());
    }
    let moving = how == Take::ToHerdr;
    let mut moved: Vec<(String, String)> = Vec::new();
    for job in jobs {
        let (file, script) = routine_for(job, moving);
        let id = crate::routine::new_id(paths, &file.name);
        let r = Routine {
            id: id.clone(),
            file,
            prompt: script,
            problem: None,
            unreadable: false,
            approved: true,
        };
        // The owner chose these jobs in the list after seeing their lines.
        r.save_approved(paths)?;
        moved.push((job.line.clone(), id));
    }
    if moving && !moved.is_empty() {
        // On a fresh read: only lines still there are changed.
        change(paths, |tab| Ok(Some(without_moved(tab, &moved)))).map_err(|e| {
            format!("the routines were added, but {e}; comment the lines out by hand")
        })?;
    }
    Ok(moved.len())
}

/// Manages a job of the crontab where it is: a routine for its name and
/// settings, and the marker and wrapper around its line.
fn keep(paths: &Paths, job: &CronJob) -> Result<(), String> {
    let mut file = cron_file(name_for(&job.command), job.schedule.clone(), true);
    file.runs_in = RunsIn::Cron;
    let id = crate::routine::new_id(paths, &file.name);
    let r = Routine {
        id: id.clone(),
        file,
        prompt: job.command.clone(),
        problem: None,
        unreadable: false,
        approved: true,
    };
    r.save_approved(paths)?;
    prepare_wrapper(paths, &id)?;
    change(paths, |tab| {
        let now = tab.text();
        let skip: Vec<std::ops::Range<usize>> = managed(&now)
            .iter()
            .map(|m| m.start..m.start + m.len)
            .collect();
        let Some(at) = now
            .lines()
            .enumerate()
            .position(|(n, l)| l == job.line && !skip.iter().any(|r| r.contains(&n)))
        else {
            return Err("the job is no longer in the crontab".into());
        };
        let shell = shell_before(&now, at);
        // The line itself stays as written, byte for byte; only the
        // marker and SHELL go around.
        let mut next = tab.clone();
        next.lines
            .insert(at + 1, format!("SHELL={shell}").into_bytes());
        next.lines.insert(
            at,
            format!("SHELL={}", wrapper(paths, &id).display()).into_bytes(),
        );
        next.lines.insert(at, marker(&id).into_bytes());
        Ok(Some(next))
    })
    .inspect_err(|_| {
        // Not taken over after all: the routine made for it goes again.
        let _ = Routine::delete(paths, &id, true);
        let _ = std::fs::remove_file(wrapper(paths, &id));
    })
}

// ---- managed cron jobs ----------------------------------------------------

fn marker(id: &str) -> String {
    format!("{MARK} {id} (its name and runs are in the herdr-routines list)")
}

/// The shell in effect at a line of the crontab (before it).
fn shell_before(text: &str, line: usize) -> String {
    let mut shell = "/bin/sh".to_string();
    let groups = managed(text);
    for (n, l) in text.lines().enumerate().take(line) {
        if groups
            .iter()
            .any(|m| (m.start..m.start + m.len).contains(&n))
        {
            continue;
        }
        let t = l.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = env_assignment(t) {
            if k == "SHELL" {
                shell = v;
            }
        }
    }
    shell
}

/// The copy of the plugin binary the crontab's wrappers point to (the
/// plugin's own folder changes with each update).
pub fn binary(paths: &Paths) -> PathBuf {
    paths.state.join("bin").join("herdr-routines")
}

/// The wrapper cron starts a managed job with: a link to `binary`, named
/// after the job.
pub fn wrapper(paths: &Paths, id: &str) -> PathBuf {
    paths.state.join("bin").join("cron").join(id)
}

/// What a wrapper needs to know besides its own place: written next to
/// the wrappers.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct WrapperEnv {
    config: PathBuf,
    socket: PathBuf,
    herdr: Option<PathBuf>,
}

/// Puts the running binary where the wrappers point, when it differs.
fn place_binary(paths: &Paths) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = binary(paths);
    if me == target {
        return Ok(());
    }
    let bytes = std::fs::read(&me).map_err(|e| e.to_string())?;
    if std::fs::read(&target).ok().as_deref() == Some(&bytes[..]) {
        return Ok(());
    }
    if let Some(dir) = target.parent() {
        paths::create_private_dir(dir).map_err(|e| e.to_string())?;
    }
    paths::write_atomic(&target, &bytes).map_err(|e| format!("cannot copy the plugin: {e}"))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())
}

/// The binary, the job's wrapper link and what the wrappers need to know.
pub fn prepare_wrapper(paths: &Paths, id: &str) -> Result<(), String> {
    place_binary(paths)?;
    let link = wrapper(paths, id);
    let dir = link.parent().ok_or("no folder")?;
    paths::create_private_dir(dir).map_err(|e| e.to_string())?;
    if std::fs::read_link(&link).ok().as_deref() != Some(binary(paths).as_path()) {
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(binary(paths), &link)
            .map_err(|e| format!("cannot make the cron wrapper: {e}"))?;
    }
    let herdr = crate::herdr::bin();
    let env = WrapperEnv {
        config: paths.config.clone(),
        socket: paths.socket.clone(),
        herdr: herdr.is_absolute().then_some(herdr),
    };
    let text = serde_json::to_vec_pretty(&env).map_err(|e| e.to_string())?;
    paths::write_atomic(&dir.join("env.json"), &text).map_err(|e| e.to_string())
}

/// When cron starts a managed job, as its crontab line says it: five cron
/// fields, or `@reboot`.
pub fn when(r: &Routine) -> Option<String> {
    use chrono::{Datelike, Timelike};
    match r.schedule()? {
        Schedule::AtStart => Some("@reboot".into()),
        Schedule::Once(t) => Some(format!(
            "{} {} {} {} *",
            t.minute(),
            t.hour(),
            t.day(),
            t.month()
        )),
        // Only what cron itself understands; "every two weeks" and the
        // last day of a month are Herdr's own.
        Schedule::Cron(c) => c.classic().then(|| r.file.schedule.trim().to_string()),
        Schedule::Every(_) => None,
    }
}

/// The group of lines for a managed job.
fn group(paths: &Paths, r: &Routine, shell: &str) -> Result<Vec<String>, String> {
    let when = when(r).ok_or("the schedule cannot be read")?;
    let command = r.prompt.trim();
    if command.contains('\n') {
        return Err("a cron job's command is one line".into());
    }
    // In the crontab a % starts the command's input: escape it.
    let mut escaped = String::new();
    let mut prev = ' ';
    for c in command.chars() {
        if c == '%' && prev != '\\' {
            escaped.push('\\');
        }
        escaped.push(c);
        prev = c;
    }
    let job = format!("{when} {escaped}");
    Ok(vec![
        marker(&r.id),
        format!("SHELL={}", wrapper(paths, &r.id).display()),
        if r.file.enabled {
            job
        } else {
            format!("{OFF}{job}")
        },
        format!("SHELL={shell}"),
    ])
}

/// Writes a managed cron job into the crontab: in place when it is there,
/// else at the end.
///
/// Only an approved routine reaches the crontab. One that waits for
/// approval (its files changed outside the list) leaves its group as it
/// is, so cron keeps running the line the owner approved until `a`
/// approves the new one; whichever caller asks, nothing is written.
pub fn write_job(paths: &Paths, r: &Routine) -> Result<(), String> {
    if !r.approved {
        return Ok(());
    }
    // Checked before anything is touched.
    group(paths, r, "/bin/sh")?;
    let mut prepared = false;
    change(paths, |tab| {
        let now = tab.text();
        let mut next = tab.clone();
        match managed(&now).into_iter().find(|m| m.id == r.id) {
            Some(m) => {
                // A job whose time and command are as they were keeps its
                // line byte for byte (it need not be UTF-8, and its spacing
                // is the owner's); only on or off follows the routine.
                let same = when(r).as_deref() == Some(m.when.as_str())
                    && r.prompt.trim() == m.command.trim();
                let kept = job_line(tab, &m)
                    .filter(|_| same)
                    .map(|(_, line)| without_off(line).to_vec());
                let mut g: Vec<Vec<u8>> = group(paths, r, &m.shell)?
                    .into_iter()
                    .map(String::into_bytes)
                    .collect();
                if let Some(line) = kept {
                    g[2] = if r.file.enabled {
                        line
                    } else {
                        [OFF.as_bytes(), &line].concat()
                    };
                }
                next.put(m.start..m.start + m.len, g);
            }
            None => {
                let g = group(paths, r, &shell_before(&now, next.lines.len()))?;
                if next
                    .lines
                    .last()
                    .is_some_and(|l| !shown(l).trim().is_empty())
                {
                    next.lines.push(Vec::new());
                }
                next.lines.extend(g.into_iter().map(String::into_bytes));
            }
        }
        if next != *tab && !prepared {
            prepare_wrapper(paths, &r.id)?;
            prepared = true;
        }
        Ok(Some(next))
    })
}

/// Where a managed group's job line is, and its bytes.
fn job_line<'a>(tab: &'a Tab, m: &Managed) -> Option<(usize, &'a [u8])> {
    (m.start..m.start + m.len).find_map(|n| {
        let line = tab.lines.get(n)?;
        let text = shown(line);
        let text = text.trim();
        split_job(text.strip_prefix(OFF).unwrap_or(text)).map(|_| (n, line.as_slice()))
    })
}

/// A job line's bytes without the `#off ` in front.
fn without_off(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    line[start..].strip_prefix(OFF.as_bytes()).unwrap_or(line)
}

/// Switches a managed job's line in the crontab on or off and changes
/// nothing else about it. For a routine that waits for approval: the line
/// stays the approved one, but `space` in the list still stops and starts
/// it.
pub fn switch(paths: &Paths, id: &str, enabled: bool) -> Result<(), String> {
    change(paths, |tab| {
        let Some(m) = managed(&tab.text()).into_iter().find(|m| m.id == id) else {
            return Ok(None);
        };
        let Some((at, line)) = job_line(tab, &m).filter(|_| m.enabled != enabled) else {
            return Ok(None);
        };
        let line = if enabled {
            without_off(line).to_vec()
        } else {
            [OFF.as_bytes(), line].concat()
        };
        let mut next = tab.clone();
        next.lines[at] = line;
        Ok(Some(next))
    })
}

/// A managed group's lines turned back into what the job was before: its
/// own line (commented out when it is switched off), or nothing.
fn released(tab: &Tab, m: &Managed, keep_line: bool) -> Vec<Vec<u8>> {
    match job_line(tab, m).filter(|_| keep_line) {
        Some((_, line)) if m.enabled => vec![line.to_vec()],
        Some((_, line)) => vec![[b"# ", without_off(line)].concat()],
        None => Vec::new(),
    }
}

/// Takes a managed job out of the crontab. `keep_line`: its line stays as
/// an ordinary job (not managed any more); otherwise it goes.
pub fn remove_job(paths: &Paths, id: &str, keep_line: bool) -> Result<(), String> {
    change(paths, |tab| {
        let Some(m) = managed(&tab.text()).into_iter().find(|m| m.id == id) else {
            return Ok(None);
        };
        let mut next = tab.clone();
        next.lines
            .splice(m.start..m.start + m.len, released(tab, &m, keep_line));
        Ok(Some(next))
    })?;
    let _ = std::fs::remove_file(wrapper(paths, id));
    Ok(())
}

/// Hands every managed cron job back to cron as a plain crontab line: the
/// marker and the `SHELL` lines go, the job's own line stays (commented
/// out when it is switched off). For before the plugin or its state folder
/// is removed: a managed job is started through a wrapper in that folder,
/// and without it cron can no longer start the job at all. Returns how
/// many jobs were handed back.
pub fn release_all(paths: &Paths) -> Result<usize, String> {
    let mut ids: Vec<String> = Vec::new();
    change(paths, |tab| {
        let groups = managed(&tab.text());
        ids = groups.iter().map(|m| m.id.clone()).collect();
        if groups.is_empty() {
            return Ok(None);
        }
        let mut next = tab.clone();
        // From the end, so the lines before keep their numbers.
        for m in groups.iter().rev() {
            next.lines
                .splice(m.start..m.start + m.len, released(tab, m, true));
        }
        Ok(Some(next))
    })?;
    for id in &ids {
        let _ = std::fs::remove_file(wrapper(paths, id));
    }
    Ok(ids.len())
}

/// Brings the managed cron jobs in the plugin in line with the crontab,
/// which wins for schedule, command and on or off (it may have been
/// edited by hand). A job whose lines are gone is switched off. Also keeps
/// the wrappers pointing at the current binary. Returns whether anything
/// changed.
pub fn pull(paths: &Paths) -> Result<bool, String> {
    let now = read()?;
    let found = managed(&now);
    let mut changed = false;
    for m in &found {
        if !crate::routine::valid_id(&m.id) {
            continue;
        }
        match Routine::load(paths, &m.id) {
            // One that waits for approval keeps its own version: approving
            // it writes that into the crontab.
            Ok(mut r) if r.is_cron() && r.approved => {
                let same_when =
                    when(&r).as_deref() == Some(m.when.as_str()) || r.file.schedule == m.schedule;
                if !same_when || r.prompt.trim() != m.command.trim() || r.file.enabled != m.enabled
                {
                    if !same_when {
                        r.file.schedule = m.schedule.clone();
                    }
                    r.prompt = m.command.clone();
                    r.file.enabled = m.enabled;
                    r.save_approved(paths)?;
                    changed = true;
                }
            }
            Ok(_) => {}
            Err(_) if !paths.routine_dir(&m.id).exists() => {
                // A marker without its routine (deleted files): back again.
                let mut file = cron_file(name_for(&m.command), m.schedule.clone(), m.enabled);
                file.runs_in = RunsIn::Cron;
                let r = Routine {
                    id: m.id.clone(),
                    file,
                    prompt: m.command.clone(),
                    problem: None,
                    unreadable: false,
                    approved: true,
                };
                r.save_approved(paths)?;
                changed = true;
            }
            Err(_) => {}
        }
        prepare_wrapper(paths, &m.id)?;
    }
    for mut r in crate::routine::load_all(paths) {
        // One that waits for approval may never have been written yet: its
        // line comes when it is approved.
        if r.is_cron() && r.approved && r.file.enabled && !found.iter().any(|m| m.id == r.id) {
            r.file.enabled = false;
            r.save_approved(paths)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// The backup of the crontab as it was before this plugin first changed
/// it. It is never deleted.
const FIRST_BACKUP: &str = "crontab-first.txt";
/// How many later backups are kept besides it.
const BACKUPS: usize = 5;

/// Changes the crontab. `edit` gets the crontab as it is now and returns
/// the one to install (`None`, or the same: nothing to do).
///
/// The crontab has no way to say "replace it only if it is still what I
/// read", so a change is: read, work out the new crontab, read again, and
/// install only when nothing changed in between (a hand edit, another
/// program); otherwise start over on the fresh read. Other processes of
/// this plugin (a second list, a cron job's wrapper) wait for the lock.
fn change(
    paths: &Paths,
    mut edit: impl FnMut(&Tab) -> Result<Option<Tab>, String>,
) -> Result<(), String> {
    paths::locked(&paths.crontab_lock_file(), || {
        for _ in 0..3 {
            let before = read_tab()?;
            let Some(next) = edit(&before)?.filter(|next| *next != before) else {
                return Ok(());
            };
            if read_tab()? == before {
                return install(paths, &before, &next);
            }
        }
        Err("the crontab kept changing while it was read; nothing was written".into())
    })
}

/// Replaces the crontab with `next`, after a backup of the current one
/// (`now`, read just before) in the state folder: the first one ever is
/// kept for good, of the later ones the last five.
fn install(paths: &Paths, now: &Tab, next: &Tab) -> Result<(), String> {
    let program = program().ok_or("there is no crontab here")?;
    let dir = paths.state.join("crontab-backups");
    paths::create_private_dir(&dir).map_err(|e| e.to_string())?;
    let backup = |name: String| {
        paths::write_atomic(&dir.join(name), &now.bytes())
            .map_err(|e| format!("cannot back up the crontab: {e}"))
    };
    if !dir.join(FIRST_BACKUP).exists() {
        backup(FIRST_BACKUP.to_string())?;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    backup(format!("crontab-{stamp}.txt"))?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    old.retain(|p| p.file_name().is_some_and(|n| n != FIRST_BACKUP));
    old.sort();
    let extra = old.len().saturating_sub(BACKUPS);
    for p in old.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
    // One file per process: two never share it.
    let file = paths
        .state
        .join(format!("crontab-new-{}.txt", std::process::id()));
    paths::write_atomic(&file, &next.bytes()).map_err(|e| e.to_string())?;
    let mut command = Command::new(program);
    command.arg(&file);
    let out = crate::shell::tool(command, LIMIT).map_err(|e| format!("cannot run crontab: {e}"));
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if out.success {
        Ok(())
    } else {
        Err(format!(
            "the crontab could not be changed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

// ---- the wrapper ------------------------------------------------------------

/// Whether the program was started as a cron job's wrapper: as
/// `<state>/bin/cron/<id> -c <command>`.
///
/// The arguments are taken as the system hands them over, not as text: a
/// command in the crontab need not be UTF-8, and it must run all the same.
pub fn wrapper_call<S: AsRef<OsStr>>(args: &[S]) -> Option<(PathBuf, std::ffi::OsString)> {
    if args.len() < 3 || args[1].as_ref() != "-c" {
        return None;
    }
    let me = PathBuf::from(args[0].as_ref());
    (me.parent()?.file_name()? == "cron").then(|| (me, args[2].as_ref().to_os_string()))
}

/// Starts a managed cron job now, as cron would (`r` in the list): its
/// wrapper with its command, in the background.
///
/// The routine is read again here and must be one that can run: the files
/// may have changed since the list (or whoever calls) last looked, and
/// the owner's "yes" was for the approved routine.
pub fn run_now(paths: &Paths, r: &Routine) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let r = &Routine::load(paths, &r.id)?;
    if let Some(problem) = &r.problem {
        return Err(problem.clone());
    }
    if !r.approved {
        return Err("it changed outside the list and waits for approval there (key a)".into());
    }
    if !r.is_cron() {
        return Err("it is not a cron job".into());
    }
    prepare_wrapper(paths, &r.id)?;
    // The command as the crontab has it, which is what cron runs.
    let command = managed(&read()?)
        .into_iter()
        .find(|m| m.id == r.id)
        .map(|m| m.command)
        .unwrap_or_else(|| r.prompt.trim().to_string());
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut child = Command::new(wrapper(paths, &r.id))
        .arg("-c")
        .arg(command)
        .env("HERDR_ROUTINES_TRIGGER", "manual")
        .current_dir(home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Its own group: closing the list does not stop it.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start it: {e}"))?;
    // Reaped when it ends, so no finished process lingers.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The wrapper: runs the command as cron would (the crontab's shell, `-c`),
/// records the run with its output, retries and alerts as the job's
/// settings say, and hands the output on to cron. Never keeps cron from
/// running the command: whatever is wrong with the plugin's side, the
/// command runs.
///
/// What the plugin adds (name, time limit, retries, alerts, runs kept)
/// comes from the routine as the owner approved it (`Approvals::routine`),
/// never from files that still wait for approval; and the wrapper itself
/// never approves anything.
pub fn wrapper_main(me: &Path, cmd: impl AsRef<OsStr>) -> i32 {
    use crate::store::{Run, Status, Trigger};
    let cmd = cmd.as_ref();
    let id = me
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let state = me
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let env: WrapperEnv = me
        .parent()
        .and_then(|d| std::fs::read(d.join("env.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let defaults = Paths::from_env();
    let paths = Paths {
        config: if env.config.as_os_str().is_empty() {
            defaults.config
        } else {
            env.config
        },
        state: state.unwrap_or(defaults.state),
        socket: if env.socket.as_os_str().is_empty() {
            defaults.socket
        } else {
            env.socket
        },
    };
    if let Some(h) = &env.herdr {
        std::env::set_var("HERDR_BIN_PATH", h);
    }
    let text = read().unwrap_or_default();
    let group = managed(&text).into_iter().find(|m| m.id == id);
    let shell = group
        .as_ref()
        .map(|m| m.shell.clone())
        .filter(|s| !s.contains("/bin/cron/"))
        .unwrap_or_else(|| "/bin/sh".into());
    let approved = crate::routine::valid_id(&id)
        .then(|| Approvals::load(&paths).routine(&id))
        .flatten();
    let Some(routine) = approved else {
        // Without the plugin's side (no routine the owner approved): just
        // the command, as cron would.
        return Command::new(&shell)
            .arg("-c")
            .arg(cmd)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(1);
    };
    if crate::scheduler::paused(&paths) {
        paths::log(
            &paths,
            &format!("{id}: cron time skipped, routines are off"),
        );
        return 0;
    }
    let manual = std::env::var("HERDR_ROUTINES_TRIGGER").as_deref() == Ok("manual");
    // A single run on a date: cron has no year, so its line comes round
    // every year on that day. It runs only on the date itself (as long
    // as the line is still the one written for that date; a line edited
    // by hand runs as edited).
    let once = match routine.schedule() {
        Some(Schedule::Once(t)) if !manual => Some(t),
        _ => None,
    };
    if let Some(t) = once {
        let as_written = group
            .as_ref()
            .is_none_or(|m| when(&routine).as_deref() == Some(m.when.as_str()));
        if as_written && t.date_naive() != chrono::Local::now().date_naive() {
            paths::log(
                &paths,
                &format!("{id}: not run, it is set for {} only", t.format("%Y-%m-%d")),
            );
            return 0;
        }
    }
    // Bounds also for a routine approved before they were checked.
    let retries = routine.file.retries.min(crate::routine::MAX_RETRIES);
    let retry_delay = routine
        .file
        .retry_delay_minutes
        .clamp(1, crate::routine::MAX_MINUTES);
    let pid = std::process::id();
    let started = crate::scheduler::started_at(pid).unwrap_or_default();
    let mut trigger = if manual {
        Trigger::Manual
    } else {
        Trigger::Scheduled
    };
    let mut attempt = 0;
    let mut code;
    loop {
        let run = crate::scheduler::with_lock(&paths, &id, || {
            if crate::scheduler::busy(&paths, &id) {
                let mut skipped = Run::new(&id, &routine.file.name, SHELL, trigger, None);
                skipped.cron_pid = pid;
                skipped.cron_started = started.clone();
                skipped.tab_closed = true;
                skipped.finish(Status::Skipped, "The previous run was still going");
                let _ = skipped.save(&paths);
                return None;
            }
            let mut run = Run::new(&id, &routine.file.name, SHELL, trigger, None);
            run.cron_pid = pid;
            run.cron_started = started.clone();
            run.attempt = attempt;
            run.status = Status::Running;
            run.tab_closed = true;
            let _ = run.save(&paths);
            Some(run)
        });
        let Some(mut run) = run else { return 0 };
        let limit_minutes = routine.file.time_limit_minutes.max(1);
        let limit = std::time::Duration::from_secs(u64::from(limit_minutes) * 60);
        // The output goes into the run's file and on to cron as it comes.
        let tee = || Tee {
            file: paths::create_private_file(&run.file(&paths, "out")).ok(),
        };
        // What cron feeds the command (the text after a `%` in its line)
        // is there once: the first try gets it.
        let input = if attempt == 0 {
            std::process::Stdio::inherit()
        } else {
            std::process::Stdio::null()
        };
        let (tail, ended) =
            match crate::shell::run_as_cron(&shell, cmd, limit, input, Some(Box::new(tee()))) {
                Some(e) => (e.tail, e.code),
                None => {
                    let text = format!("Could not start {shell}.\n");
                    let _ = std::fs::write(run.file(&paths, "out"), &text);
                    print!("{text}");
                    (text, Some(127))
                }
            };
        run.result = crate::runner::last_line(&tail);
        let failed = match ended {
            Some(0) => {
                run.finish(Status::Finished, "");
                code = 0;
                false
            }
            Some(c) => {
                run.finish(Status::Failed, format!("Ended with exit code {c}"));
                code = c;
                true
            }
            None => {
                let minutes = if limit_minutes == 1 {
                    "1 minute".to_string()
                } else {
                    format!("{limit_minutes} minutes")
                };
                run.finish(
                    Status::TimedOut,
                    format!("Stopped after {minutes}, its time limit"),
                );
                code = 124;
                true
            }
        };
        let _ = run.save(&paths);
        crate::store::prune(&paths, &id, routine.file.keep_runs, routine.file.keep_days);
        let retry = failed && run.status == Status::Failed && attempt < retries;
        if failed && !retry {
            crate::alert::tell(
                &paths,
                &run,
                true,
                &format!("{} failed", run.routine_name),
                &format!(
                    "The cron job: {}. l on the run in herdr-routines shows its output.",
                    run.message.to_lowercase()
                ),
                true,
            );
        } else if !failed {
            let settings = crate::routine::Settings::load(&paths);
            crate::alert::tell(
                &paths,
                &run,
                false,
                &format!("{} finished", run.routine_name),
                "The cron job ended without an error. Its output is in herdr-routines.",
                settings.notify_finished,
            );
        }
        if retry {
            std::thread::sleep(std::time::Duration::from_secs(u64::from(retry_delay) * 60));
            attempt += 1;
            trigger = Trigger::Retry;
            continue;
        }
        break;
    }
    // A single run on a date switches its cron job off: the routine as it
    // was approved, saved switched off (as `space` would), when its files
    // are still the approved ones. Files that changed meanwhile wait for
    // the owner and are neither overwritten nor approved here; then only
    // the crontab line is switched off.
    if once.is_some() {
        let unchanged = Routine::load(&paths, &id).is_ok_and(|now| now.approved);
        let mut done = routine.clone();
        done.file.enabled = false;
        if unchanged && done.save_approved(&paths).is_ok() {
            let _ = write_job(&paths, &done);
        } else {
            let _ = switch(&paths, &id, false);
        }
    }
    code
}

/// Where a cron job's output goes as it comes: into the run's output file
/// and on to cron, which mails or drops it as it would without the plugin.
/// Nothing of it is held back, however much it is.
struct Tee {
    file: Option<std::fs::File>,
}

impl std::io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Neither side stops the other: a full disk still lets cron have
        // the output, and a closed pipe to cron still fills the file.
        if self
            .file
            .as_mut()
            .is_some_and(|f| f.write_all(buf).is_err())
        {
            self.file = None;
        }
        let _ = std::io::stdout().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stdout().flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB: &str = "# my jobs\nMAILTO=me@example.com\nPATH=/usr/local/bin:/usr/bin\n\n15 2 * * *   /home/user/bin/backup.sh --all >> /tmp/b.log 2>&1\n@reboot sleep 30 && ~/start.sh\n*/5 * * * * date +\\%H\\%M\n# moved to herdr-routines (routine x): 0 1 * * * old\nbroken line\n";

    #[test]
    fn reads_jobs_and_their_variables() {
        let jobs = parse(TAB);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].schedule, "15 2 * * *");
        assert_eq!(
            jobs[0].command,
            "/home/user/bin/backup.sh --all >> /tmp/b.log 2>&1"
        );
        assert_eq!(
            jobs[0].env,
            vec![("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string())]
        );
        assert_eq!(jobs[1].schedule, "@start");
        assert_eq!(jobs[2].command, "date +%H%M");
        assert!(jobs[2].note.is_none());
        assert!(parse("* * * * * printf a%b")
            .first()
            .unwrap()
            .note
            .is_some());
        assert_eq!(name_for(&jobs[0].command), "backup.sh --all");
        assert_eq!(name_for("sleep 30 && ~/start.sh"), "sleep 30");
    }

    #[test]
    fn routines_and_commented_lines() {
        let jobs = parse(TAB);
        let (file, script) = routine_for(&jobs[0], true);
        assert!(file.enabled && file.agent == SHELL && file.missed == Missed::Skip);
        assert!(script.contains("export PATH='/usr/local/bin:/usr/bin'\n"));
        assert!(script.ends_with("2>&1\n"));
        let moved = vec![(jobs[0].line.clone(), "backup-sh-all".to_string())];
        let next = without(TAB, &moved);
        assert!(next.contains(
            "# moved to herdr-routines (routine backup-sh-all): 15 2 * * *   /home/user/bin/backup.sh"
        ));
        assert_eq!(parse(&next).len(), 2);
    }

    #[test]
    fn managed_jobs_are_groups_around_their_own_line() {
        let text = "SHELL=/bin/bash\n0 1 * * * a\n# herdr-routines: say-hi (x)\nSHELL=/s/bin/cron/say-hi\n*/5 * * * * echo \"$(date +\\%H)\" >> /tmp/x\nSHELL=/bin/bash\n#off 0 3 * * * not ours\n# herdr-routines: off-one (x)\nSHELL=/s/bin/cron/off-one\n#off @daily b\nSHELL=/bin/bash\n";
        let m = managed(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "say-hi");
        assert!(m[0].enabled);
        assert_eq!(m[0].when, "*/5 * * * *");
        assert_eq!(m[0].command, "echo \"$(date +%H)\" >> /tmp/x");
        assert_eq!(m[0].shell, "/bin/bash");
        assert_eq!((m[0].start, m[0].len), (2, 4));
        assert!(!m[1].enabled && m[1].when == "@daily" && m[1].schedule == "0 0 * * *");
        // Others' jobs only; the groups' SHELL lines do not leak.
        let others = parse(text);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].command, "a");
        assert_eq!(shell_before(text, text.lines().count()), "/bin/bash");
    }

    #[test]
    fn the_wrapper_is_recognised_by_its_place() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(wrapper_call(&args(&["/s/bin/cron/say-hi", "-c", "echo hi"])).is_some());
        assert!(wrapper_call(&args(&["/s/bin/herdr-routines", "-c", "x"])).is_none());
        assert!(wrapper_call(&args(&["/s/bin/cron/x", "list"])).is_none());
        // A command that is not UTF-8 arrives as it is.
        use std::os::unix::ffi::OsStringExt;
        let command = std::ffi::OsString::from_vec(b"echo caf\xe9".to_vec());
        let args = ["/s/bin/cron/x".into(), "-c".into(), command.clone()];
        assert_eq!(wrapper_call(&args).unwrap().1, command);
    }

    /// Made-up folders and a stand-in for `crontab` on this thread: `-l`
    /// runs `on_list` and prints the file, a path replaces it. Returns the
    /// folders and the file.
    fn stand_in(tag: &str, text: &[u8], on_list: &str) -> (Paths, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "herdr-routines-crontab-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        };
        std::fs::create_dir_all(&paths.state).unwrap();
        let tab = dir.join("tab");
        std::fs::write(&tab, text).unwrap();
        let fake = dir.join("crontab");
        let script = format!(
            "#!/bin/sh\ncd '{}'\nif [ \"$1\" = -l ]; then cat tab; {on_list}\nelse cp \"$1\" tab; fi\n",
            dir.display()
        );
        std::fs::write(&fake, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        use_fake(Some(&fake));
        (paths, tab)
    }

    /// An approved cron job routine, saved.
    fn job(paths: &Paths, id: &str, schedule: &str, command: &str, enabled: bool) -> Routine {
        let mut file = cron_file(id.to_string(), schedule.to_string(), enabled);
        file.runs_in = RunsIn::Cron;
        let r = Routine {
            id: id.to_string(),
            file,
            prompt: command.to_string(),
            problem: None,
            unreadable: false,
            approved: true,
        };
        r.save_approved(paths).unwrap();
        r
    }

    #[test]
    fn bytes_that_are_not_text_survive_every_change() {
        // Latin-1 in a comment and in a command, and Windows line ends.
        let before: &[u8] =
            b"# caf\xe9 jobs\r\nMAILTO=\"\"\r\n*/5 * * * *   /opt/caf\xe9/run --all\r\n0 1 * * * other\n";
        let (paths, tab) = stand_in("bytes", before, ":");
        let now = || std::fs::read(&tab).unwrap();
        let has = |part: &[u8]| now().windows(part.len()).any(|w| w == part);
        // Managed where it is: the line keeps its bytes and its spacing.
        let jobs = parse(&read().unwrap());
        assert_eq!(jobs.len(), 2);
        keep(&paths, &jobs[0]).unwrap();
        let line: &[u8] = b"\n*/5 * * * *   /opt/caf\xe9/run --all\r\n";
        assert!(has(line), "{:?}", String::from_utf8_lossy(&now()));
        let id = managed(&read().unwrap())[0].id.clone();
        // Off and on again, by the key and by a saved routine.
        switch(&paths, &id, false).unwrap();
        assert!(has(b"\n#off */5 * * * *   /opt/caf\xe9/run --all\r\n"));
        assert!(!managed(&read().unwrap())[0].enabled);
        switch(&paths, &id, true).unwrap();
        assert!(has(line));
        pull(&paths).unwrap();
        let mut r = Routine::load(&paths, &id).unwrap();
        assert!(r.approved && r.problem.is_none(), "{:?}", r.problem);
        r.file.enabled = false;
        r.file.name = "Caf run".into();
        r.save_approved(&paths).unwrap();
        write_job(&paths, &r).unwrap();
        assert!(has(b"\n#off */5 * * * *   /opt/caf\xe9/run --all\r\n"));
        // Another job is added and removed: the rest stays byte for byte.
        let other = job(&paths, "say-hi", "0 9 * * *", "echo 100%", true);
        write_job(&paths, &other).unwrap();
        assert!(has(b"\n0 9 * * * echo 100\\%\nSHELL=/bin/sh\n"));
        assert!(has(b"# caf\xe9 jobs\r\nMAILTO=\"\"\r\n"));
        remove_job(&paths, "say-hi", false).unwrap();
        r.file.enabled = true;
        r.save_approved(&paths).unwrap();
        write_job(&paths, &r).unwrap();
        // Handed back to cron: the crontab is what it was at the start
        // (but for the empty line a new group leaves behind).
        assert_eq!(release_all(&paths).unwrap(), 1);
        assert_eq!(now(), [before, b"\n"].concat());
        assert!(!wrapper(&paths, &id).exists());
        use_fake(None);
    }

    #[test]
    fn a_routine_that_waits_for_approval_is_never_written() {
        let (paths, tab) = stand_in("waiting", b"0 1 * * * other\n", ":");
        let mut r = job(&paths, "say-hi", "0 9 * * *", "echo hi", true);
        write_job(&paths, &r).unwrap();
        let approved = std::fs::read(&tab).unwrap();
        // As `Routine::load` hands over one whose files changed.
        r.prompt = "echo something else".into();
        r.file.schedule = "* * * * *".into();
        r.approved = false;
        write_job(&paths, &r).unwrap();
        assert_eq!(std::fs::read(&tab).unwrap(), approved);
        // Switching it changes on or off and nothing else.
        switch(&paths, "say-hi", false).unwrap();
        let off = String::from_utf8(std::fs::read(&tab).unwrap()).unwrap();
        assert!(off.contains("\n#off 0 9 * * * echo hi\n"), "{off}");
        switch(&paths, "say-hi", false).unwrap();
        assert_eq!(
            String::from_utf8(std::fs::read(&tab).unwrap()).unwrap(),
            off
        );
        switch(&paths, "say-hi", true).unwrap();
        assert_eq!(std::fs::read(&tab).unwrap(), approved);
        switch(&paths, "no-such-job", false).unwrap();
        assert_eq!(std::fs::read(&tab).unwrap(), approved);
        use_fake(None);
    }

    #[test]
    fn a_crontab_that_changes_meanwhile_is_not_overwritten() {
        // Something else edits the crontab once, right after it was read.
        let edit_once =
            "if [ ! -e edited ]; then echo '0 2 * * * by hand' >> tab; touch edited; fi";
        let (paths, tab) = stand_in("race", b"0 1 * * * other\n", edit_once);
        let r = job(&paths, "say-hi", "0 9 * * *", "echo hi", true);
        write_job(&paths, &r).unwrap();
        let now = std::fs::read_to_string(&tab).unwrap();
        assert!(
            now.starts_with("0 1 * * * other\n0 2 * * * by hand\n"),
            "{now}"
        );
        assert!(now.contains("\n0 9 * * * echo hi\n"), "{now}");
        // One that changes on every read is left alone, with an error.
        let (paths, tab) = stand_in("race-always", b"0 1 * * * other\n", "echo '# again' >> tab");
        let r = job(&paths, "say-hi", "0 9 * * *", "echo hi", true);
        let e = write_job(&paths, &r).unwrap_err();
        assert!(e.contains("kept changing"), "{e}");
        assert!(!std::fs::read_to_string(&tab).unwrap().contains("echo hi"));
        use_fake(None);
    }

    #[test]
    fn the_first_backup_stays_besides_the_last_five() {
        let first = "0 1 * * * as it was before the plugin\n";
        let (paths, _tab) = stand_in("backups", first.as_bytes(), ":");
        let r = job(&paths, "say-hi", "0 9 * * *", "echo hi", true);
        write_job(&paths, &r).unwrap();
        for n in 0..8 {
            switch(&paths, "say-hi", n % 2 == 1).unwrap();
            // The backups are named by the time, to the millisecond.
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let dir = paths.state.join("crontab-backups");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), BACKUPS + 1, "{names:?}");
        assert!(names.contains(&FIRST_BACKUP.to_string()));
        assert_eq!(
            std::fs::read_to_string(dir.join(FIRST_BACKUP)).unwrap(),
            first
        );
        use_fake(None);
    }

    #[test]
    fn managed_jobs_are_handed_back_as_plain_lines() {
        let (paths, tab) = stand_in("release", b"SHELL=/bin/bash\n0 1 * * * other\n", ":");
        write_job(&paths, &job(&paths, "say-hi", "0 9 * * *", "echo hi", true)).unwrap();
        write_job(&paths, &job(&paths, "say-bye", "@start", "echo bye", false)).unwrap();
        assert_eq!(managed(&read().unwrap()).len(), 2);
        assert!(wrapper(&paths, "say-hi").exists());
        assert_eq!(release_all(&paths).unwrap(), 2);
        // No marker and no wrapper are left: cron runs the lines itself,
        // with the shell the crontab had; the one that was off stays off.
        assert_eq!(
            std::fs::read_to_string(&tab).unwrap(),
            "SHELL=/bin/bash\n0 1 * * * other\n\n0 9 * * * echo hi\n\n# @reboot echo bye\n"
        );
        assert!(!wrapper(&paths, "say-hi").exists() && !wrapper(&paths, "say-bye").exists());
        assert_eq!(release_all(&paths).unwrap(), 0);
        use_fake(None);
    }
}
