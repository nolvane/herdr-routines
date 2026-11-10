//! Where the plugin keeps its files.
//!
//! - Config dir (`HERDR_PLUGIN_CONFIG_DIR`): `config.toml` with the settings
//!   and `routines/<id>/` with each routine's `routine.toml` and `prompt.md`.
//! - State dir (`HERDR_PLUGIN_STATE_DIR`): the scheduler's pid file and log,
//!   when each routine was last checked, and `runs/<id>/` with one record and
//!   one output log per run.

use std::path::{Path, PathBuf};

pub const PLUGIN_ID: &str = "herdr-routines";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub socket: PathBuf,
}

impl Paths {
    pub fn from_env() -> Paths {
        let config = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config/herdr/plugins/config").join(PLUGIN_ID));
        let state = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state/herdr/plugins").join(PLUGIN_ID));
        let socket = std::env::var_os("HERDR_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config/herdr/herdr.sock"));
        Paths {
            config,
            state,
            socket,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn routines_dir(&self) -> PathBuf {
        self.config.join("routines")
    }

    pub fn routine_dir(&self, id: &str) -> PathBuf {
        self.routines_dir().join(id)
    }

    pub fn runs_dir(&self, id: &str) -> PathBuf {
        self.state.join("runs").join(id)
    }

    pub fn all_runs_dir(&self) -> PathBuf {
        self.state.join("runs")
    }

    /// The pid file of the one scheduler, with the socket it serves. One
    /// scheduler for all Herdr servers, since they share the routines.
    pub fn pid_file(&self) -> PathBuf {
        self.state.join("scheduler.pid")
    }

    /// This Herdr server: its socket and the socket file's inode, which
    /// changes when the server restarts (and its pane ids start over).
    pub fn server_id(&self) -> String {
        server_id_of(&self.socket)
    }

    pub fn checked_file(&self) -> PathBuf {
        self.state.join("checked.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state.join("routines.log")
    }
}

fn server_id_of(socket: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    let ino = std::fs::metadata(socket).map(|m| m.ino()).unwrap_or(0);
    format!("{}#{ino}", socket.display())
}

/// Whether the server a run was started on still runs (same socket, same
/// server instance).
pub fn server_alive(server_id: &str) -> bool {
    match server_id.rsplit_once('#') {
        Some((socket, _)) if !socket.is_empty() => server_id_of(Path::new(socket)) == server_id,
        _ => false,
    }
}

/// FNV-1a: stable across builds, unlike the standard library's hasher.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// `~` and `~/…` to the home folder.
pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// The home folder as `~` for display.
pub fn shorten(path: &Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Writes through a temporary file and a rename, so a reader never sees half
/// a file.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Appends a timestamped line to the plugin log.
pub fn log(paths: &Paths, line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(&paths.state);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_file())
    {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "{now} [{}] {line}", std::process::id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home() {
        let h = home();
        assert_eq!(expand("~"), h);
        assert_eq!(expand("~/x"), h.join("x"));
        assert_eq!(expand("/tmp"), PathBuf::from("/tmp"));
        assert_eq!(shorten(&h.join("a/b")), "~/a/b");
    }
}
