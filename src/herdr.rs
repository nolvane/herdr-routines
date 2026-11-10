//! The Herdr command line, as the plugin uses it.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// The running Herdr binary, as Herdr hands it to plugins.
pub fn bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

/// A failed Herdr call: the error code (such as `pane_not_found`) and text.
#[derive(Debug, Clone)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.code.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} ({})", self.message, self.code)
        }
    }
}

fn run(args: &[&str]) -> Result<Vec<u8>, Error> {
    let out = Command::new(bin()).args(args).output().map_err(|e| Error {
        code: String::new(),
        message: format!("cannot run herdr: {e}"),
    })?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let v: Value = serde_json::from_slice(&out.stderr).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or("").to_string();
    let message = v["error"]["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_string());
    Err(Error { code, message })
}

fn call(args: &[&str]) -> Result<Value, Error> {
    let stdout = run(args)?;
    Ok(serde_json::from_slice(&stdout).unwrap_or(Value::Null))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// Where a new run lives.
#[derive(Debug, Clone, Default)]
pub struct Place {
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    /// Herdr's random id of the pane's terminal (see `Run::terminal_id`).
    pub terminal_id: String,
}

/// What Herdr says about a pane now: its tab and its terminal.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaneInfo {
    pub tab_id: String,
    pub terminal_id: String,
}

/// The pane's tab and terminal, or `None` when the pane is gone.
pub fn pane_info(pane: &str) -> Result<Option<PaneInfo>, Error> {
    match call(&["pane", "get", pane]) {
        Ok(v) => Ok(Some(pane_info_from(&v))),
        Err(e) if e.code == "pane_not_found" => Ok(None),
        Err(e) => Err(e),
    }
}

fn pane_info_from(v: &Value) -> PaneInfo {
    let p = &v["result"]["pane"];
    PaneInfo {
        tab_id: s(&p["tab_id"]),
        terminal_id: s(&p["terminal_id"]),
    }
}

/// The workspace with this label, if one exists.
pub fn find_workspace(label: &str) -> Result<Option<String>, Error> {
    let v = call(&["workspace", "list"])?;
    Ok(v["result"]["workspaces"].as_array().and_then(|ws| {
        ws.iter()
            .find(|w| w["label"].as_str() == Some(label))
            .map(|w| s(&w["workspace_id"]))
    }))
}

/// Whether the workspace with this label is the one on screen.
pub fn workspace_label_focused(label: &str) -> Result<bool, Error> {
    let v = call(&["workspace", "list"])?;
    Ok(v["result"]["workspaces"].as_array().is_some_and(|ws| {
        ws.iter()
            .any(|w| w["label"].as_str() == Some(label) && w["focused"].as_bool() == Some(true))
    }))
}

/// A new tab for a run: in the workspace with this label, which is created
/// (with the run in its first tab) when it does not exist yet.
pub fn open_tab(workspace_label: &str, cwd: &str, tab_label: &str) -> Result<Place, Error> {
    // Launches run in threads: one at a time here, so two due routines do
    // not both create the workspace.
    static OPENING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = OPENING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ws) = find_workspace(workspace_label)? {
        let v = call(&[
            "tab",
            "create",
            "--workspace",
            &ws,
            "--cwd",
            cwd,
            "--label",
            tab_label,
            "--no-focus",
        ])?;
        return Ok(Place {
            workspace_id: ws,
            tab_id: s(&v["result"]["tab"]["tab_id"]),
            pane_id: s(&v["result"]["root_pane"]["pane_id"]),
            terminal_id: s(&v["result"]["root_pane"]["terminal_id"]),
        });
    }
    let v = call(&[
        "workspace",
        "create",
        "--cwd",
        cwd,
        "--label",
        workspace_label,
        "--no-focus",
    ])?;
    let place = Place {
        workspace_id: s(&v["result"]["workspace"]["workspace_id"]),
        tab_id: s(&v["result"]["tab"]["tab_id"]),
        pane_id: s(&v["result"]["root_pane"]["pane_id"]),
        terminal_id: s(&v["result"]["root_pane"]["terminal_id"]),
    };
    let _ = call(&["tab", "rename", &place.tab_id, tab_label]);
    Ok(place)
}

/// A new tab in the workspace the list runs in, shown right away, with
/// these environment variables (`KEY=VALUE`).
pub fn open_focused_tab(cwd: &str, tab_label: &str, env: &[String]) -> Result<Place, Error> {
    let ws = std::env::var("HERDR_WORKSPACE_ID").unwrap_or_default();
    let mut a = vec![
        "tab", "create", "--cwd", cwd, "--label", tab_label, "--focus",
    ];
    if !ws.is_empty() {
        a.extend(["--workspace", ws.as_str()]);
    }
    for e in env {
        a.extend(["--env", e.as_str()]);
    }
    let v = call(&a)?;
    Ok(Place {
        workspace_id: s(&v["result"]["tab"]["workspace_id"]),
        tab_id: s(&v["result"]["tab"]["tab_id"]),
        pane_id: s(&v["result"]["root_pane"]["pane_id"]),
        terminal_id: s(&v["result"]["root_pane"]["terminal_id"]),
    })
}

