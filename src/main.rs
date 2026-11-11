//! herdr-routines: runs coding agents on a schedule while Herdr runs, each
//! run in its own tab, with a list to manage routines and their runs.
//!
//! Commands:
//! - `start`   (startup hook) starts the scheduler in the background
//! - `restart` (action) starts a fresh scheduler; the old one hands over
//! - `stop`    (action) stops the scheduler
//! - `status`  (action) says whether the scheduler runs
//! - `run`     the scheduler itself, in the foreground
//! - `list`    the list, in the current terminal
//! - `open [overlay|split|tab]` (action) opens or closes the list as a pane
//! - `fire <id>` starts a routine now (a cron job through its wrapper)
//! - `routines` prints the routines and their latest run as JSON
//! - `runs [<id>]` prints the run history (of one routine) as JSON
//! - `release-cron` turns every managed cron job back into a plain line
//!   of the crontab (before the plugin is removed)
//! - `alert-pane` (pane) the popup about a run, for routines that ask for it
//!
//! Started as `<state>/bin/cron/<id> -c <command>` (the `SHELL` of a
//! managed cron job in the crontab), it is that job's wrapper instead.

mod accounts;
mod agent;
mod alert;
mod app;
mod approval;
mod autostart;
mod awake;
mod clipboard;
mod cron;
mod crontab;
mod diff;
mod editor;
mod form;
mod herdr;
mod models;
mod pane;
mod paths;
mod routine;
mod runner;
mod schedule;
mod scheduler;
mod shell;
mod store;
mod theme;
mod ui;

use paths::Paths;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use routine::{Routine, Settings};
use store::Trigger;

/// The list's screen, with the mouse and pasting on.
pub fn enter_terminal() -> ratatui::DefaultTerminal {
    // ratatui's own panic hook restores raw mode and the screen; this one
    // also turns mouse reporting and bracketed paste off again.
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = ratatui::crossterm::execute!(
                std::io::stdout(),
                DisableBracketedPaste,
                DisableMouseCapture
            );
            previous(info);
        }));
    });
    let terminal = ratatui::init();
    let _ =
        ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste);
    terminal
}

pub fn leave_terminal() {
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
    ratatui::restore();
}

fn list(paths: Paths) -> std::io::Result<()> {
    let app = app::App::new(paths);
    // Herdr keeps right clicks for its own pane menu unless the pane asks
    // for them; the list has its own menu.
    let right_clicks = herdr::right_clicks_to_me(true);
    let mut terminal = enter_terminal();
    let result = app.run(&mut terminal);
    leave_terminal();
    if right_clicks {
        // The pane may stay (a shell the list was started in by hand).
        herdr::right_clicks_to_me(false);
    }
    pane::forget_self();
    result
}

/// Starts a routine now, in this process, and waits until its agent is up.
fn fire(paths: &Paths, id: &str) -> Result<String, String> {
    let routine = Routine::load(paths, id)?;
    if let Some(p) = &routine.problem {
        return Err(format!("cannot run {id}: {p}"));
    }
    if !routine.approved {
        return Err(format!(
            "cannot run {id}: it changed outside the list and waits for approval there (key a)"
        ));
    }
    if routine.is_cron() {
        // A classic cron job runs as cron would run it, as `r` in the list
        // does: through its wrapper, in the background, not in a tab.
        crontab::run_now(paths, &routine).map_err(|e| format!("cannot run {id}: {e}"))?;
        return Ok(format!(
            "{id}: started in the background, as cron would run it; `runs {id}` shows it"
        ));
    }
    let settings = Settings::load(paths);
    let run = scheduler::with_lock(paths, id, || {
        if scheduler::busy(paths, id) {
            return None;
        }
        Some(runner::prepare(paths, &routine, Trigger::Manual, None))
    })
    .ok_or_else(|| format!("{id} is already running"))?;
    let run = runner::launch(paths, &settings, &routine, run);
    // A run that did not get going may have started its command after a run.
    runner::wait_for_after();
    paths::log(
        paths,
        &format!(
            "{id}: run {} by hand is {:?} {}",
            run.id, run.status, run.message
        ),
    );
    Ok(format!(
        "{}: {} {}",
        run.id,
        run.status.label(),
        run.message
    ))
}

/// A run as the `routines` and `runs` commands print it.
fn run_json(l: &store::Run) -> serde_json::Value {
    serde_json::json!({
        "id": l.id,
        "routine_id": l.routine_id,
        "routine_name": l.routine_name,
        "agent": l.agent,
        "status": l.status,
        "trigger": l.trigger,
        "attempt": l.attempt,
        "slot": l.slot.map(|t| t.to_rfc3339()),
        "started": l.started.to_rfc3339(),
        "ended": l.ended.map(|t| t.to_rfc3339()),
        "message": l.message,
        "result": l.result,
        "problem": l.status.problem(),
    })
}

