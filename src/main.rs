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
//! - `fire <id>` starts a routine now
//! - `routines` prints the routines and their latest run as JSON

mod agent;
mod app;
mod cron;
mod editor;
mod form;
mod herdr;
mod pane;
mod paths;
mod routine;
mod runner;
mod scheduler;
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
    let mut terminal = enter_terminal();
    let result = app.run(&mut terminal);
    leave_terminal();
    pane::forget_self();
    result
}

/// Starts a routine now, in this process, and waits until its agent is up.
fn fire(paths: &Paths, id: &str) -> Result<String, String> {
    let routine = Routine::load(paths, id)?;
    if let Some(p) = &routine.problem {
        return Err(format!("cannot run {id}: {p}"));
    }
    if scheduler::busy(paths, id) {
        return Err(format!("{id} is already running"));
    }
    let settings = Settings::load(paths);
    let run = runner::prepare(paths, &routine, Trigger::Manual, None);
    let run = runner::launch(paths, &settings, &routine, run);
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

fn routines_json(paths: &Paths) -> String {
    let now = chrono::Local::now();
    let items: Vec<serde_json::Value> = routine::load_all(paths)
        .iter()
        .map(|r| {
            let last = store::runs_of(paths, &r.id).into_iter().next();
            serde_json::json!({
                "id": r.id,
                "name": r.file.name,
                "enabled": r.file.enabled,
                "schedule": r.file.schedule,
                "schedule_text": r.schedule_text(),
                "agent": r.file.agent,
                "permission_mode": r.file.permission_mode,
                "model": r.file.model,
                "effort": r.file.effort,
                "folder": r.file.folder,
                "problem": r.problem,
                "next": r.cron().filter(|_| r.file.enabled).and_then(|c| c.next_after(&now)).map(|t| t.to_rfc3339()),
                "last_run": last.map(|l| serde_json::json!({
                    "id": l.id, "status": l.status, "started": l.started.to_rfc3339(),
                    "ended": l.ended.map(|t| t.to_rfc3339()), "message": l.message,
                })),
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

fn main() {
    let command = std::env::args().nth(1).unwrap_or_else(|| "list".into());
    let paths = Paths::from_env();
    let _ = std::fs::create_dir_all(&paths.state);
    let code = match command.as_str() {
        "start" => {
            if let Some(pid) = scheduler::keep_running(&paths) {
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
                Some((pid, socket)) => println!("running, pid {pid}, socket {socket}"),
                None => println!("not running"),
            }
            0
        }
        "run" => scheduler::run(&paths),
        "list" => match list(paths) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("herdr-routines: {e}");
                1
            }
        },
        "open" => {
            let settings = Settings::load(&paths);
            let placement = match std::env::args().nth(2).as_deref() {
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
        "fire" => match std::env::args().nth(2) {
            Some(id) => match fire(&paths, &id) {
                Ok(msg) => {
                    println!("{msg}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
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
        "--version" | "-V" => {
            println!("herdr-routines {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!(
                "usage: herdr-routines [list|open [overlay|split|tab]|fire <id>|routines|start|restart|stop|status|run|--version]"
            );
            2
        }
    };
    std::process::exit(code);
}
