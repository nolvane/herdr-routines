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

use crate::paths::{self, Paths};
use crate::routine::{Missed, Routine, RoutineFile, RunsIn, SHELL};
use crate::schedule::{Schedule, AT_START};
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The crontab, empty when there is none.
pub fn read() -> Result<String, String> {
    let Some(program) = program() else {
        return Ok(String::new());
    };
    let out = Command::new(program)
        .arg("-l")
        .output()
        .map_err(|e| format!("cannot run crontab: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("no crontab") {
        Ok(String::new())
    } else {
        Err(err.trim().to_string())
    }
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

/// The crontab with the moved jobs' lines commented out.
pub fn without(text: &str, moved: &[(String, String)]) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match moved.iter().find(|(l, _)| l == line) {
            Some((_, id)) => out.push(format!("{MOVED} (routine {id}): {line}")),
            None => out.push(line.to_string()),
        }
    }
    out.join("\n") + "\n"
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
        // Read again: only lines still there are changed.
        let now = read()?;
        install(paths, &now, &without(&now, &moved)).map_err(|e| {
            format!("the routines were added, but {e}; comment the lines out by hand")
        })?;
    }
    Ok(moved.len())
}

/// Manages a job of the crontab where it is: a routine for its name and
/// settings, and the marker and wrapper around its line.
fn keep(paths: &Paths, job: &CronJob) -> Result<(), String> {
    let now = read()?;
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
    let lines: Vec<&str> = now.lines().collect();
    let skip: Vec<std::ops::Range<usize>> = managed(&now)
        .iter()
        .map(|m| m.start..m.start + m.len)
        .collect();
    let Some(at) = lines
        .iter()
        .enumerate()
        .position(|(n, l)| *l == job.line && !skip.iter().any(|r| r.contains(&n)))
    else {
        return Err("the job is no longer in the crontab".into());
    };
    let shell = shell_before(&now, at);
    // The line itself stays as written; only the marker and SHELL go around.
    let group = vec![
        marker(&id),
        format!("SHELL={}", wrapper(paths, &id).display()),
        job.line.clone(),
        format!("SHELL={shell}"),
    ];
    let mut next: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    next.splice(at..at + 1, group);
    r.save_approved(paths)?;
    prepare_wrapper(paths, &id)?;
    install(paths, &now, &(next.join("\n") + "\n"))
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
pub fn write_job(paths: &Paths, r: &Routine) -> Result<(), String> {
    let now = read()?;
    let mut lines: Vec<String> = now.lines().map(str::to_string).collect();
    match managed(&now).into_iter().find(|m| m.id == r.id) {
        Some(m) => {
            let g = group(paths, r, &m.shell)?;
            lines.splice(m.start..m.start + m.len, g);
        }
        None => {
            let g = group(paths, r, &shell_before(&now, lines.len()))?;
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.extend(g);
        }
    }
    let next = lines.join("\n") + "\n";
    if next == now {
        return Ok(());
    }
    prepare_wrapper(paths, &r.id)?;
    install(paths, &now, &next)
}

/// Takes a managed job out of the crontab. `keep_line`: its line stays as
/// an ordinary job (not managed any more); otherwise it goes.
pub fn remove_job(paths: &Paths, id: &str, keep_line: bool) -> Result<(), String> {
    let now = read()?;
    let Some(m) = managed(&now).into_iter().find(|m| m.id == id) else {
        return Ok(());
    };
    let mut lines: Vec<String> = now.lines().map(str::to_string).collect();
    let job: Vec<String> = lines[m.start..m.start + m.len]
        .iter()
        .find(|l| split_job(l.trim().strip_prefix(OFF).unwrap_or(l.trim())).is_some())
        .map(|l| {
            if m.enabled {
                vec![l.trim().to_string()]
            } else {
                vec![format!("# {}", l.trim().trim_start_matches(OFF))]
            }
        })
        .filter(|_| keep_line)
        .unwrap_or_default();
    lines.splice(m.start..m.start + m.len, job);
    let _ = std::fs::remove_file(wrapper(paths, id));
    install(paths, &now, &(lines.join("\n") + "\n"))
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

/// Replaces the crontab with `text`, after a backup of the current one in
/// the state folder (the last five are kept).
fn install(paths: &Paths, now: &str, text: &str) -> Result<(), String> {
    let program = program().ok_or("there is no crontab here")?;
    let dir = paths.state.join("crontab-backups");
    paths::create_private_dir(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    paths::write_atomic(&dir.join(format!("crontab-{stamp}.txt")), now.as_bytes())
        .map_err(|e| format!("cannot back up the crontab: {e}"))?;
    let mut old: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    old.sort();
    let extra = old.len().saturating_sub(5);
    for p in old.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
    let next = paths.state.join("crontab-new.txt");
    paths::write_atomic(&next, text.as_bytes()).map_err(|e| e.to_string())?;
    let out = Command::new(program)
        .arg(&next)
        .output()
        .map_err(|e| format!("cannot run crontab: {e}"));
    let _ = std::fs::remove_file(&next);
    let out = out?;
    if out.status.success() {
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
pub fn wrapper_call(args: &[String]) -> Option<(PathBuf, String)> {
    if args.len() < 3 || args[1] != "-c" {
        return None;
    }
    let me = PathBuf::from(&args[0]);
    (me.parent()?.file_name()? == "cron").then(|| (me, args[2].clone()))
}

/// Starts a managed cron job now, as cron would (`r` in the list): its
/// wrapper with its command, in the background.
pub fn run_now(paths: &Paths, r: &Routine) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
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
pub fn wrapper_main(me: &Path, cmd: &str) -> i32 {
    use crate::store::{Run, Status, Trigger};
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
    let shell = managed(&text)
        .into_iter()
        .find(|m| m.id == id)
        .map(|m| m.shell)
        .filter(|s| !s.contains("/bin/cron/"))
        .unwrap_or_else(|| "/bin/sh".into());
    let Ok(routine) = Routine::load(&paths, &id) else {
        // Without the plugin's side: just the command, as cron would.
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
    let pid = std::process::id();
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
                skipped.tab_closed = true;
                skipped.finish(Status::Skipped, "The previous run was still going");
                let _ = skipped.save(&paths);
                return None;
            }
            let mut run = Run::new(&id, &routine.file.name, SHELL, trigger, None);
            run.cron_pid = pid;
            run.attempt = attempt;
            run.status = Status::Running;
            run.tab_closed = true;
            let _ = run.save(&paths);
            Some(run)
        });
        let Some(mut run) = run else { return 0 };
        let limit_minutes = routine.file.time_limit_minutes.max(1);
        let limit = std::time::Duration::from_secs(u64::from(limit_minutes) * 60);
        let (output, ended) = match crate::shell::run_as_cron(&shell, cmd, limit) {
            Some(e) => (e.output, e.code),
            None => (format!("Could not start {shell}.\n"), Some(127)),
        };
        // Cron mails or drops what a job prints: hand it on.
        print!("{output}");
        let _ = paths::write_atomic(&run.file(&paths, "out"), output.as_bytes());
        run.result = crate::runner::last_line(&output);
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
        let retry = failed && run.status == Status::Failed && attempt < routine.file.retries;
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
            std::thread::sleep(std::time::Duration::from_secs(
                u64::from(routine.file.retry_delay_minutes) * 60,
            ));
            attempt += 1;
            trigger = Trigger::Retry;
            continue;
        }
        break;
    }
    // A single run on a date switches its cron job off.
    if !manual && matches!(routine.schedule(), Some(Schedule::Once(_))) {
        if let Ok(mut done) = Routine::load(&paths, &id) {
            done.file.enabled = false;
            if done.save_approved(&paths).is_ok() {
                let _ = write_job(&paths, &done);
            }
        }
    }
    code
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
    }
}
