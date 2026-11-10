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

    /// The routines the owner approved, with the approved file contents.
    pub fn approvals_file(&self) -> PathBuf {
        self.state.join("approved.json")
    }

    /// Held while a process checks whether a routine is free and records
    /// its new run, so two processes cannot both start it.
    pub fn lock_file(&self, id: &str) -> PathBuf {
        self.runs_dir(id).join(".lock")
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

/// Creates a folder and its missing parents readable only by the owner
/// (`0700`). Folders that already exist keep their mode.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Makes the plugin's own config and state folders owner-only, also when an
/// older version created them readable by others. Their files hold prompts,
/// commands and agent output, which can contain secrets.
pub fn secure_roots(paths: &Paths) {
    use std::os::unix::fs::PermissionsExt;
    for dir in [&paths.config, &paths.state] {
        if create_private_dir(dir).is_ok() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
}

/// Opens a new file readable only by the owner (`0600`). Fails when the
/// name exists, a link included, so it never writes through a planted link.
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Writes through a temporary file and a rename, so a reader never sees half
/// a file. The file is readable only by the owner.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    create_private_dir(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.tmp{}-{nanos}", std::process::id()));
    let written = create_private_file(&tmp).and_then(|mut f| f.write_all(content));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Appends a timestamped line to the plugin log.
pub fn log(paths: &Paths, line: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let _ = create_private_dir(&paths.state);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(paths.log_file())
    {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "{now} [{}] {line}", std::process::id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("herdr-routines-paths-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn writes_are_owner_only_and_never_follow_links() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("modes");
        let f = d.join("runs/x/1.log");
        write_atomic(&f, b"secret").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&f), 0o600);
        assert_eq!(mode(&d.join("runs/x")), 0o700);
        // A link planted where the target goes is replaced, not written through.
        let outside = d.join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        let g = d.join("runs/x/2.log");
        std::os::unix::fs::symlink(&outside, &g).unwrap();
        write_atomic(&g, b"new").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        assert_eq!(std::fs::read(&g).unwrap(), b"new");
        // No temporary files are left behind.
        let left: Vec<_> = std::fs::read_dir(d.join("runs/x"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(left.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn secure_roots_tightens_existing_folders() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("roots");
        let p = Paths {
            config: d.join("config"),
            state: d.join("state"),
            socket: d.join("sock"),
        };
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::set_permissions(&p.config, std::fs::Permissions::from_mode(0o755)).unwrap();
        secure_roots(&p);
        for dir in [&p.config, &p.state] {
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn expands_home() {
        let h = home();
        assert_eq!(expand("~"), h);
        assert_eq!(expand("~/x"), h.join("x"));
        assert_eq!(expand("/tmp"), PathBuf::from("/tmp"));
        assert_eq!(shorten(&h.join("a/b")), "~/a/b");
    }
}
