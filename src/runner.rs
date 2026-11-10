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
/// How often the log of a run with an open tab is saved again, so it
/// survives the tab being closed by hand.
const LOG_EVERY_SECS: u64 = 60;
/// How long a check command may take.
const CHECK_LIMIT: std::time::Duration = std::time::Duration::from_secs(300);
/// Results of earlier runs kept for a routine that remembers.
const MEMORY_ENTRIES: usize = 10;
/// Between two results in the memory file.
const MEMORY_SEPARATOR: &str = "<!-- run -->";
/// The lines around the summary an agent ends its run with.
const RESULT_START: &str = "RESULT START";
const RESULT_END: &str = "RESULT END";

/// The prompt as the agent gets it: the routine's prompt, then what the
/// check command printed, the results of earlier runs (when the routine
/// remembers) and the request for a summary.
pub fn full_prompt(paths: &Paths, routine: &Routine, check_output: Option<&str>) -> String {
    let mut p = routine.prompt.trim_end().to_string();
    if let Some(out) = check_output {
        let out: String = out.chars().take(8000).collect();
        p.push_str(&format!(
            "\n\nThe check command `{}` printed:\n```\n{}\n```",
            routine.file.check_command.trim(),
            out.trim_end()
        ));
    }
    if routine.file.remember {
        let memory = std::fs::read_to_string(memory_file(paths, &routine.id)).unwrap_or_default();
        if memory.trim().is_empty() {
            p.push_str(
                "\n\nThis is the first run that keeps results; there are no earlier results yet.",
            );
        } else {
            p.push_str(&format!(
                "\n\nResults of this routine's earlier runs, newest first, for context:\n\n{}",
                memory.replace(MEMORY_SEPARATOR, "").trim_end()
            ));
        }
    }
    p.push_str(&format!(
        "\n\nThis runs on a schedule and nobody watches. When you are done, end your last message with a short summary of the result (what you did, what you found, what is open), on lines of its own between a line {RESULT_START} and a line {RESULT_END}."
    ));
    p
}

/// The file with the results of a routine's earlier runs.
pub fn memory_file(paths: &Paths, routine_id: &str) -> std::path::PathBuf {
    paths.state.join("memory").join(format!("{routine_id}.md"))
}

