//! The person's existing cron jobs (`crontab -l`): shown in the list and
//! taken over as shell routines. Moving comments their lines out of the
//! crontab (after a backup), so nothing runs twice; copying leaves the
//! crontab alone and adds the routines switched off.

use crate::paths::{self, Paths};
use crate::routine::{Missed, Routine, RoutineFile, SHELL};
use crate::schedule::{Schedule, AT_START};
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

/// The jobs in a crontab, in order. Lines it cannot read are left out, and
/// so is the block of this plugin's own cron jobs.
pub fn parse(text: &str) -> Vec<CronJob> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut jobs = Vec::new();
    let mut ours = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with(BEGIN) {
            ours = true;
            continue;
        }
        if line.starts_with(END) {
            ours = false;
            continue;
        }
        if ours || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let looks_like_job =
            line.starts_with('@') || line.starts_with(|c: char| c.is_ascii_digit() || c == '*');
        if !looks_like_job {
            if let Some((k, v)) = env_assignment(line) {
                env.retain(|(name, _)| *name != k);
                if k != "MAILTO" {
                    env.push((k, v));
                }
            }
            continue;
        }
        let (schedule, command) = if let Some(rest) = line.strip_prefix('@') {
            let (word, command) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let schedule = match word {
                "reboot" => AT_START,
                "yearly" | "annually" => "0 0 1 1 *",
                "monthly" => "0 0 1 * *",
                "weekly" => "0 0 * * 0",
                "daily" | "midnight" => "0 0 * * *",
                "hourly" => "0 * * * *",
                _ => continue,
            };
            (schedule.to_string(), command.trim().to_string())
        } else {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                continue;
            }
            // The command keeps its own spacing: cut after the fifth field.
            let mut rest = line;
            for _ in 0..5 {
                rest = rest.trim_start();
                rest = &rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..];
            }
            (parts[..5].join(" "), rest.trim().to_string())
        };
        if command.is_empty() || Schedule::parse(&schedule).is_err() {
            continue;
        }
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

/// The routine a job becomes: a shell command in the home folder (where
/// cron runs it), with its crontab variables, skipping missed times as cron
/// does.
pub fn routine_for(job: &CronJob, enabled: bool) -> (RoutineFile, String) {
    let mut script = format!("# From the crontab: {}\n", job.line.trim());
    for (k, v) in &job.env {
        script.push_str(&format!("export {k}={}\n", quote(v)));
    }
    script.push_str(&job.command);
    script.push('\n');
    let file = RoutineFile {
        name: name_for(&job.command),
        enabled,
        schedule: job.schedule.clone(),
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
        runs_in: Default::default(),
    };
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
    /// Still a cron job, now one of this plugin's (shown, edited and
    /// recorded here); its old line is commented out.
    KeepCron,
    /// A routine that runs in Herdr, switched off; the crontab stays.
    Copy,
}

/// Takes over the jobs as routines (see `Take`). Commenting lines out
/// keeps a backup of the crontab in the state folder. Returns how many
/// were added.
pub fn import(paths: &Paths, jobs: &[CronJob], how: Take) -> Result<usize, String> {
    let moving = how != Take::Copy;
    let mut moved: Vec<(String, String)> = Vec::new();
    for job in jobs {
        let (mut file, script) = routine_for(job, moving);
        if how == Take::KeepCron {
            file.runs_in = crate::routine::RunsIn::Cron;
        }
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
    if how == Take::KeepCron {
        sync(paths)?;
    }
    Ok(moved.len())
}

// ---- this plugin's own cron jobs -----------------------------------------
//
// A routine with `runs_in = "cron"` gets one line in a marked block of the
// crontab. The line starts `herdr-routines cron-run <id>` from a copy of the
// plugin binary in the state folder (the plugin's own folder changes with
// each update), which runs the routine's command and records the run like
// any other. The block is written again whenever such a routine changes.

const BEGIN: &str = "# BEGIN herdr-routines";
const END: &str = "# END herdr-routines";

/// The copy of the plugin binary the crontab calls.
pub fn binary(paths: &Paths) -> std::path::PathBuf {
    paths.state.join("bin").join("herdr-routines")
}

/// Puts the running binary where the crontab calls it, when it differs.
fn place_binary(paths: &Paths) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = binary(paths);
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

/// A path for a crontab line: in single quotes, refused when it holds a
/// character the shell or cron would read differently.
fn cron_quote(p: &std::path::Path) -> Result<String, String> {
    let s = p.to_string_lossy();
    if s.contains(['\'', '\\', '%', '\n', '\r']) {
        return Err(format!("the path {s} has a character cron cannot take"));
    }
    Ok(format!("'{s}'"))
}

/// When cron starts it: five cron fields, or `@reboot`.
fn when(r: &Routine) -> Option<String> {
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
        Schedule::Cron(_) => Some(r.file.schedule.trim().to_string()),
    }
}

/// This plugin's block for the crontab: one line per cron job that is on.
pub fn block(paths: &Paths, routines: &[Routine]) -> Result<String, String> {
    let jobs: Vec<&Routine> = routines
        .iter()
        .filter(|r| r.is_cron() && r.file.enabled && !r.unreadable)
        .collect();
    if jobs.is_empty() {
        return Ok(String::new());
    }
    let mut env = format!(
        "HERDR_PLUGIN_CONFIG_DIR={} HERDR_PLUGIN_STATE_DIR={} HERDR_SOCKET_PATH={}",
        cron_quote(&paths.config)?,
        cron_quote(&paths.state)?,
        cron_quote(&paths.socket)?
    );
    let herdr = crate::herdr::bin();
    if herdr.is_absolute() {
        env.push_str(&format!(" HERDR_BIN_PATH={}", cron_quote(&herdr)?));
    }
    let bin = cron_quote(&binary(paths))?;
    let mut out =
        format!("{BEGIN}: cron jobs of the herdr-routines plugin; change them in its list\n");
    for r in jobs {
        let Some(when) = when(r) else { continue };
        let name = crate::app::visible_text(&r.file.name).replace(['\n', '\r'], " ");
        out.push_str(&format!("# {name}\n{when} {env} {bin} cron-run {}\n", r.id));
    }
    out.push_str(END);
    out.push('\n');
    Ok(out)
}

