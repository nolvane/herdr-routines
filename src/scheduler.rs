//! The scheduler: a background process that runs while the Herdr server
//! runs, starts routines when they are due and follows their runs.
//!
//! Each tick looks at the window since the routine was last checked. A due
//! time at most [`ON_TIME`] old runs as scheduled. An older one means Herdr
//! or the computer was off or asleep: the routine's `missed` setting decides
//! whether it runs once now or waits for its next time.

use crate::herdr;
use crate::paths::{self, Paths};
use crate::routine::{self, Missed, Routine, Settings};
use crate::runner;
use crate::schedule::Schedule;
use crate::store::{self, Checked, Status, Trigger};
use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::thread::JoinHandle;

/// A due time this recent counts as on time, older ones as missed.
pub const ON_TIME: i64 = 3;
/// Missed times older than this are not looked for.
const LOOKBACK_DAYS: i64 = 7;
const TICK: std::time::Duration = std::time::Duration::from_secs(10);
/// Failed Herdr calls in a row after which the scheduler gives up.
const MAX_FAILURES: u32 = 60;

#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    Not,
    Now(DateTime<Local>),
    Missed(DateTime<Local>),
    SkipMissed(DateTime<Local>),
}

/// Whether a routine is due in `(last_checked, now]`.
pub fn due(routine: &Routine, last_checked: DateTime<Local>, now: DateTime<Local>) -> Due {
    let Some(schedule) = routine.schedule() else {
        return Due::Not;
    };
    let from = last_checked.max(now - Duration::days(LOOKBACK_DAYS));
    match schedule.latest_in(&from, &now) {
        None => Due::Not,
        Some(slot) if now - slot <= Duration::minutes(ON_TIME) => Due::Now(slot),
        Some(slot) => match routine.file.missed {
            Missed::RunOnce => Due::Missed(slot),
            Missed::Skip => Due::SkipMissed(slot),
        },
    }
}

/// Runs `f` while holding the routine's lock file, so the scheduler, a
/// `fire` process and a scheduler handing over cannot all find the routine
/// free and each start a run. Without the lock (a file system that has no
/// locks) `f` still runs.
pub fn with_lock<T>(paths: &Paths, routine_id: &str, f: impl FnOnce() -> T) -> T {
    paths::locked(&paths.lock_file(routine_id), f)
}

/// Whether the routine's previous run still holds it.
pub fn busy(paths: &Paths, routine_id: &str) -> bool {
    store::runs_of(paths, routine_id)
        .iter()
        .any(|r| runner::occupied(paths, r))
}

/// Starts a routine now in a thread of this process. `slot` is the due time
/// it runs for, `None` when started by hand. Returns the thread, so the
/// scheduler can let launches finish before it exits.
pub fn fire(
    paths: &Paths,
    settings: &Settings,
    routine: &Routine,
    trigger: Trigger,
    slot: Option<DateTime<Local>>,
) -> Option<JoinHandle<()>> {
    fire_attempt(paths, settings, routine, trigger, slot, 0)
}

/// [`fire`] for a given try: 0 first, 1 for the first retry.
fn fire_attempt(
    paths: &Paths,
    settings: &Settings,
    routine: &Routine,
    trigger: Trigger,
    slot: Option<DateTime<Local>>,
    attempt: u32,
) -> Option<JoinHandle<()>> {
    let prepared = with_lock(paths, &routine.id, || {
        if busy(paths, &routine.id) {
            if let Some(slot) = slot {
                runner::skip(paths, routine, trigger, slot);
            }
            return None;
        }
        let mut run = runner::prepare(paths, routine, trigger, slot);
        if attempt > 0 {
            run.attempt = attempt;
            let _ = run.save(paths);
        }
        Some(run)
    });
    let Some(run) = prepared else {
        paths::log(
            paths,
            &format!("{}: previous run still going, skipped", routine.id),
        );
        return None;
    };
    paths::log(
        paths,
        &format!("{}: start run {} ({:?})", routine.id, run.id, trigger),
    );
    let (paths, settings, routine) = (paths.clone(), settings.clone(), routine.clone());
    Some(std::thread::spawn(move || {
        let run = runner::launch(&paths, &settings, &routine, run);
        paths::log(
            &paths,
            &format!(
                "{}: run {} is {:?} {}",
                routine.id, run.id, run.status, run.message
            ),
        );
    }))
}

