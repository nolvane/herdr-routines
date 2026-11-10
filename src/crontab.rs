//! The person's existing cron jobs (`crontab -l`): shown in the list and
//! taken over as shell routines. Moving comments their lines out of the
//! crontab (after a backup), so nothing runs twice; copying leaves the
//! crontab alone and adds the routines switched off.

use crate::paths::{self, Paths};
use crate::routine::{Missed, Routine, RoutineFile, SHELL};
use crate::schedule::{Schedule, AT_START};
use std::process::Command;

/// The program to call; `HERDR_ROUTINES_CRONTAB` replaces it in tests.
fn program() -> String {
    std::env::var("HERDR_ROUTINES_CRONTAB").unwrap_or_else(|_| "crontab".into())
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
    let out = Command::new(program())
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

/// The jobs in a crontab, in order. Lines it cannot read are left out.
pub fn parse(text: &str) -> Vec<CronJob> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut jobs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
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

/// Takes over the jobs as routines. `moving`: on, and their crontab lines
/// commented out (a backup of the crontab goes to the state folder first);
/// otherwise off, and the crontab stays. Returns how many were added.
pub fn import(paths: &Paths, jobs: &[CronJob], moving: bool) -> Result<usize, String> {
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
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let backup = paths.state.join(format!("crontab-backup-{stamp}.txt"));
        paths::write_atomic(&backup, now.as_bytes())
            .map_err(|e| format!("cannot back up the crontab: {e}"))?;
        let next = paths.state.join("crontab-new.txt");
        paths::write_atomic(&next, without(&now, &moved).as_bytes()).map_err(|e| e.to_string())?;
        let out = Command::new(program())
            .arg(&next)
            .output()
            .map_err(|e| format!("cannot run crontab: {e}"))?;
        let _ = std::fs::remove_file(&next);
        if !out.status.success() {
            return Err(format!(
                "the routines were added, but the crontab could not be changed ({}); comment the lines out by hand",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(moved.len())
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