/// The crontab with this plugin's block replaced by `block` (removed when
/// empty; added at the end when there was none).
pub fn with_block(text: &str, block: &str) -> String {
    let mut out = String::new();
    let mut ours = false;
    let mut placed = false;
    for line in text.lines() {
        if line.trim().starts_with(BEGIN) {
            ours = true;
            if !placed {
                out.push_str(block);
                placed = true;
            }
            continue;
        }
        if ours {
            if line.trim().starts_with(END) {
                ours = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !placed && !block.is_empty() {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(block);
    }
    out
}

/// Replaces the crontab with `text`, after a backup of the current one in
/// the state folder (the last five are kept).
fn install(paths: &Paths, now: &str, text: &str) -> Result<(), String> {
    let program = program().ok_or("there is no crontab here")?;
    let dir = paths.state.join("crontab-backups");
    paths::create_private_dir(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
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

/// Brings the crontab in line with the cron jobs among the routines.
/// Returns whether it changed. Touches nothing when there is nothing to
/// change, so a person without cron jobs never gets a crontab.
pub fn sync(paths: &Paths) -> Result<bool, String> {
    let routines = crate::routine::load_all(paths);
    let block = block(paths, &routines)?;
    let now = read()?;
    if block.is_empty() && !now.lines().any(|l| l.trim().starts_with(BEGIN)) {
        return Ok(false);
    }
    let next = with_block(&now, &block);
    if next == now {
        return Ok(false);
    }
    if !block.is_empty() {
        place_binary(paths)?;
    }
    install(paths, &now, &next)?;
    Ok(true)
}

/// `herdr-routines cron-run <id>`, from the crontab: runs the cron job's
/// command in the login shell, in its folder, and records the run (output,
/// result, status), with its retries and alert.
pub fn run_job(paths: &Paths, id: &str) -> i32 {
    use crate::store::{Run, Status, Trigger};
    let Ok(routine) = Routine::load(paths, id) else {
        return 2;
    };
    // All routines switched off in the list: cron jobs skip their times.
    if crate::scheduler::paused(paths) {
        paths::log(paths, &format!("{id}: cron time skipped, routines are off"));
        return 0;
    }
    // A line left over from before a change: the routine decides.
    if !routine.is_cron() || !routine.file.enabled {
        let _ = sync(paths);
        return 0;
    }
    let pid = std::process::id();
    let mut trigger = Trigger::Scheduled;
    let mut attempt = 0;
    loop {
        let run = crate::scheduler::with_lock(paths, id, || {
            if crate::scheduler::busy(paths, id) {
                let mut skipped = Run::new(id, &routine.file.name, SHELL, trigger, None);
                skipped.cron_pid = pid;
                skipped.finish(Status::Skipped, "The previous run was still going");
                let _ = skipped.save(paths);
                return None;
            }
            let mut run = Run::new(id, &routine.file.name, SHELL, trigger, None);
            run.cron_pid = pid;
            run.attempt = attempt;
            run.status = Status::Running;
            run.tab_closed = true;
            let _ = run.save(paths);
            Some(run)
        });
        let Some(mut run) = run else { return 0 };
        if !routine.can_run() {
            let why = match &routine.problem {
                Some(p) => format!("Cannot run: {p}"),
                None => "Waits for your approval: it changed outside the list".into(),
            };
            run.finish(Status::Failed, why);
            let _ = run.save(paths);
            crate::alert::tell(
                paths,
                &run,
                true,
                &format!("{} did not run", run.routine_name),
                "The cron job changed outside the list. Open herdr-routines and approve it with a.",
                true,
            );
            return 1;
        }
        let folder = paths::expand(&routine.file.folder);
        let limit_minutes = routine.file.time_limit_minutes.max(1);
        let limit = std::time::Duration::from_secs(u64::from(limit_minutes) * 60);
        let ended = crate::shell::run_full(&routine.prompt, Some(&folder), limit);
        let (output, code) = match ended {
            Some(e) => (e.output, e.code),
            None => (String::from("Could not start the login shell.\n"), Some(-1)),
        };
        let _ = paths::write_atomic(&run.file(paths, "out"), output.as_bytes());
        run.result = crate::runner::last_line(&output);
        let failed = match code {
            Some(0) => {
                run.finish(Status::Finished, "");
                false
            }
            Some(c) => {
                run.finish(Status::Failed, format!("Ended with exit code {c}"));
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
                true
            }
        };
        let _ = run.save(paths);
        crate::store::prune(paths, id, crate::routine::Settings::load(paths).history);
        let retry = failed && run.status == Status::Failed && attempt < routine.file.retries;
        if failed && !retry {
            crate::alert::tell(
                paths,
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
            let settings = crate::routine::Settings::load(paths);
            crate::alert::tell(
                paths,
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
    // A single run switches its cron job off, as the scheduler does.
    if matches!(routine.schedule(), Some(Schedule::Once(_))) {
        if let Ok(mut done) = Routine::load(paths, id) {
            if done.approved {
                done.file.enabled = false;
                let _ = done.save_approved(paths);
                let _ = sync(paths);
            }
        }
    }
    0
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
}