/// Routines that were due while they waited for approval and whose owner
/// was told, so the notification comes once per wait.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Waiting(BTreeSet<String>);

impl Waiting {
    fn load(paths: &Paths) -> Waiting {
        std::fs::read(paths.waiting_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, paths: &Paths) {
        if let Ok(text) = serde_json::to_vec_pretty(self) {
            let _ = paths::write_atomic(&paths.waiting_file(), &text);
        }
    }

    fn insert(&mut self, id: String) -> bool {
        self.0.insert(id)
    }

    fn retain(&mut self, keep: impl FnMut(&String) -> bool) {
        self.0.retain(keep)
    }
}

/// One round: start what is due, follow active runs, close old tabs.
fn tick(
    paths: &Paths,
    checked: &mut Checked,
    launches: &mut Vec<JoinHandle<()>>,
) -> Result<(), herdr::Error> {
    let settings = Settings::load(paths);
    let routines = routine::load_all(paths);
    let now = Local::now();
    let mut waiting = Waiting::load(paths);
    let waiting_before = waiting.clone();

    let server = paths.server_id();
    let mut started = Started::load(paths);
    for r in &routines {
        let last = checked.0.get(&r.id).copied();
        checked.0.insert(r.id.clone(), now);
        let at_start = matches!(r.schedule(), Some(Schedule::AtStart));
        let Some(last) = last else {
            // New: counts from now, also for "when Herdr starts".
            if at_start {
                started.0.insert(r.id.clone(), server.clone());
            }
            continue;
        };
        // A time passed while it waited for approval: tell the owner, once
        // per wait, since nothing else shows that it did not run.
        if r.file.enabled
            && !r.approved
            && !r.unreadable
            && due(r, last, now) != Due::Not
            && waiting.insert(r.id.clone())
        {
            let name = crate::app::visible_text(&r.file.name);
            paths::log(paths, &format!("{}: due, but waits for approval", r.id));
            herdr::notify(
                &format!("{name} waits for your approval"),
                "It did not run: it changed outside the list. a in the routines list shows it.",
                "request",
            );
        }
        // Off, broken or changed outside the list without the owner's
        // approval: no run, and no catch-up for the times passed meanwhile.
        if !r.file.enabled || !r.can_run() {
            continue;
        }
        if at_start {
            // Once per Herdr server, not per scheduler (it restarts on
            // updates).
            if started.0.get(&r.id) != Some(&server) {
                started.0.insert(r.id.clone(), server.clone());
                launches.extend(fire(paths, &settings, r, Trigger::Start, None));
            }
            continue;
        }
        if let Some(launch) = retry(paths, &settings, r, now) {
            launches.push(launch);
            continue;
        }
        let once = matches!(r.schedule(), Some(Schedule::Once(_)));
        let launch = match due(r, last, now) {
            Due::Not => None,
            Due::Now(slot) => fire(paths, &settings, r, Trigger::Scheduled, Some(slot)),
            Due::Missed(slot) => fire(paths, &settings, r, Trigger::Missed, Some(slot)),
            Due::SkipMissed(slot) => {
                paths::log(
                    paths,
                    &format!(
                        "{}: missed {}, skipped as set",
                        r.id,
                        slot.format("%Y-%m-%d %H:%M")
                    ),
                );
                None
            }
        };
        if once && launch.is_some() {
            // A single run switches its routine off once it started. It
            // ran approved, so it stays approved (off), as `space` would.
            let mut done = r.clone();
            done.file.enabled = false;
            let _ = done.save_approved(paths);
        }
        launches.extend(launch);
    }
    launches.retain(|h| !h.is_finished());
    checked
        .0
        .retain(|id, _| routines.iter().any(|r| &r.id == id));
    checked.save(paths);
    started
        .0
        .retain(|id, _| routines.iter().any(|r| &r.id == id));
    started.save(paths);
    // Approved or gone: the next wait is announced again.
    waiting.retain(|id| routines.iter().any(|r| &r.id == id && !r.approved));
    if waiting != waiting_before {
        waiting.save(paths);
    }

    // Reachability check, so a gone server ends the loop.
    let in_view = workspace_in_view(&settings.workspace)?;
    let me = paths.server_id();
    for mut run in store::all_runs(paths) {
        if run.server != me {
            // Pane ids belong to the server the run was started on. When
            // that server is gone, so is the agent.
            if !paths::server_alive(&run.server) && (run.status.active() || !run.tab_closed) {
                if run.status.active() {
                    run.finish(Status::Closed, "Its Herdr session ended before it finished");
                }
                run.tab_closed = true;
                let _ = run.save(paths);
            }
            continue;
        }
        if run.status.active() {
            let routine = routines.iter().find(|r| r.id == run.routine_id);
            if runner::watch(paths, &settings, &mut run, routine) {
                paths::log(
                    paths,
                    &format!("{}: run {} is {:?}", run.routine_id, run.id, run.status),
                );
            }
        } else {
            runner::tidy(paths, &settings, &mut run, in_view);
        }
    }
    for r in &routines {
        store::prune(paths, &r.id, settings.history);
    }
    Ok(())
}

/// Tries a failed run again when the routine allows it and the wait is
/// over. Only the newest run counts: a later success ends the retries.
fn retry(
    paths: &Paths,
    settings: &Settings,
    routine: &Routine,
    now: DateTime<Local>,
) -> Option<JoinHandle<()>> {
    if routine.file.retries == 0 {
        return None;
    }
    let last = store::runs_of(paths, &routine.id).into_iter().next()?;
    let wait = Duration::minutes(i64::from(routine.file.retry_delay_minutes));
    let ended = last.ended?;
    if last.status != Status::Failed || last.attempt >= routine.file.retries || now - ended < wait {
        return None;
    }
    paths::log(
        paths,
        &format!(
            "{}: retry {} of {}",
            routine.id,
            last.attempt + 1,
            routine.file.retries
        ),
    );
    fire_attempt(
        paths,
        settings,
        routine,
        Trigger::Retry,
        last.slot,
        last.attempt + 1,
    )
}

/// The Herdr server each "when Herdr starts" routine last ran for.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Started(std::collections::BTreeMap<String, String>);

impl Started {
    fn file(paths: &Paths) -> std::path::PathBuf {
        paths.state.join("started.json")
    }

