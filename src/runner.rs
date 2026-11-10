//! Starting a run and following it until it is done.
//!
//! A run gets its own tab in the routines workspace. Herdr starts the agent
//! there and the prompt goes in as if typed, so a person can watch the run
//! and step in. The scheduler then reads the agent's state from Herdr:
//! `working` means running, `blocked` means it asks something, `idle` or
//! `done` after having worked means finished. A shell routine runs its
//! command in the tab instead and is done when the command ends.

use crate::herdr::{self, Place};
use crate::paths::{self, Paths};
use crate::routine::{Routine, Settings, SHELL};
use crate::store::{Run, Status, Trigger};
use chrono::{DateTime, Duration, Local};
use std::path::Path;
use std::thread::sleep;

/// How long a run may stay in `Starting` before it counts as failed (the
/// process that started it is gone). Longer than a launch can take.
const START_GRACE_MINUTES: i64 = 10;
/// How long after the prompt the agent must have started working.
const PROMPT_GRACE_MINUTES: i64 = 3;
/// Lines of output kept in a run's log.
const LOG_LINES: u32 = 2000;

/// An agent name Herdr accepts (`[a-z][a-z0-9_-]{0,31}`), unique per run.
pub fn agent_name(routine_id: &str, run_id: &str) -> String {
    let slug: String = routine_id
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .take(12)
        .collect();
    let slug = slug.trim_end_matches('-');
    // Routines whose ids start alike stay apart by a hash of the whole id.
    let hash = paths::fnv1a(routine_id.as_bytes()) & 0xffff;
    // The run id is `YYYYMMDD-HHMMSS-mmm`: keep the time and the milliseconds.
    let time: String = run_id
        .chars()
        .skip(9)
        .filter(char::is_ascii_digit)
        .collect();
    format!("rt-{slug}-{hash:04x}-{time}")
}

/// The tab label of a run, such as "Inbox clean up 08:15".
fn tab_label(routine: &Routine, started: &DateTime<Local>) -> String {
    let name: String = routine.file.name.chars().take(40).collect();
    format!("{name} {}", started.format("%H:%M"))
}

/// Records a new run as `Starting`. Saved at once, so the scheduler sees it
/// before the agent is up and does not start the routine twice.
pub fn prepare(
    paths: &Paths,
    routine: &Routine,
    trigger: Trigger,
    slot: Option<DateTime<Local>>,
) -> Run {
    let mut run = Run::new(
        &routine.id,
        &routine.file.name,
        &routine.file.agent,
        trigger,
        slot,
    );
    run.server = paths.server_id();
    let _ = run.save(paths);
    run
}

/// Records a run that did not start because the previous one still runs.
pub fn skip(paths: &Paths, routine: &Routine, trigger: Trigger, slot: DateTime<Local>) {
    let mut run = Run::new(
        &routine.id,
        &routine.file.name,
        &routine.file.agent,
        trigger,
        Some(slot),
    );
    run.server = paths.server_id();
    run.finish(Status::Skipped, "The previous run was still going");
    let _ = run.save(paths);
}

/// Opens the tab, starts the agent and hands it the prompt. Blocks while the
/// agent starts: up to a minute, a bit longer while the new shell gets ready.
pub fn launch(paths: &Paths, settings: &Settings, routine: &Routine, mut run: Run) -> Run {
    if let Some(problem) = &routine.problem {
        run.finish(Status::Failed, format!("Cannot run: {problem}"));
        let _ = run.save(paths);
        return run;
    }
    let folder = paths::expand(&routine.file.folder);
    let place = match herdr::open_tab(
        &settings.workspace,
        &folder.to_string_lossy(),
        &tab_label(routine, &run.started),
    ) {
        Ok(p) => p,
        Err(e) => {
            run.finish(Status::Failed, format!("Could not open a tab: {e}"));
            let _ = run.save(paths);
            return run;
        }
    };
    run.workspace_id = place.workspace_id;
    run.tab_id = place.tab_id;
    run.pane_id = place.pane_id.clone();
    if routine.is_shell() {
        return launch_shell(paths, routine, run);
    }
    run.agent_name = agent_name(&routine.id, &run.id);
    let _ = run.save(paths);

    // The new shell needs a moment before it shows its prompt: retry quick
    // refusals, but not a timeout, since the agent may then already run.
    let mut started = Err(herdr::Error {
        code: String::new(),
        message: "not tried".into(),
    });
    for attempt in 0..6 {
        if attempt > 0 {
            sleep(std::time::Duration::from_secs(2));
        }
        started = herdr::agent_start(
            &run.agent_name,
            &routine.file.agent,
            &place.pane_id,
            &crate::agent::flags(&routine.file),
        );
        match &started {
            Ok(()) => break,
            // Up, but asking something first (such as trusting the folder).
            Err(e) if e.code == "agent_not_ready" || e.code == "timeout" => break,
            Err(_) => continue,
        }
    }
    match started {
        Ok(()) => match hand_over(&mut run, &routine.prompt) {
            Ok(()) => {}
            Err(e) if e.code == "agent_blocked" => {
                run.status = Status::NeedsYou;
                run.message = "The agent asks something before it can start".into();
            }
            Err(e) => run.finish(
                Status::Failed,
                format!("Could not hand over the prompt: {e}"),
            ),
        },
        Err(e) if e.code == "agent_not_ready" => {
            run.status = Status::NeedsYou;
            run.message = "The agent asks something before it can start".into();
        }
        // The prompt follows once the agent is ready (see `watch`).
        Err(e) if e.code == "timeout" => {
            run.status = Status::NeedsYou;
            run.message = "The agent was slow to start; check its tab".into();
        }
        Err(e) => run.finish(
            Status::Failed,
            format!("Could not start {}: {e}", routine.file.agent),
        ),
    }
    if run.status == Status::NeedsYou {
        notify_needs_you(&mut run);
    }
    let _ = run.save(paths);
    run
}

