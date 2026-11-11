//! The scheduler: a background process that runs while the Herdr server
//! runs, starts routines when they are due and follows their runs.
//!
//! Each tick looks at the window since the routine was last checked. A due
//! time at most [`ON_TIME`] old runs as scheduled. An older one means Herdr
//! or the computer was off or asleep: the routine's `missed` setting decides
//! whether it runs once now or waits for its next time.

use crate::approval::Approvals;
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
/// Failed rounds in a row (ten minutes) after which the scheduler takes it
/// that Herdr does not answer for now and asks less often.
const MAX_FAILURES: u32 = 60;
/// Rounds left out between two tries then: one try a minute.
const SLOW_ROUNDS: u32 = 5;

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
    round: u64,
) -> Result<(), herdr::Error> {
    let settings = Settings::load(paths);
    // Reachability first, so a gone server ends the loop and nothing is
    // started (and recorded as failed) while Herdr does not answer. The
    // times that pass meanwhile count as missed once it answers again.
    let in_view = workspace_in_view(&settings.workspace)?;
    let routines = routine::load_all(paths);
    let now = Local::now();
    let mut waiting = Waiting::load(paths);
    let waiting_before = waiting.clone();

    let server = paths.server_id();
    let mut started = Started::load(paths);
    for r in &routines {
        // Cron jobs: the crontab starts them (`crontab::wrapper_main`).
        if r.is_cron() {
            continue;
        }
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

    let me = paths.server_id();
    // Routines with a run that ended in this round: their history is
    // pruned right away.
    let mut ended: BTreeSet<String> = BTreeSet::new();
    for mut run in store::all_runs(paths) {
        if run.cron_pid != 0 {
            // A cron job's run belongs to its process; only one that ended
            // without finishing the record is closed here.
            if run.status.active() && !alive_since(run.cron_pid, &run.cron_started) {
                run.finish(Status::Closed, "The cron run ended before it finished");
                let _ = run.save(paths);
            }
            continue;
        }
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
                if !run.status.active() {
                    ended.insert(run.routine_id.clone());
                }
            }
        } else {
            runner::tidy(paths, &settings, &mut run, in_view);
        }
    }
    prune(paths, &routines, &ended, round);
    Ok(())
}

/// Every this many rounds (about ten minutes) all histories are pruned,
/// for runs that became too old; in between only those of routines whose
/// run just ended. Reading every record of every routine in every round
/// would grow with the history.
const PRUNE_EVERY: u64 = 60;

/// Prunes run histories with the settings the owner approved. A routine
/// whose files changed outside the list (or no longer load) keeps what its
/// approved version says until the change is approved: an edit nobody has
/// seen must not delete history. One that was never approved has no runs.
fn prune(paths: &Paths, routines: &[Routine], ended: &BTreeSet<String>, round: u64) {
    let all = round.is_multiple_of(PRUNE_EVERY);
    if !all && ended.is_empty() {
        return;
    }
    let approvals = Approvals::load(paths);
    for r in routines {
        if !all && !ended.contains(&r.id) {
            continue;
        }
        let keep = if r.approved && !r.unreadable {
            Some((r.file.keep_runs, r.file.keep_days))
        } else {
            approvals
                .routine(&r.id)
                .map(|a| (a.file.keep_runs, a.file.keep_days))
        };
        if let Some((runs, days)) = keep {
            store::prune(paths, &r.id, runs, days);
        }
    }
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

/// The pid in the pid file, the socket of the server it serves and when
/// that process started (empty in a file an older version wrote).
fn read_pid(paths: &Paths) -> Option<(u32, String, String)> {
    let text = paths::read_text_limited(&paths.pid_file(), paths::FILE_LIMIT).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let socket = lines.next().unwrap_or("").to_string();
    Some((pid, socket, lines.next().unwrap_or("").trim().to_string()))
}

/// One column of `ps` for a process, `None` when there is no such process.
fn ps(pid: u32, column: &str) -> Option<String> {
    let mut command = Command::new("ps");
    command
        .args(["-p", &pid.to_string(), "-o", column])
        // The start time in one form everywhere.
        .env("LC_ALL", "C");
    let out = crate::shell::tool(command, std::time::Duration::from_secs(10)).ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.success && !text.is_empty()).then_some(text)
}

/// When a process started, as `ps` prints it (`Tue Nov 10 08:15:02 2026`).
/// With the pid it names one process: the system hands a pid out again,
/// but not with the same start time.
pub fn started_at(pid: u32) -> Option<String> {
    ps(pid, "lstart=")
}

/// Whether the process with this pid and start time still runs. Records
/// from before the start time was kept (`started` empty) fall back to the
/// process's name, which says less: it depends on where the plugin's
/// folders are and is cut short on Linux.
pub fn alive_since(pid: u32, started: &str) -> bool {
    if started.is_empty() {
        return ps(pid, "comm=").is_some_and(|name| name.contains("herdr-routines"));
    }
    started_at(pid).as_deref() == Some(started)
}

