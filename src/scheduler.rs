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
use crate::store::{self, Checked, Status, Trigger};
use chrono::{DateTime, Duration, Local};
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
    let Some(cron) = routine.cron() else {
        return Due::Not;
    };
    let from = last_checked.max(now - Duration::days(LOOKBACK_DAYS));
    match cron.latest_in(&from, &now) {
        None => Due::Not,
        Some(slot) if now - slot <= Duration::minutes(ON_TIME) => Due::Now(slot),
        Some(slot) => match routine.file.missed {
            Missed::RunOnce => Due::Missed(slot),
            Missed::Skip => Due::SkipMissed(slot),
        },
    }
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
    if busy(paths, &routine.id) {
        paths::log(
            paths,
            &format!("{}: previous run still going, skipped", routine.id),
        );
        if let Some(slot) = slot {
            runner::skip(paths, routine, trigger, slot);
        }
        return None;
    }
    let run = runner::prepare(paths, routine, trigger, slot);
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

/// One round: start what is due, follow active runs, close old tabs.
fn tick(
    paths: &Paths,
    checked: &mut Checked,
    launches: &mut Vec<JoinHandle<()>>,
) -> Result<(), herdr::Error> {
    let settings = Settings::load(paths);
    let routines = routine::load_all(paths);
    let now = Local::now();

    for r in &routines {
        let last = checked.0.get(&r.id).copied();
        checked.0.insert(r.id.clone(), now);
        let Some(last) = last else {
            continue; // New: counts from now.
        };
        if !r.file.enabled || r.problem.is_some() {
            continue;
        }
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
        launches.extend(launch);
    }
    launches.retain(|h| !h.is_finished());
    checked
        .0
        .retain(|id, _| routines.iter().any(|r| &r.id == id));
    checked.save(paths);

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
    let _ = std::fs::create_dir_all(&paths.state);
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
                args: vec![],
                folder: "~".into(),
                missed,
                time_limit_minutes: 60,
            },
            prompt: "go".into(),
            problem: None,
            unreadable: false,
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
