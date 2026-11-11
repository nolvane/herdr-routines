//! Keeps the computer awake so a run is not cut off by idle sleep:
//! `caffeinate -i` on macOS, `systemd-inhibit` on Linux. Closing a laptop's
//! lid still puts it to sleep; nothing a program may do changes that.
//! Only the computer is kept awake: the screen goes dark as its settings
//! say (`caffeinate -d` would hold it on), and runs go on behind it.
//!
//! The helper watches the process that started it (`-w <pid>`), so it ends
//! with that process however it ends, and nothing is left holding the
//! computer awake.

use crate::paths::{self, Paths};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// When the computer is kept awake, as chosen in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Off,
    /// While a run is going.
    #[default]
    Runs,
    /// While the scheduler runs, so no time is missed for idle sleep.
    Always,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Runs, Mode::Always, Mode::Off];

    fn word(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Runs => "runs",
            Mode::Always => "always",
        }
    }

    /// As the chip at the top says it.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Off => "awake off",
            Mode::Runs => "awake during runs",
            Mode::Always => "awake always",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Mode::Off => "Off",
            Mode::Runs => "During runs",
            Mode::Always => "Always",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Mode::Off => "The computer sleeps as its own settings say, also in the middle of a run",
            Mode::Runs => "The computer stays awake while a run is going",
            Mode::Always => "The computer stays awake while Herdr runs, so no time is missed",
        }
    }
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.state.join("awake")
}

/// The chosen mode; "during runs" until the owner picks another.
pub fn mode(paths: &Paths) -> Mode {
    let text = std::fs::read_to_string(file(paths)).unwrap_or_default();
    Mode::ALL
        .into_iter()
        .find(|m| m.word() == text.trim())
        .unwrap_or_default()
}

pub fn set_mode(paths: &Paths, mode: Mode) -> Result<(), String> {
    paths::write_atomic(&file(paths), format!("{}\n", mode.word()).as_bytes())
        .map_err(|e| format!("could not save it: {e}"))
}

/// The helper's command line for this system, watching process `pid`.
/// None where there is no such helper.
fn helper(pid: u32) -> Option<(&'static str, Vec<String>)> {
    if cfg!(target_os = "macos") {
        let bin = "/usr/bin/caffeinate";
        return Path::new(bin)
            .exists()
            .then(|| (bin, vec!["-i".into(), "-w".into(), pid.to_string()]));
    }
    // systemd-inhibit holds the lock while its command runs: `tail --pid`
    // ends when the watched process does.
    ["/usr/bin/systemd-inhibit", "/bin/systemd-inhibit"]
        .into_iter()
        .find(|bin| Path::new(bin).exists())
        .map(|bin| {
            (
                bin,
                vec![
                    "--what=idle:sleep".into(),
                    "--who=herdr-routines".into(),
                    "--why=A routine is running".into(),
                    "--mode=block".into(),
                    "tail".into(),
                    format!("--pid={pid}"),
                    "-f".into(),
                    "/dev/null".into(),
                ],
            )
        })
}

/// Whether this system has a helper to keep it awake.
pub fn available() -> bool {
    helper(0).is_some()
}

/// Holds the computer awake while switched on. One per process.
#[derive(Default)]
pub struct Awake {
    child: Option<Child>,
}

impl Awake {
    /// Starts or ends the helper; calling it again with the same answer
    /// costs nothing. Returns whether the computer is now held awake.
    pub fn hold(&mut self, on: bool) -> bool {
        // A helper that ended by itself (killed from outside) is started
        // again when still wanted.
        if let Some(child) = self.child.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                self.child = None;
            }
        }
        match (on, self.child.is_some()) {
            (true, false) => {
                self.child = helper(std::process::id()).and_then(|(bin, args)| {
                    Command::new(bin)
                        .args(args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .ok()
                });
            }
            (false, true) => self.release(),
            _ => {}
        }
        self.child.is_some()
    }

    fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_kept_and_starts_as_during_runs() {
        let dir = std::env::temp_dir().join(format!("hr-awake-{}", std::process::id()));
        let paths = Paths {
            config: dir.join("config"),
            state: dir.join("state"),
            socket: dir.join("herdr.sock"),
        };
        std::fs::create_dir_all(&paths.state).unwrap();
        assert_eq!(mode(&paths), Mode::Runs);
        for m in Mode::ALL {
            set_mode(&paths, m).unwrap();
            assert_eq!(mode(&paths), m);
        }
        // Anything else in the file reads as the default.
        std::fs::write(file(&paths), "forever\n").unwrap();
        assert_eq!(mode(&paths), Mode::Runs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_helper_watches_the_process_and_ends_when_released() {
        let Some((bin, args)) = helper(4242) else {
            return;
        };
        assert!(args.iter().any(|a| a.contains("4242")), "{bin} {args:?}");
        let mut awake = Awake::default();
        assert!(!awake.hold(false));
        assert!(awake.hold(true));
        let pid = awake.child.as_ref().map(Child::id).unwrap();
        // Asking again keeps the same helper.
        assert!(awake.hold(true));
        assert_eq!(awake.child.as_ref().map(Child::id), Some(pid));
        assert!(!awake.hold(false));
        assert!(awake.child.is_none());
    }
}