    fn load(paths: &Paths) -> Started {
        std::fs::read(Started::file(paths))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, paths: &Paths) {
        if let Ok(text) = serde_json::to_vec_pretty(self) {
            let _ = paths::write_atomic(&Started::file(paths), &text);
        }
    }
}

/// Whether the routines workspace is the one on screen.
fn workspace_in_view(label: &str) -> Result<bool, herdr::Error> {
    herdr::workspace_label_focused(label)
}

/// The pid in the pid file and the socket of the server it serves.
fn read_pid(paths: &Paths) -> Option<(u32, String)> {
    let text = std::fs::read_to_string(paths.pid_file()).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    Some((pid, lines.next().unwrap_or("").to_string()))
}

/// Whether the process runs and is this plugin (a pid left over from before
/// a restart may belong to something else by now).
pub fn alive(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("herdr-routines")
        })
        .unwrap_or(false)
}

/// The scheduler's pid and the socket it serves, when it runs.
pub fn running(paths: &Paths) -> Option<(u32, String)> {
    read_pid(paths).filter(|(pid, _)| alive(*pid))
}

/// Whether `start` should leave a running scheduler alone: it serves another
/// Herdr server that still runs. The same socket means this server
/// restarted, so the new start takes over.
pub fn keep_running(paths: &Paths) -> Option<u32> {
    let (pid, socket) = running(paths)?;
    let other = socket != paths.socket.to_string_lossy();
    (other && Path::new(&socket).exists()).then_some(pid)
}