/// Text in single quotes for any shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The wrapper a shell routine runs through: the routine's command in the
/// person's shell, then its exit code into a file the scheduler reads.
fn wrapper(shell: &str, script: &Path, exit: &Path) -> String {
    format!(
        "#!/bin/sh\n# herdr-routines: runs the routine's command and notes its exit code.\n{} {}\nprintf '%s' \"$?\" > {}\n",
        quote(shell),
        quote(&script.to_string_lossy()),
        quote(&exit.to_string_lossy()),
    )
}

/// Runs a shell routine in the run's tab: its command goes into a script
/// next to the run record, and the tab's shell runs it through `/bin/sh`
/// (so the line works in any shell) with the tab's environment.
fn launch_shell(paths: &Paths, routine: &Routine, mut run: Run) -> Run {
    let script = run.file(paths, "sh");
    let wrap = run.file(paths, "run");
    let exit = run.file(paths, "exit");
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let written = paths::write_atomic(&script, routine.prompt.as_bytes())
        .and_then(|_| paths::write_atomic(&wrap, wrapper(&shell, &script, &exit).as_bytes()));
    if let Err(e) = written {
        run.finish(Status::Failed, format!("Could not write the command: {e}"));
        let _ = run.save(paths);
        return run;
    }
    let line = format!("/bin/sh {}", quote(&wrap.to_string_lossy()));
    let mut result = herdr::pane_run(&run.pane_id, &line);
    for _ in 0..3 {
        if result.is_ok() {
            break;
        }
        sleep(std::time::Duration::from_secs(2));
        result = herdr::pane_run(&run.pane_id, &line);
    }
    match result {
        Ok(()) => {
            run.status = Status::Running;
            run.prompted = Some(Local::now());
        }
        Err(e) => run.finish(Status::Failed, format!("Could not run the command: {e}")),
    }
    let _ = run.save(paths);
    run
}

/// Follows a shell run: done once its exit code is written.
fn watch_shell(paths: &Paths, settings: &Settings, run: &mut Run) {
    if let Ok(code) = std::fs::read_to_string(run.file(paths, "exit")) {
        let code = code.trim();
        if code == "0" {
            run.finish(Status::Finished, "");
            if settings.notify_finished {
                herdr::notify(
                    &format!("{} finished", run.routine_name),
                    "Open herdr-routines to see the run.",
                    "done",
                );
            }
        } else {
            run.finish(Status::Failed, format!("Ended with exit code {code}"));
            herdr::notify(
                &format!("{} failed", run.routine_name),
                &format!("The command ended with exit code {code}. Its tab stays open."),
                "request",
            );
        }
    } else if !herdr::pane_exists(&run.pane_id).unwrap_or(true) {
        run.finish(
            Status::Closed,
            "The tab was closed before the command ended",
        );
        run.tab_closed = true;
    }
}

/// Types the prompt into the agent, noting its completed turns first so
/// that one more turn later means the prompt is done.
fn hand_over(run: &mut Run, prompt: &str) -> Result<(), herdr::Error> {
    let turns = herdr::agent_state(&run.pane_id)?.map(|(_, t)| t);
    herdr::agent_prompt(&run.pane_id, prompt)?;
    run.turns_before = turns;
    run.seen_working = false;
    run.prompted = Some(Local::now());
    run.status = Status::Running;
    run.notified = false;
    Ok(())
}