/// The scheduler's pid and the socket it serves, when it runs.
pub fn running(paths: &Paths) -> Option<(u32, String)> {
    read_pid(paths)
        .filter(|(pid, _, started)| alive_since(*pid, started))
        .map(|(pid, socket, _)| (pid, socket))
}

/// Herdr servers whose `start` found the scheduler serving another server
/// and left it alone, one socket per line. When that server goes away, the
/// scheduler hands over to one of these instead of leaving them without.
fn waiting_servers_file(paths: &Paths) -> std::path::PathBuf {
    paths.state.join("waiting-servers")
}

fn waiting_servers(paths: &Paths) -> Vec<std::path::PathBuf> {
    paths::read_text_limited(&waiting_servers_file(paths), paths::FILE_LIMIT)
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(std::path::PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn save_waiting_servers(paths: &Paths, sockets: &[std::path::PathBuf]) {
    let text: String = sockets
        .iter()
        .map(|s| format!("{}\n", s.display()))
        .collect();
    let _ = paths::write_atomic(&waiting_servers_file(paths), text.as_bytes());
}

/// Notes that this server has no scheduler of its own (see
/// [`waiting_servers_file`]).
pub fn note_waiting(paths: &Paths) {
    let mut all = waiting_servers(paths);
    if !all.contains(&paths.socket) {
        all.push(paths.socket.clone());
        save_waiting_servers(paths, &all);
    }
}

/// Whether a Herdr server answers on this socket.
fn server_listens(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// The first waiting server that still runs, taken off the list together
/// with those that are gone and with this scheduler's own.
fn next_server(paths: &Paths) -> Option<std::path::PathBuf> {
    let mut all = waiting_servers(paths);
    all.retain(|s| *s != paths.socket && server_listens(s));
    let next = (!all.is_empty()).then(|| all.remove(0));
    save_waiting_servers(paths, &all);
    next
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
    spawn_for(paths, None)
}

/// [`spawn`], for the Herdr server on `socket` when given.
fn spawn_for(paths: &Paths, socket: Option<&Path>) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own binary: {e}"))?;
    let mut command = Command::new(exe);
    if let Some(socket) = socket {
        command.env("HERDR_SOCKET_PATH", socket);
    }
    let child = command
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

/// The marker of "routines off" in the list: no scheduler starts (also not
/// when Herdr starts) and cron jobs skip their times, until switched on.
fn paused_file(paths: &Paths) -> std::path::PathBuf {
    paths.state.join("paused")
}

pub fn paused(paths: &Paths) -> bool {
    paused_file(paths).exists()
}

/// Switches all routines off (`true`) or on again.
pub fn set_paused(paths: &Paths, off: bool) -> Result<(), String> {
    if off {
        paths::write_atomic(&paused_file(paths), b"switched off in the list\n")
            .map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(paused_file(paths)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
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
    let content = format!(
        "{me}\n{}\n{}\n",
        paths.socket.display(),
        started_at(me).unwrap_or_default()
    );
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
    let mut round = 0u64;
    // Rounds left out while Herdr does not answer (see below).
    let mut pause = 0u32;
    // The server this scheduler serves went away (its socket is gone).
    let mut server_gone = false;
    loop {
        if read_pid(paths).map(|(p, _, _)| p) != Some(me) {
            paths::log(
                paths,
                "another scheduler took over or stop was requested; exiting",
            );
            break;
        }
        if identity.is_some() && socket_identity(&paths.socket) != identity {
            paths::log(paths, "the Herdr server changed; exiting");
            // A new server on the same socket starts its own scheduler.
            server_gone = !paths.socket.exists();
            break;
        }
        if pause > 0 {
            pause -= 1;
            sleep(TICK);
            continue;
        }
        match tick(paths, &mut checked, &mut launches, round) {
            Ok(()) => {
                if failures >= MAX_FAILURES {
                    paths::log(paths, "Herdr answers again");
                }
                failures = 0;
            }
            Err(e) => {
                failures += 1;
                if failures == 1 || failures.is_multiple_of(10) {
                    paths::log(paths, &format!("tick failed ({failures}): {e}"));
                }
                if failures >= MAX_FAILURES {
                    if identity.is_none() {
                        // There never was a server on this socket.
                        paths::log(paths, "Herdr unreachable; exiting");
                        break;
                    }
                    if !server_listens(&paths.socket) {
                        // Its socket file is left, but nobody is behind it:
                        // the server ended without tidying up. The next
                        // server makes a new socket and starts its own
                        // scheduler.
                        paths::log(paths, "the Herdr server is gone; exiting");
                        server_gone = true;
                        break;
                    }
                    // The server is there but does not answer. Nothing
                    // would start a scheduler again once this one is gone,
                    // so it stays and asks once a minute instead of every
                    // round.
                    if failures == MAX_FAILURES {
                        paths::log(
                            paths,
                            "Herdr does not answer; trying once a minute until it does",
                        );
                    }
                    pause = SLOW_ROUNDS;
                }
            }
        }
        round += 1;
        sleep(TICK);
    }
    // Let launches in progress finish, so no run is left half started.
    for handle in launches {
        let _ = handle.join();
    }
    if read_pid(paths).map(|(p, _, _)| p) == Some(me) {
        let _ = std::fs::remove_file(paths.pid_file());
        // Another Herdr server was left without a scheduler because this
        // one ran: it gets one now.
        if server_gone && !paused(paths) {
            if let Some(socket) = next_server(paths) {
                match spawn_for(paths, Some(&socket)) {
                    Ok(pid) => paths::log(
                        paths,
                        &format!("handed over to pid {pid} for {}", socket.display()),
                    ),
                    Err(e) => paths::log(paths, &format!("could not hand over: {e}")),
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routine::RoutineFile;
    use crate::store::Run;
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
                runs_in: Default::default(),
                keep_runs: crate::routine::KEEP_RUNS,
                keep_days: crate::routine::KEEP_DAYS,
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

    fn temp_paths(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!(
            "herdr-routines-scheduler-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        };
        std::fs::create_dir_all(&paths.state).unwrap();
        paths
    }

    #[test]
    fn a_process_is_known_by_its_pid_and_start_time() {
        let me = std::process::id();
        let started = started_at(me).expect("ps knows this process");
        assert!(alive_since(me, &started));
        // The same pid handed out again to another process is not it.
        assert!(!alive_since(me, "Thu Jan  1 00:00:00 1970"));
        // A pid file whose process is gone, or is something else.
        let p = temp_paths("pid");
        std::fs::write(p.pid_file(), format!("{me}\n/tmp/x.sock\n{started}\n")).unwrap();
        assert_eq!(running(&p), Some((me, "/tmp/x.sock".to_string())));
        std::fs::write(
            p.pid_file(),
            format!("{me}\n/tmp/x.sock\nThu Jan  1 00:00:00 1970\n"),
        )
        .unwrap();
        assert_eq!(running(&p), None);
        // One written before the start time was kept: by name, and this
        // test is not the plugin (its binary has another name).
        std::fs::write(p.pid_file(), "1\n/tmp/x.sock\n").unwrap();
        assert_eq!(running(&p), None);
    }

    #[test]
    fn a_server_left_without_a_scheduler_gets_the_next_one() {
        let p = temp_paths("waiting");
        // Short names: a socket's path has a small limit.
        let dir = std::env::temp_dir().join(format!("hr-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (gone, live) = (dir.join("gone.sock"), dir.join("live.sock"));
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        for socket in [&gone, &live, &p.socket] {
            let mut other = p.clone();
            other.socket = socket.to_path_buf();
            note_waiting(&other);
            note_waiting(&other);
        }
        assert_eq!(waiting_servers(&p).len(), 3);
        // Not the one that is gone, and not this scheduler's own.
        assert_eq!(next_server(&p), Some(live));
        assert_eq!(next_server(&p), None);
        assert!(waiting_servers(&p).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_pruned_with_approved_settings_only() {
        let p = temp_paths("prune");
        let mut r = routine("0 9 * * *", Missed::Skip);
        (r.file.keep_runs, r.file.keep_days) = (5, 0);
        r.save_approved(&p).unwrap();
        for n in 0..8 {
            let mut run = Run::new("r", "R", "claude", Trigger::Scheduled, None);
            run.id = format!("20261101-08{n:02}00-000");
            run.finish(crate::store::Status::Finished, "");
            run.save(&p).unwrap();
        }
        let count = || store::runs_of(&p, "r").len();
        let all = |p: &Paths| crate::routine::load_all(p);
        let ended = BTreeSet::from(["r".to_string()]);
        // Changed outside the list to keep one run: nobody approved that.
        let file = p.routine_dir("r").join("routine.toml");
        let approved = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, approved.replace("keep_runs = 5", "keep_runs = 1")).unwrap();
        assert!(!all(&p)[0].approved);
        // Between the full rounds only routines whose run just ended.
        prune(&p, &all(&p), &BTreeSet::new(), 1);
        assert_eq!(count(), 8);
        prune(&p, &all(&p), &ended, 1);
        assert_eq!(count(), 5, "the approved 5, not the 1 in the file");
        // A file that no longer reads (half written, a typo) deletes nothing.
        std::fs::write(&file, "name = ").unwrap();
        assert!(all(&p)[0].unreadable);
        prune(&p, &all(&p), &ended, 0);
        assert_eq!(count(), 5);
        // Never approved: there are no settings to prune with.
        let mut other = routine("0 9 * * *", Missed::Skip);
        other.id = "other".into();
        (other.file.keep_runs, other.approved) = (1, false);
        other.save(&p).unwrap();
        for n in 0..3 {
            let mut run = Run::new("other", "O", "claude", Trigger::Manual, None);
            run.id = format!("20261101-09{n:02}00-000");
            run.finish(crate::store::Status::Finished, "");
            run.save(&p).unwrap();
        }
        prune(&p, &all(&p), &BTreeSet::new(), 0);
        assert_eq!(store::runs_of(&p, "other").len(), 3);
    }
}