/// Every routine with every setting of its `routine.toml` (also those
/// left out of the file because they are at their default), its problem,
/// whether it is approved, its next time and its latest run.
fn routines_json(paths: &Paths) -> String {
    let now = chrono::Local::now();
    let items: Vec<serde_json::Value> = routine::load_all(paths)
        .iter()
        .map(|r| {
            let last = store::runs_of(paths, &r.id).into_iter().next();
            let f = &r.file;
            serde_json::json!({
                "id": r.id,
                "name": f.name,
                "enabled": f.enabled,
                "schedule": f.schedule,
                "schedule_text": r.schedule_text(),
                "agent": f.agent,
                "permission_mode": f.permission_mode,
                "model": f.model,
                "effort": f.effort,
                "args": f.args,
                "folder": f.folder,
                "missed": f.missed,
                "time_limit_minutes": f.time_limit_minutes,
                "stop_at_limit": f.stop_at_limit,
                "retries": f.retries,
                "retry_delay_minutes": f.retry_delay_minutes,
                "check_command": f.check_command,
                "remember": f.remember,
                "alert": f.alert,
                "runs_in": f.runs_in,
                "keep_runs": f.keep_runs,
                "keep_days": f.keep_days,
                "late_hours": f.more.late_hours,
                "after_command": f.more.after_command,
                "after_always": f.more.after_always,
                "off_after_failures": f.more.off_after_failures,
                "account": f.more.account,
                "failures_in_a_row": runner::failures_in_a_row(paths, &r.id),
                "problem": r.problem,
                "approved": r.approved,
                "next": r.schedule().filter(|_| f.enabled).and_then(|c| c.next_after(&now)).map(|t| t.to_rfc3339()),
                "last_run": last.as_ref().map(run_json),
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

/// The run history, newest first: of one routine, or of all.
fn runs_json(paths: &Paths, id: Option<&str>) -> Result<String, String> {
    let runs = match id {
        Some(id) if !routine::valid_id(id) => {
            return Err(format!("\"{id}\" is not a routine id"));
        }
        Some(id) => store::runs_of(paths, id),
        None => store::all_runs(paths),
    };
    let items: Vec<serde_json::Value> = runs.iter().map(run_json).collect();
    Ok(serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()))
}

/// Text for the terminal: what a routine's files say goes out without
/// control characters or characters that hide or turn text around, so a
/// file cannot send escape sequences to whoever runs a command here.
fn shown(text: &str) -> String {
    app::visible_text(&text.replace(['\n', '\r'], " "))
}

fn main() {
    // Cron starting a managed job: run it before anything else, with no
    // other setup, so the command runs whatever else is wrong.
    // The arguments as they are: `std::env::args` stops the program on one
    // that is not UTF-8, which a crontab line can hold.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some((me, cmd)) = crontab::wrapper_call(&args) {
        std::process::exit(crontab::wrapper_main(&me, &cmd));
    }
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let command = args.get(1).cloned().unwrap_or_else(|| "list".into());
    let paths = Paths::from_env();
    paths::secure_roots(&paths);
    let code = match command.as_str() {
        // Switched off in the list: stays off until switched on there.
        "start" | "restart" if scheduler::paused(&paths) => {
            println!("routines are switched off in the list; not started");
            0
        }
        "start" => {
            if let Some(pid) = scheduler::keep_running(&paths) {
                // When that session ends, the scheduler hands over to this one.
                scheduler::note_waiting(&paths);
                println!("already running for another Herdr session, pid {pid}");
                0
            } else {
                match scheduler::spawn(&paths) {
                    Ok(pid) => {
                        println!("started pid {pid}");
                        0
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                }
            }
        }
        "restart" => match scheduler::spawn(&paths) {
            Ok(pid) => {
                herdr::notify("herdr-routines", "Scheduler restarted", "none");
                println!("started pid {pid}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        "stop" => {
            scheduler::stop(&paths);
            herdr::notify(
                "herdr-routines",
                "Scheduler stopped: no routine runs until it starts again",
                "none",
            );
            println!("stopped");
            0
        }
        "status" => {
            match scheduler::running(&paths) {
                Some((pid, socket)) => println!("running, pid {pid}, socket {}", shown(&socket)),
                None => println!("not running"),
            }
            0
        }
        "run" => scheduler::run(&paths),
        "alert-pane" => alert::pane(&paths),
        "list" => match list(paths) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("herdr-routines: {e}");
                1
            }
        },
        "open" => {
            let settings = Settings::load(&paths);
            let placement = match args.get(2).map(String::as_str) {
                Some(p @ ("split" | "tab" | "overlay")) => p.to_string(),
                _ => settings.placement.clone(),
            };
            match pane::toggle(&placement, &settings.split_direction) {
                Ok(msg) => {
                    println!("{msg}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "fire" => match args.get(2) {
            Some(id) => match fire(&paths, id) {
                Ok(msg) => {
                    println!("{}", shown(&msg));
                    0
                }
                Err(e) => {
                    eprintln!("{}", shown(&e));
                    1
                }
            },
            None => {
                eprintln!("usage: herdr-routines fire <routine id>");
                2
            }
        },
        "routines" => {
            println!("{}", routines_json(&paths));
            0
        }
        "runs" => match runs_json(&paths, args.get(2).map(String::as_str)) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(e) => {
                eprintln!("{}", shown(&e));
                2
            }
        },
        "release-cron" => match crontab::release_all(&paths) {
            Ok(0) => {
                println!("no cron job is managed by herdr-routines; the crontab was left as it is");
                0
            }
            Ok(n) => {
                println!(
                    "{n} cron job(s) handed back to cron as plain crontab lines; a backup of the crontab is in {}",
                    shown(&paths.state.join("crontab-backups").to_string_lossy())
                );
                0
            }
            Err(e) => {
                eprintln!("{}", shown(&e));
                1
            }
        },
        "--version" | "-V" => {
            println!("herdr-routines {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!(
                "usage: herdr-routines [list|open [overlay|split|tab]|fire <id>|routines|runs [<id>]|release-cron|start|restart|stop|status|run|--version]"
            );
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_commands_print_every_setting_and_the_history() {
        let dir = std::env::temp_dir().join(format!("herdr-routines-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        };
        // A minimal file: what it leaves out is printed at its default.
        let file: routine::RoutineFile =
            toml::from_str("name = \"Report\"\nschedule = \"0 9 * * *\"\nretries = 2").unwrap();
        let r = Routine {
            id: "report".into(),
            file,
            prompt: "Write it.".into(),
            problem: None,
            unreadable: false,
            approved: true,
        };
        r.save_approved(&paths).unwrap();
        for n in 0..2 {
            let mut run = store::Run::new("report", "Report", "claude", Trigger::Manual, None);
            run.id = format!("20261101-08{n:02}00-000");
            run.result = format!("Result {n}\u{1b}[2J");
            run.finish(store::Status::Finished, "");
            run.save(&paths).unwrap();
        }
        let all: serde_json::Value = serde_json::from_str(&routines_json(&paths)).unwrap();
        let one = &all[0];
        for field in [
            "id",
            "name",
            "enabled",
            "schedule",
            "schedule_text",
            "agent",
            "permission_mode",
            "model",
            "effort",
            "args",
            "folder",
            "missed",
            "time_limit_minutes",
            "stop_at_limit",
            "retries",
            "retry_delay_minutes",
            "check_command",
            "remember",
            "alert",
            "runs_in",
            "keep_runs",
            "keep_days",
            "late_hours",
            "after_command",
            "after_always",
            "off_after_failures",
            "account",
            "failures_in_a_row",
            "problem",
            "approved",
            "next",
            "last_run",
        ] {
            assert!(one.get(field).is_some(), "{field} is missing");
        }
        assert_eq!(one["retries"], 2);
        assert_eq!(one["keep_runs"], routine::KEEP_RUNS);
        assert_eq!(one["missed"], "run-once");
        assert_eq!(one["runs_in"], "herdr");
        assert_eq!(one["last_run"]["id"], "20261101-080100-000");
        // The history, newest first; of one routine or of all.
        let text = runs_json(&paths, Some("report")).unwrap();
        assert!(!text.contains('\u{1b}'), "JSON escapes control characters");
        let runs: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 2);
        assert_eq!(runs[0]["id"], "20261101-080100-000");
        assert_eq!(runs[1]["status"], "finished");
        assert_eq!(runs[1]["trigger"], "manual");
        let all_runs: serde_json::Value =
            serde_json::from_str(&runs_json(&paths, None).unwrap()).unwrap();
        assert_eq!(all_runs.as_array().unwrap().len(), 2);
        assert_eq!(runs_json(&paths, Some("nobody")).unwrap(), "[]");
        assert!(runs_json(&paths, Some("../x")).is_err());
        // What goes to the terminal as text has no escape sequences.
        assert_eq!(shown("a\u{1b}[2Jb\nc"), "a[2Jb c");
        let _ = std::fs::remove_dir_all(dir);
    }
}