/// The summary between the result lines of the agent's last answer.
pub fn extract_result(log: &str) -> String {
    let clean = |l: &str| {
        l.trim()
            .trim_start_matches(['⏺', '●', '•', '>', '│'])
            .trim()
            .to_string()
    };
    let lines: Vec<String> = log.lines().map(clean).collect();
    let Some(end) = lines.iter().rposition(|l| l == RESULT_END) else {
        return String::new();
    };
    let Some(start) = lines[..end].iter().rposition(|l| l == RESULT_START) else {
        return String::new();
    };
    lines[start + 1..end]
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The last line with text.
fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// Adds a run's result at the top of its routine's memory, keeping the
/// newest [`MEMORY_ENTRIES`].
fn remember(paths: &Paths, run: &Run) {
    if run.result.trim().is_empty() {
        return;
    }
    let path = memory_file(paths, &run.routine_id);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut entries: Vec<String> = old
        .split(MEMORY_SEPARATOR)
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    entries.insert(
        0,
        format!(
            "Run of {}:\n{}",
            run.started.format("%Y-%m-%d %H:%M"),
            run.result.trim()
        ),
    );
    entries.truncate(MEMORY_ENTRIES);
    let text = entries.join(&format!("\n\n{MEMORY_SEPARATOR}\n\n")) + "\n";
    let _ = std::fs::create_dir_all(path.parent().expect("has a parent"));
    let _ = paths::write_atomic(&path, text.as_bytes());
}

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
    if !routine.approved {
        run.finish(
            Status::Failed,
            "Cannot run: it changed outside the list and waits for approval",
        );
        let _ = run.save(paths);
        return run;
    }
    let folder = paths::expand(&routine.file.folder);
    // The check command decides whether there is anything to do.
    let mut check_output = None;
    let check = routine.file.check_command.trim();
    if !routine.is_shell() && !check.is_empty() {
        match crate::shell::run(check, Some(&folder), CHECK_LIMIT) {
            Some((0, out)) => check_output = Some(out),
            Some((code, out)) => {
                let last = last_line(&out);
                run.finish(
                    Status::NothingToDo,
                    if last.is_empty() {
                        format!("The check ended with exit code {code}")
                    } else {
                        format!("Check: {last}")
                    },
                );
                let _ = paths::write_atomic(&run.file(paths, "out"), out.as_bytes());
                let _ = run.save(paths);
                return run;
            }
            None => {
                run.finish(
                    Status::Failed,
                    format!(
                        "The check command did not start or took over {} minutes",
                        CHECK_LIMIT.as_secs() / 60
                    ),
                );
                let _ = run.save(paths);
                return run;
            }
        }
    }
    let prompt = if routine.is_shell() {
        routine.prompt.clone()
    } else {
        full_prompt(paths, routine, check_output.as_deref())
    };
    // What the agent gets, also for handing it over later (see `watch`).
    let _ = paths::write_atomic(&run.file(paths, "prompt"), prompt.as_bytes());
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
    run.terminal_id = place.terminal_id;
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
        Ok(()) => match hand_over(&mut run, &prompt) {
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

/// Text in single quotes for `/bin/sh`.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A path in single quotes that every common shell reads the same way:
/// sh, bash and zsh take single quotes literally, and fish does too as
/// long as the text has no quote and no backslash. `None` for a path with
/// either, or with a control character a line editor would act on.
fn portable_quote(s: &str) -> Option<String> {
    if s.chars().any(|c| c == '\'' || c == '\\' || c.is_control()) {
        return None;
    }
    Some(format!("'{s}'"))
}

/// The prompt as it is typed into the agent: no control characters, which
/// could end the paste early or press keys in the agent's screen (such as
/// the one that switches its permission mode). Line breaks and tabs stay.
fn clean_prompt(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// Whether the run's pane id still means the pane Herdr made for the run:
/// the same tab and, for records that have it, the same terminal. A
/// restarted Herdr hands out the same pane ids again, so the id alone could
/// point at another tab of the person.
fn own_pane(run: &Run) -> Result<bool, herdr::Error> {
    if run.pane_id.is_empty() {
        return Ok(false);
    }
    Ok(match herdr::pane_info(&run.pane_id)? {
        Some(p) => {
            p.tab_id == run.tab_id
                && (run.terminal_id.is_empty() || p.terminal_id == run.terminal_id)
        }
        None => false,
    })
}

/// The wrapper a shell routine runs through: the routine's command in the
/// person's shell, its whole output also into a file, then its exit code
/// into a file the scheduler reads (moved in place only once the output is
/// complete).
fn wrapper(shell: &str, script: &Path, out: &Path, exit: &Path) -> String {
    let exit = exit.to_string_lossy();
    let tmp = format!("{exit}.tmp");
    format!(
        "#!/bin/sh\n# herdr-routines: runs the routine's command, keeps its output and notes its exit code.\n{{ {} {}; printf '%s' \"$?\" > {}; }} 2>&1 | tee {}\nmv {} {}\n",
        quote(shell),
        quote(&script.to_string_lossy()),
        quote(&tmp),
        quote(&out.to_string_lossy()),
        quote(&tmp),
        quote(&exit),
    )
}

/// Runs a shell routine in the run's tab: its command goes into a script
/// next to the run record, and the tab's shell runs it through `/bin/sh`
/// (so the line works in any shell) with the tab's environment.
fn launch_shell(paths: &Paths, routine: &Routine, mut run: Run) -> Run {
    let script = run.file(paths, "sh");
    let wrap = run.file(paths, "run");
    let exit = run.file(paths, "exit");
    let Some(quoted_wrap) = portable_quote(&wrap.to_string_lossy()) else {
        run.finish(
            Status::Failed,
            "The state folder's path has a quote or backslash; a shell command cannot run from it",
        );
        let _ = run.save(paths);
        return run;
    };
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let written = paths::write_atomic(&script, routine.prompt.as_bytes()).and_then(|_| {
        paths::write_atomic(
            &wrap,
            wrapper(&shell, &script, &run.file(paths, "out"), &exit).as_bytes(),
        )
    });
    if let Err(e) = written {
        run.finish(Status::Failed, format!("Could not write the command: {e}"));
        let _ = run.save(paths);
        return run;
    }
    let line = format!("/bin/sh {quoted_wrap}");
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
    } else if !own_pane(run).unwrap_or(true) {
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
    let turns = herdr::agent_state(run.agent_target())?.map(|(_, t)| t);
    herdr::agent_prompt(run.agent_target(), &clean_prompt(prompt))?;
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
    // Only the run's own pane: never copy another tab's output.
    if !own_pane(run).unwrap_or(false) {
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
    // What was prepared for the agent at launch (with the check output and
    // memory), else the routine's prompt.
    let prepared = std::fs::read_to_string(run.file(paths, "prompt")).ok();
    let prompt = prepared
        .as_deref()
        .or_else(|| routine.map(|r| r.prompt.as_str()));
    let limit_minutes = routine.map_or(120, |r| r.file.time_limit_minutes);
    let stop_at_limit = routine.is_some_and(|r| r.file.stop_at_limit);
    let remembers = routine.is_some_and(|r| r.file.remember);
    let before = run.clone();

    if run.status == Status::Starting {
        if now - run.started > Duration::minutes(START_GRACE_MINUTES) {
            run.finish(Status::Failed, "Starting was interrupted");
        }
        return finish_up(paths, run, &before, remembers);
    }
    if !run.status.active() {
        return false;
    }
    // Keep the log current, so it survives the tab being closed by hand.
    if run.agent != SHELL && log_is_old(paths, run) {
        save_log(paths, run);
    }

    if run.agent == SHELL {
        watch_shell(paths, settings, run);
    } else {
        let (state, turns) = match herdr::agent_state(run.agent_target()) {
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
                let pane_alive = own_pane(run).unwrap_or(true);
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
        let minutes = match limit.num_minutes() {
            1 => "1 minute".to_string(),
            n => format!("{n} minutes"),
        };
        if stop_at_limit {
            // Keep what it wrote, then end it by closing its tab.
            save_log(paths, run);
            run.finish(
                Status::TimedOut,
                format!("Stopped after {minutes}, its time limit"),
            );
            if own_pane(run).unwrap_or(false) && herdr::close_tab(&run.tab_id).is_ok() {
                run.tab_closed = true;
            }
            herdr::notify(
                &format!("{} stopped", run.routine_name),
                &format!("The run reached its time limit of {minutes} and was stopped."),
                "request",
            );
        } else {
            run.finish(
                Status::TimedOut,
                format!("Not finished after {minutes}; the tab stays open"),
            );
            herdr::notify(
                &format!("{} timed out", run.routine_name),
                "The run is past its time limit. Its tab stays open.",
                "request",
            );
        }
    }
    finish_up(paths, run, &before, remembers)
}

/// Saves a run that just ended: its log, its result and, for a routine
/// that remembers, the result in its memory.
fn finish_up(paths: &Paths, run: &mut Run, before: &Run, remembers: bool) -> bool {
    if run == before {
        return false;
    }
    if !run.status.active() && before.status.active() {
        if !run.tab_closed {
            save_log(paths, run);
        }
        if run.result.is_empty() {
            run.result = if run.agent == SHELL {
                let out = std::fs::read_to_string(run.file(paths, "out")).unwrap_or_default();
                last_line(&out)
            } else {
                let log = std::fs::read_to_string(run.log_path(paths)).unwrap_or_default();
                extract_result(&log)
            };
        }
        if remembers && run.status == Status::Finished {
            remember(paths, run);
        }
    }
    let _ = run.save(paths);
    true
}

/// Whether the saved log is older than [`LOG_EVERY_SECS`] (or missing).
fn log_is_old(paths: &Paths, run: &Run) -> bool {
    std::fs::metadata(run.log_path(paths))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age.as_secs() >= LOG_EVERY_SECS)
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
        return !run.file(paths, "exit").exists() && own_pane(run).unwrap_or(true);
    }
    match herdr::agent_status(run.agent_target()) {
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
    // A conversation can go on after the run: keep its log current while
    // the tab is open.
    if !run.tab_closed && run.agent != SHELL && !run.pane_id.is_empty() && log_is_old(paths, run) {
        match own_pane(run) {
            Ok(true) => save_log(paths, run),
            // Gone, or the id now means another pane.
            Ok(false) => {
                run.tab_closed = true;
                let _ = run.save(paths);
                return true;
            }
            Err(_) => {}
        }
    }
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
    match herdr::agent_state(run.agent_target()) {
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
    // Gone, or the id now means another pane: the run's tab is not there
    // any more, and the other tab is not the plugin's to close.
    match own_pane(run) {
        Ok(true) => {}
        Ok(false) => {
            run.tab_closed = true;
            let _ = run.save(paths);
            return true;
        }
        Err(_) => return false,
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
    match own_pane(run) {
        Ok(true) => herdr::focus(&Place {
            workspace_id: run.workspace_id.clone(),
            tab_id: run.tab_id.clone(),
            pane_id: run.pane_id.clone(),
            terminal_id: run.terminal_id.clone(),
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
        let dir = std::env::temp_dir().join(format!("rt-wrap {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, out, exit) = (dir.join("1.sh"), dir.join("1.out"), dir.join("1.exit"));
        std::fs::write(&script, "echo one\necho two >&2\nexit 4\n").unwrap();
        let w = wrapper("/bin/sh", &script, &out, &exit);
        assert!(w.contains(&quote(&script.to_string_lossy())), "{w}");
        // It really runs: output and exit code land in their files.
        let wrap = dir.join("1.run");
        std::fs::write(&wrap, w).unwrap();
        let status = std::process::Command::new("/bin/sh")
            .arg(&wrap)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert_eq!(std::fs::read_to_string(&exit).unwrap(), "4");
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("one") && text.contains("two"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn results_and_memory() {
        let log = "❯ do it. ... between a line RESULT START and a line RESULT END.\n⏺ Done.\n\n  RESULT START\n  Cleaned 3 files.\n  Nothing open.\n  RESULT END\n";
        assert_eq!(extract_result(log), "Cleaned 3 files.\nNothing open.");
        assert_eq!(extract_result("no markers"), "");
        assert_eq!(last_line("a\nlast one\n\n"), "last one");
    }

    #[test]
    fn the_typed_line_reads_the_same_in_every_shell() {
        assert_eq!(
            portable_quote("/state/runs/a b/1.run").as_deref(),
            Some("'/state/runs/a b/1.run'")
        );
        // fish reads \' inside single quotes as a quote; a line editor acts
        // on control characters.
        assert_eq!(portable_quote("/st'ate/1.run"), None);
        assert_eq!(portable_quote("/st\\ate/1.run"), None);
        assert_eq!(portable_quote("/state/\u{15}1.run"), None);
    }

    #[test]
    fn prompts_lose_control_characters() {
        assert_eq!(
            clean_prompt("Do this:\r\n- a\tb\u{1b}[Z\u{7}\n- c"),
            "Do this:\n- a\tb[Z\n- c"
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