pub fn agent_start(name: &str, kind: &str, pane: &str, args: &[String]) -> Result<(), Error> {
    let mut a: Vec<&str> = vec![
        "agent",
        "start",
        name,
        "--kind",
        kind,
        "--pane",
        pane,
        "--timeout",
        "60000",
    ];
    if !args.is_empty() {
        a.push("--");
        a.extend(args.iter().map(String::as_str));
    }
    call(&a).map(|_| ())
}

/// Types a command line into the pane's shell and runs it.
pub fn pane_run(pane: &str, command: &str) -> Result<(), Error> {
    call(&["pane", "run", pane, command]).map(|_| ())
}

pub fn agent_prompt(target: &str, text: &str) -> Result<(), Error> {
    call(&["agent", "prompt", target, text]).map(|_| ())
}

/// The agent's state (`working`, `blocked`, `idle`, `done`, `unknown`), or
/// `None` when no agent is in the pane any more.
pub fn agent_status(pane: &str) -> Result<Option<String>, Error> {
    agent_state(pane).map(|a| a.map(|(status, _)| status))
}

/// The agent's state and how many turns it has completed so far (Herdr's
/// `completion_seq`), or `None` when no agent is in the pane any more.
pub fn agent_state(pane: &str) -> Result<Option<(String, u64)>, Error> {
    match call(&["agent", "get", pane]) {
        Ok(v) => {
            let a = &v["result"]["agent"];
            Ok(Some((
                s(&a["agent_status"]),
                a["completion_seq"].as_u64().unwrap_or(0),
            )))
        }
        Err(e) if e.code == "agent_not_found" || e.code == "pane_not_found" => Ok(None),
        Err(e) => Err(e),
    }
}

/// The last lines of the pane's output, soft wraps joined.
pub fn read(pane: &str, lines: u32) -> Result<String, Error> {
    let n = lines.to_string();
    run(&[
        "pane",
        "read",
        pane,
        "--source",
        "recent-unwrapped",
        "--lines",
        &n,
    ])
    .map(|b| String::from_utf8_lossy(&b).to_string())
}

pub fn close_tab(tab: &str) -> Result<(), Error> {
    call(&["tab", "close", tab]).map(|_| ())
}

/// Shows the pane: its workspace, its tab, then the pane itself.
pub fn focus(place: &Place) -> Result<(), Error> {
    if !place.workspace_id.is_empty() {
        call(&["workspace", "focus", &place.workspace_id])?;
    }
    if !place.tab_id.is_empty() {
        call(&["tab", "focus", &place.tab_id])?;
    }
    if !place.pane_id.is_empty() {
        let _ = call(&["agent", "focus", &place.pane_id]);
    }
    Ok(())
}

/// A Herdr notification. `sound` is `none`, `done` or `request`.
/// Opens one of the plugin's panes as a popup over whatever is in front,
/// with the focus. False when Herdr cannot.
pub fn open_popup(entrypoint: &str, width: u16, height: u16, env: &[(&str, &str)]) -> bool {
    let (w, h) = (width.to_string(), height.to_string());
    let mut args: Vec<String> = [
        "plugin",
        "pane",
        "open",
        "--plugin",
        "herdr-routines",
        "--entrypoint",
        entrypoint,
        "--placement",
        "popup",
        "--width",
        &w,
        "--height",
        &h,
        "--focus",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for (k, v) in env {
        args.push("--env".into());
        args.push(format!("{k}={v}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs).is_ok()
}

/// Where right clicks in this process's own pane go: `true` to the pane
/// (the list's menu), `false` back to Herdr's pane menu. Herdr keeps them
/// for its own menu unless a pane asks. Nothing outside a Herdr pane.
pub fn right_clicks_to_me(mine: bool) -> bool {
    let Ok(pane) = std::env::var("HERDR_PANE_ID") else {
        return false;
    };
    let target = if mine { "pane" } else { "herdr" };
    run(&["pane", "input", &pane, "--right-click", target]).is_ok()
}

pub fn notify(title: &str, body: &str, sound: &str) {
    let _ = call(&[
        "notification",
        "show",
        title,
        "--body",
        body,
        "--sound",
        sound,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Herdr 0.9.5's answers, trimmed (taken from a throwaway session).
    #[test]
    fn reads_tab_and_terminal_of_a_pane() {
        let get = r#"{"id":"cli:pane:get","result":{"pane":{"agent_status":"unknown","pane_id":"w1:p1","tab_id":"w1:t1","terminal_id":"term_65ff00fc5d6f11","workspace_id":"w1"},"type":"pane_info"}}"#;
        let v: Value = serde_json::from_str(get).unwrap();
        assert_eq!(
            pane_info_from(&v),
            PaneInfo {
                tab_id: "w1:t1".into(),
                terminal_id: "term_65ff00fc5d6f11".into()
            }
        );
        let created = r#"{"result":{"root_pane":{"pane_id":"w1:p1","tab_id":"w1:t1","terminal_id":"term_65ff00fc5d6f11"},"tab":{"tab_id":"w1:t1"}}}"#;
        let v: Value = serde_json::from_str(created).unwrap();
        assert_eq!(
            s(&v["result"]["root_pane"]["terminal_id"]),
            "term_65ff00fc5d6f11"
        );
    }
}