fn notify_needs_you(run: &mut Run) {
    if !run.notified {
        herdr::notify(
            &format!("{} needs you", run.routine_name),
            "The routine's agent is waiting for an answer. Open herdr-routines to jump to it.",
            "request",
        );
        run.notified = true;
    }
}

fn save_log(paths: &Paths, run: &Run) {
    if run.pane_id.is_empty() {
        return;
    }
    if let Ok(text) = herdr::read(&run.pane_id, LOG_LINES) {
        if !text.trim().is_empty() {
            let _ = paths::write_atomic(&run.log_path(paths), text.as_bytes());
        }
    }
}

/// Follows an active run one step. Returns whether the record changed.
pub fn watch(paths: &Paths, settings: &Settings, run: &mut Run, routine: Option<&Routine>) -> bool {
    let now = Local::now();
    let prompt = routine.map(|r| r.prompt.as_str());
    let limit_minutes = routine.map_or(120, |r| r.file.time_limit_minutes);
    let before = run.clone();

    if run.status == Status::Starting {
        if now - run.started > Duration::minutes(START_GRACE_MINUTES) {
            run.finish(Status::Failed, "Starting was interrupted");
        }
        return finish_up(paths, settings, run, &before);
    }
    if !run.status.active() {
        return false;
    }

    if run.agent == SHELL {
        watch_shell(paths, settings, run);
    } else {
        let (state, turns) = match herdr::agent_state(&run.pane_id) {
            Ok(Some((s, t))) => (Some(s), t),
            Ok(None) => (None, 0),
            // Herdr unreachable: try again next time.
            Err(_) => return false,
        };
        // A quick agent can finish between two looks, so a new completed turn
        // counts as having worked.
        if run.prompted.is_some() && run.turns_before.is_some_and(|b| turns > b) {
            run.seen_working = true;
        }
        match state.as_deref() {
            None => {
                let pane_alive = herdr::pane_exists(&run.pane_id).unwrap_or(true);
                if run.seen_working && pane_alive {
                    run.finish(Status::Finished, "The agent ended");
                } else if pane_alive {
                    run.finish(Status::Closed, "The agent quit before it finished");
                } else {
                    run.finish(Status::Closed, "The tab was closed before the run finished");
                    run.tab_closed = true;
                }
            }
            Some("working") => {
                // Working before the prompt (starting up) does not count.
                if run.prompted.is_some() {
                    run.seen_working = true;
                }
                run.status = Status::Running;
                run.notified = false;
                if run.message.starts_with("The agent asks")
                    || run.message.starts_with("The agent did not")
                    || run.message.starts_with("The agent was slow")
                {
                    run.message.clear();
                }
            }
            Some("blocked") => {
                run.status = Status::NeedsYou;
                if run.message.is_empty() {
                    run.message = "The agent asks a question or waits for an approval".into();
                }
                notify_needs_you(run);
            }
            Some("idle") | Some("done") => {
                if run.prompted.is_none() {
                    // The agent was held up at start; it is ready now.
                    if let Some(text) = prompt {
                        match hand_over(run, text) {
                            Ok(()) => run.message.clear(),
                            Err(e) => run.message = format!("Could not hand over the prompt: {e}"),
                        }
                    }
                } else if run.seen_working {
                    run.finish(Status::Finished, "");
                    run.turns_at_end = Some(turns);
                    if settings.notify_finished {
                        herdr::notify(
                            &format!("{} finished", run.routine_name),
                            "Open herdr-routines to see the run.",
                            "done",
                        );
                    }
                } else if run
                    .prompted
                    .is_some_and(|p| now - p > Duration::minutes(PROMPT_GRACE_MINUTES))
                {
                    run.status = Status::NeedsYou;
                    run.message = "The agent did not start on the prompt".into();
                    notify_needs_you(run);
                }
            }
            // `unknown`: an agent is there but Herdr cannot tell its state.
            _ => {}
        }
    }

    let limit = Duration::minutes(i64::from(limit_minutes));
    if run.status.active() && now - run.started > limit {
        run.finish(
            Status::TimedOut,
            format!(
                "Not finished after {} minutes; the tab stays open",
                limit.num_minutes()
            ),
        );
        herdr::notify(
            &format!("{} timed out", run.routine_name),
            "The run is past its time limit. Its tab stays open.",
            "request",
        );
    }
    finish_up(paths, settings, run, &before)
}