/// Starts `run` detached from Herdr's pipes and process group, so the startup
/// hook or action returns at once. A running scheduler hands over and exits.
pub fn spawn(paths: &Paths) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own binary: {e}"))?;
    let child = Command::new(exe)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("start failed: {e}"))?;
    paths::log(paths, &format!("spawned scheduler pid {}", child.id()));
    Ok(child.id())
}

pub fn stop(paths: &Paths) {
    let _ = std::fs::remove_file(paths.pid_file());
}

/// Identifies the server behind the socket: a new server binds a new socket
/// file, so its device and inode change.
fn socket_identity(socket: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(socket).ok().map(|m| (m.dev(), m.ino()))
}

/// The loop itself.
pub fn run(paths: &Paths) -> i32 {
    let me = std::process::id();
    let _ = paths::create_private_dir(&paths.state);
    // Taking over the pid file tells an older instance to hand over and exit.
    let content = format!("{me}\n{}\n", paths.socket.display());
    if paths::write_atomic(&paths.pid_file(), content.as_bytes()).is_err() {
        paths::log(paths, "cannot write pid file");
        return 1;
    }
    paths::log(
        paths,
        &format!(
            "scheduler v{} started for {}",
            env!("CARGO_PKG_VERSION"),
            paths.socket.display()
        ),
    );
    let identity = socket_identity(&paths.socket);
    let mut checked = Checked::load(paths);
    let mut failures = 0u32;
    let mut launches = Vec::new();
    loop {
        if read_pid(paths).map(|(p, _)| p) != Some(me) {
            paths::log(
                paths,
                "another scheduler took over or stop was requested; exiting",
            );
            break;
        }
        if identity.is_some() && socket_identity(&paths.socket) != identity {
            paths::log(paths, "the Herdr server changed; exiting");
            break;
        }
        match tick(paths, &mut checked, &mut launches) {
            Ok(()) => failures = 0,
            Err(e) => {
                failures += 1;
                if failures == 1 || failures.is_multiple_of(10) {
                    paths::log(paths, &format!("tick failed ({failures}): {e}"));
                }
                if failures >= MAX_FAILURES {
                    paths::log(paths, "Herdr unreachable; exiting");
                    break;
                }
            }
        }
        sleep(TICK);
    }
    // Let launches in progress finish, so no run is left half started.
    for handle in launches {
        let _ = handle.join();
    }
    if read_pid(paths).map(|(p, _)| p) == Some(me) {
        let _ = std::fs::remove_file(paths.pid_file());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routine::RoutineFile;
    use chrono::TimeZone;

    fn routine(schedule: &str, missed: Missed) -> Routine {
        Routine {
            id: "r".into(),
            file: RoutineFile {
                name: "R".into(),
                enabled: true,
                schedule: schedule.into(),
                agent: "claude".into(),
                permission_mode: String::new(),
                model: String::new(),
                effort: String::new(),
                args: vec![],
                folder: "~".into(),
                missed,
                time_limit_minutes: 60,
                stop_at_limit: false,
                retries: 0,
                retry_delay_minutes: 10,
                check_command: String::new(),
                remember: false,
                alert: Default::default(),
            },
            prompt: "go".into(),
            problem: None,
            unreadable: false,
            approved: true,
        }
    }

    fn t(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 10, h, m, 0).unwrap()
    }

    #[test]
    fn on_time_missed_and_skipped() {
        let r = routine("15 8-18 * * *", Missed::RunOnce);
        assert_eq!(due(&r, t(8, 0), t(8, 10)), Due::Not);
        assert_eq!(due(&r, t(8, 10), t(8, 15)), Due::Now(t(8, 15)));
        assert_eq!(due(&r, t(8, 14), t(8, 17)), Due::Now(t(8, 15)));
        // Off from 07:00 to 10:30: one catch-up run for the latest time.
        assert_eq!(due(&r, t(7, 0), t(10, 30)), Due::Missed(t(10, 15)));
        let r = routine("15 8-18 * * *", Missed::Skip);
        assert_eq!(due(&r, t(7, 0), t(10, 30)), Due::SkipMissed(t(10, 15)));
        // Already handled.
        assert_eq!(due(&r, t(8, 15), t(8, 20)), Due::Not);
    }
}
