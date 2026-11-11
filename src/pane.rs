//! The `open` plugin action: opens the list as a Herdr plugin pane, or
//! closes it when the current tab already has one.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a Herdr call of the action may take.
const LIMIT: Duration = Duration::from_secs(20);

/// The panes this plugin opened, per Herdr server (`paths::server_id_of`:
/// its socket and the socket file's inode). Pane ids start over when a
/// server restarts, so ids remembered for an earlier server must never be
/// used on the next one, where they can mean someone else's pane.
type Opened = BTreeMap<String, Vec<String>>;

fn state_dir() -> Option<PathBuf> {
    std::env::var_os("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from)
}

fn server() -> String {
    let socket = std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_default();
    crate::paths::server_id_of(&socket)
}

fn file(dir: &Path) -> PathBuf {
    dir.join("open-panes.json")
}

fn load(dir: &Path) -> Opened {
    crate::paths::read_limited(&file(dir), crate::paths::FILE_LIMIT)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn read_ids_of(dir: &Path, server: &str) -> Vec<String> {
    load(dir).remove(server).unwrap_or_default()
}

/// Remembers this server's panes and forgets servers that are gone.
fn write_ids_of(dir: &Path, server: &str, ids: &[String]) {
    let mut all = load(dir);
    all.retain(|s, _| s != server && crate::paths::server_alive(s));
    if !ids.is_empty() {
        all.insert(server.to_string(), ids.to_vec());
    }
    if let Ok(text) = serde_json::to_vec_pretty(&all) {
        let _ = crate::paths::write_atomic(&file(dir), &text);
    }
    // Files of versions that kept one list per socket path.
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("open-panes-") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn read_ids() -> Vec<String> {
    state_dir()
        .map(|d| read_ids_of(&d, &server()))
        .unwrap_or_default()
}

fn write_ids(ids: &[String]) {
    if let Some(d) = state_dir() {
        write_ids_of(&d, &server(), ids);
    }
}

/// Where the action was invoked.
struct Context {
    pane: Option<String>,
    tab: Option<String>,
    cwd: Option<String>,
}

fn context() -> Context {
    let ctx: Value = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let get = |k: &str| ctx.get(k).and_then(Value::as_str).map(str::to_string);
    Context {
        pane: get("focused_pane_id").or_else(|| std::env::var("HERDR_PANE_ID").ok()),
        tab: get("tab_id").or_else(|| std::env::var("HERDR_TAB_ID").ok()),
        cwd: get("focused_pane_cwd").or_else(|| get("workspace_cwd")),
    }
}

/// Every live pane and the tab it is in.
fn live_panes() -> Option<HashMap<String, String>> {
    let out = crate::herdr::output(&["pane", "list"], LIMIT).ok()?;
    if !out.success {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(
        v["result"]["panes"]
            .as_array()?
            .iter()
            .filter_map(|p| {
                Some((
                    p["pane_id"].as_str()?.to_string(),
                    p["tab_id"].as_str().unwrap_or("").to_string(),
                ))
            })
            .collect(),
    )
}

/// `placement` is `overlay`, `split` or `tab`; `direction` the side of a split.
pub fn toggle(placement: &str, direction: &str) -> Result<String, String> {
    let ctx = context();

    // Forget lists that are gone.
    let mut ids = read_ids();
    if let Some(live) = live_panes() {
        ids.retain(|id| live.contains_key(id));

        // Close the list in this tab, or the list itself when it is focused.
        let here: Vec<String> = ids
            .iter()
            .filter(|id| Some(*id) == ctx.pane.as_ref() || live.get(*id) == ctx.tab.as_ref())
            .cloned()
            .collect();
        if !here.is_empty() {
            for id in &here {
                let _ = crate::herdr::output(&["plugin", "pane", "close", id], LIMIT);
            }
            ids.retain(|id| !here.contains(id));
            write_ids(&ids);
            return Ok(format!("closed {}", here.join(", ")));
        }
    }

    let mut args = vec![
        "plugin",
        "pane",
        "open",
        "--plugin",
        crate::paths::PLUGIN_ID,
        "--entrypoint",
        "list",
        "--placement",
        placement,
        "--focus",
    ];
    if let Some(cwd) = &ctx.cwd {
        args.extend(["--cwd", cwd.as_str()]);
    }
    if placement == "split" {
        args.extend(["--direction", direction]);
        if let Some(pane) = &ctx.pane {
            args.extend(["--target-pane", pane.as_str()]);
        }
    }
    let output = crate::herdr::output(&args, LIMIT).map_err(|e| format!("herdr: {e}"))?;
    if !output.success {
        return Err(format!(
            "could not open the list: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let id = answer["result"]["plugin_pane"]["pane"]["pane_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if !id.is_empty() {
        ids.push(id.clone());
        write_ids(&ids);
    }
    Ok(format!("opened {id}"))
}

/// Called when the list closes itself (`q`): forgets its pane.
pub fn forget_self() {
    let Ok(me) = std::env::var("HERDR_PANE_ID") else {
        return;
    };
    let mut ids = read_ids();
    let before = ids.len();
    ids.retain(|id| *id != me);
    if ids.len() != before {
        write_ids(&ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_ids_belong_to_one_server() {
        let dir = std::env::temp_dir().join(format!("rt-panes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A socket file stands in for a running server.
        let socket = dir.join("herdr.sock");
        std::fs::write(&socket, b"").unwrap();
        let first = crate::paths::server_id_of(&socket);
        std::fs::write(dir.join("open-panes-0123456789abcdef"), "w1:p3\n").unwrap();
        write_ids_of(&dir, &first, &["w1:p3".to_string()]);
        assert_eq!(read_ids_of(&dir, &first), vec!["w1:p3"]);
        assert!(!dir.join("open-panes-0123456789abcdef").exists());
        // The server restarts: a new socket file, and pane ids start over.
        std::fs::rename(&socket, dir.join("old.sock")).unwrap();
        std::fs::write(&socket, b"").unwrap();
        let second = crate::paths::server_id_of(&socket);
        assert_ne!(first, second);
        assert!(
            read_ids_of(&dir, &second).is_empty(),
            "ids of the earlier server are never used on the next one"
        );
        std::fs::remove_file(dir.join("old.sock")).unwrap();
        write_ids_of(&dir, &second, &["w1:p9".to_string()]);
        assert_eq!(load(&dir).len(), 1, "the gone server is forgotten");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