fn finish_up(paths: &Paths, _settings: &Settings, run: &mut Run, before: &Run) -> bool {
    if run == before {
        return false;
    }
    if !run.status.active() && before.status.active() {
        save_log(paths, run);
    }
    let _ = run.save(paths);
    true
}

/// Whether a run still holds its routine, so the next one must wait: an
/// active run, or a timed out one whose agent still works.
pub fn occupied(paths: &Paths, run: &Run) -> bool {
    if run.status.active() {
        return true;
    }
    if run.status != Status::TimedOut || run.tab_closed {
        return false;
    }
    if run.server != paths.server_id() {
        // On another server: assume busy while that server runs.
        return paths::server_alive(&run.server);
    }
    if run.agent == SHELL {
        // Still running until it notes its exit code.
        return !run.file(paths, "exit").exists()
            && herdr::pane_exists(&run.pane_id).unwrap_or(true);
    }
    match herdr::agent_status(&run.pane_id) {
        Ok(Some(s)) => s != "idle" && s != "done",
        Ok(None) => false,
        // Cannot tell: better wait than run twice.
        Err(_) => true,
    }
}

/// Closes the tab of a finished run once it is old enough, unless someone
/// works in it or looks at the routines workspace. A conversation picked up
/// after the run starts the wait again.
pub fn tidy(paths: &Paths, settings: &Settings, run: &mut Run, workspace_in_view: bool) -> bool {
    if run.status != Status::Finished
        || run.tab_closed
        || settings.close_finished_after_minutes == 0
    {
        return false;
    }
    let Some(ended) = run.ended else {
        return false;
    };
    if Local::now() - ended < Duration::minutes(i64::from(settings.close_finished_after_minutes)) {
        return false;
    }
    match herdr::agent_state(&run.pane_id) {
        Ok(Some((s, _))) if s == "working" || s == "blocked" => return false,
        Ok(Some((_, turns))) if run.turns_at_end.is_some_and(|t| turns > t) => {
            run.turns_at_end = Some(turns);
            run.ended = Some(Local::now());
            let _ = run.save(paths);
            return false;
        }
        Err(_) => return false,
        _ => {}
    }
    if !herdr::pane_exists(&run.pane_id).unwrap_or(false) {
        run.tab_closed = true;
        let _ = run.save(paths);
        return true;
    }
    if workspace_in_view {
        return false;
    }
    save_log(paths, run);
    if herdr::close_tab(&run.tab_id).is_ok() {
        run.tab_closed = true;
        let _ = run.save(paths);
        return true;
    }
    false
}

/// Shows a run's tab, if it is still open.
pub fn show(paths: &Paths, run: &Run) -> Result<(), String> {
    if run.pane_id.is_empty() || run.tab_closed {
        return Err("its tab is closed".into());
    }
    if run.server != paths.server_id() {
        return Err("its tab is in another Herdr session".into());
    }
    match herdr::pane_exists(&run.pane_id) {
        Ok(true) => herdr::focus(&Place {
            workspace_id: run.workspace_id.clone(),
            tab_id: run.tab_id.clone(),
            pane_id: run.pane_id.clone(),
        })
        .map_err(|e| e.to_string()),
        Ok(false) => Err("its tab is closed".into()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_wrapper_quotes_paths() {
        assert_eq!(quote("it's"), "'it'\\''s'");
        let w = wrapper(
            "/bin/zsh",
            Path::new("/state/runs/a b/1.sh"),
            Path::new("/state/runs/a b/1.exit"),
        );
        assert!(w.contains("'/bin/zsh' '/state/runs/a b/1.sh'\n"), "{w}");
        assert!(
            w.ends_with("printf '%s' \"$?\" > '/state/runs/a b/1.exit'\n"),
            "{w}"
        );
    }

    #[test]
    fn agent_names_are_valid() {
        let n = agent_name("inbox-manager", "20261110-081500-123");
        assert!(n.starts_with("rt-inbox-manage-"), "{n}");
        assert!(n.ends_with("-081500123"), "{n}");
        assert!(n.len() <= 32);
        assert_ne!(
            agent_name("weekly-report-team-a", "20261110-081500-123"),
            agent_name("weekly-report-team-b", "20261110-081500-123")
        );
        let n = agent_name(
            "a-very-long-routine-name-that-goes-on",
            "20261110-081500-123",
        );
        assert!(n.len() <= 32, "{n}");
        assert!(n.starts_with("rt-"));
        assert!(!n.contains("--"));
    }
}
