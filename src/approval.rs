//! The owner's approval of each routine. A routine runs only in the exact
//! form the owner saved in the list or approved there with `a`. Files that
//! anything else wrote (an agent, an editor outside the list, a sync tool)
//! wait for that approval, since an agent could otherwise give a routine
//! more authority than the owner chose, such as a stronger permission mode
//! or a shell command, and the scheduler would run it unattended.
//!
//! The approved content of each file is kept in `approved.json` in the
//! state folder (owner-only, like every file of the plugin) and compared
//! byte for byte with the files on disk.

use crate::paths::{self, Paths};
use crate::routine::Routine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The files of a routine that decide what it does.
const FILES: [&str; 3] = ["routine.toml", "prompt.md", "command.sh"];

/// Approved routines: routine id, then file name and its approved content.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approvals(pub BTreeMap<String, BTreeMap<String, String>>);

/// A routine's files as they are on disk now; missing files are left out.
pub type Snapshot = BTreeMap<String, String>;

impl Approvals {
    pub fn load(paths: &Paths) -> Approvals {
        std::fs::read(paths.approvals_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, paths: &Paths) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        paths::write_atomic(&paths.approvals_file(), &text)
            .map_err(|e| format!("cannot save the approval: {e}"))
    }

    /// Whether these files, read once, are the ones the owner approved:
    /// `routine.toml` and the text file its agent uses. The caller runs
    /// exactly these files, so what is checked is what runs.
    pub fn approves(&self, id: &str, agent: &str, files: &Snapshot) -> bool {
        let Some(approved) = self.0.get(id) else {
            return false;
        };
        ["routine.toml", Routine::text_file(agent)]
            .iter()
            .all(|f| approved.get(*f).is_some() && approved.get(*f) == files.get(*f))
    }
}

/// Changes `approved.json` under its lock, so two lists approving at the
/// same moment do not drop each other's approval.
fn update(paths: &Paths, change: impl FnOnce(&mut Approvals) -> bool) -> Result<(), String> {
    paths::locked(&paths.approvals_lock_file(), || {
        let mut all = Approvals::load(paths);
        if change(&mut all) {
            all.save(paths)
        } else {
            Ok(())
        }
    })
}

/// Only the files that decide what the routine does: `routine.toml` and the
/// text file of its agent. Another text file left in the folder (from
/// before a switch between agent and shell) is never approved along.
fn deciding(files: Snapshot) -> Snapshot {
    let agent = files
        .get("routine.toml")
        .and_then(|t| toml::from_str::<crate::routine::RoutineFile>(t).ok())
        .map(|f| f.agent);
    let text = agent.as_deref().map(Routine::text_file);
    files
        .into_iter()
        .filter(|(name, _)| name == "routine.toml" || Some(name.as_str()) == text)
        .collect()
}

/// The routine's files as they are on disk now, each read once.
pub fn snapshot(paths: &Paths, id: &str) -> Snapshot {
    let dir = paths.routine_dir(id);
    FILES
        .iter()
        .filter_map(|f| {
            std::fs::read_to_string(dir.join(f))
                .ok()
                .map(|t| (f.to_string(), t))
        })
        .collect()
}

/// Approves exactly these files of the routine (shown to the owner or
/// written by the list), replacing what was approved before.
pub fn approve(paths: &Paths, id: &str, files: Snapshot) -> Result<(), String> {
    let files = deciding(files);
    update(paths, |all| {
        all.0.insert(id.to_string(), files);
        true
    })
}

/// Approves one file the list just wrote, keeping the approval of the
/// routine's other files. Does nothing for a routine that waits for
/// approval, so saving one file never approves another file that something
/// else changed.
pub fn approve_file(
    paths: &Paths,
    id: &str,
    was_approved: bool,
    file: &str,
    text: &str,
) -> Result<(), String> {
    if !was_approved {
        return Ok(());
    }
    update(paths, |all| match all.0.get_mut(id) {
        Some(files) => {
            files.insert(file.to_string(), text.to_string());
            true
        }
        None => false,
    })
}

/// Forgets a deleted routine.
pub fn forget(paths: &Paths, id: &str) {
    let _ = update(paths, |all| all.0.remove(id).is_some());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(tag: &str) -> Paths {
        let base: PathBuf = std::env::temp_dir().join(format!(
            "herdr-routines-approval-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        Paths {
            config: base.join("config"),
            state: base.join("state"),
            socket: base.join("sock"),
        }
    }

    fn write(p: &Paths, id: &str, file: &str, text: &str) {
        paths::write_atomic(&p.routine_dir(id).join(file), text.as_bytes()).unwrap();
    }

    const TOML: &str = "name = \"A\"\nschedule = \"0 9 * * *\"\n";

    fn approved(p: &Paths, agent: &str) -> bool {
        Approvals::load(p).approves("a", agent, &snapshot(p, "a"))
    }

    #[test]
    fn a_change_outside_the_list_needs_approval_again() {
        let p = paths("change");
        write(&p, "a", "routine.toml", TOML);
        write(&p, "a", "prompt.md", "Say hi");
        assert!(!approved(&p, "claude"));
        approve(&p, "a", snapshot(&p, "a")).unwrap();
        assert!(approved(&p, "claude"));
        // Something else changes the prompt: it waits again.
        write(&p, "a", "prompt.md", "Say bye");
        assert!(!approved(&p, "claude"));
        // A command file lying next to an agent routine is not approved
        // along, so switching to a shell command needs its own approval.
        write(&p, "a", "command.sh", "echo hi");
        approve(&p, "a", snapshot(&p, "a")).unwrap();
        assert!(!Approvals::load(&p).0["a"].contains_key("command.sh"));
        assert!(!approved(&p, "shell"));
        let _ = std::fs::remove_dir_all(p.config.parent().unwrap());
    }

    #[test]
    fn saving_one_file_keeps_another_change_waiting() {
        let p = paths("one-file");
        write(&p, "a", "routine.toml", TOML);
        write(&p, "a", "prompt.md", "Say hi");
        approve(&p, "a", snapshot(&p, "a")).unwrap();
        // An agent changes routine.toml; the owner then edits the prompt.
        write(&p, "a", "routine.toml", &format!("{TOML}args = [\"x\"]\n"));
        let was = approved(&p, "claude");
        write(&p, "a", "prompt.md", "Say hello");
        approve_file(&p, "a", was, "prompt.md", "Say hello").unwrap();
        assert!(!approved(&p, "claude"));
        forget(&p, "a");
        assert!(Approvals::load(&p).0.is_empty());
        let _ = std::fs::remove_dir_all(p.config.parent().unwrap());
    }
}
