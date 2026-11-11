//! What a routine can set for its agent beyond the prompt: the permission
//! mode, the model and the effort, and the command line flags they become.
//! Only agents whose flags are known get these choices; every agent takes
//! extra arguments as they are.

use crate::routine::RoutineFile;

/// One entry of a choice: the value stored in `routine.toml`, its name in
/// the list, and a line on what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
}

const fn c(value: &'static str, label: &'static str, detail: &'static str) -> Choice {
    Choice {
        value,
        label,
        detail,
    }
}

/// The empty value: no flag, so the agent's own settings apply.
const DEFAULT: Choice = c("", "Agent default", "What the agent's own settings say");

const CLAUDE_MODES: &[Choice] = &[
    DEFAULT,
    c(
        "auto",
        "Auto",
        "Works without asking; a safety check stops risky steps",
    ),
    c(
        "acceptEdits",
        "Accept edits",
        "Changes files without asking, asks before commands",
    ),
    c("plan", "Plan only", "Reads and plans, changes nothing"),
    c(
        "dontAsk",
        "Don't ask",
        "Does only what your allow rules permit, refuses the rest",
    ),
    c(
        "bypassPermissions",
        "Bypass permissions",
        "Does everything without asking or checking (risky)",
    ),
    c(
        "manual",
        "Ask every time",
        "Asks before each change, so the run waits for you",
    ),
];

const CODEX_MODES: &[Choice] = &[
    DEFAULT,
    c(
        "auto",
        "Auto",
        "Works in the folder without asking (--sandbox workspace-write -a never)",
    ),
    c(
        "read-only",
        "Read only",
        "Reads, changes nothing (--sandbox read-only)",
    ),
    c(
        "full-access",
        "Full access",
        "Does everything without asking or a sandbox (risky)",
    ),
];

const GEMINI_MODES: &[Choice] = &[
    DEFAULT,
    c(
        "auto_edit",
        "Accept edits",
        "Changes files without asking (--approval-mode auto_edit)",
    ),
    c(
        "yolo",
        "Auto",
        "Does everything without asking (--approval-mode yolo)",
    ),
];

const CLAUDE_MODELS: &[Choice] = &[
    DEFAULT,
    c("fable", "Fable", "The latest Fable model"),
    c("opus", "Opus", "The latest Opus model"),
    c("sonnet", "Sonnet", "The latest Sonnet model"),
    c("haiku", "Haiku", "The latest Haiku model, fast and light"),
];

/// Gemini CLI's documented aliases.
const GEMINI_MODELS: &[Choice] = &[
    DEFAULT,
    c("auto", "Auto", "Gemini picks the model per task"),
    c("pro", "Pro", "The current Pro model"),
    c("flash", "Flash", "The current Flash model, faster"),
    c("flash-lite", "Flash Lite", "The lightest, fastest model"),
];

/// Kilo's own routing choices.
const KILO_MODELS: &[Choice] = &[
    DEFAULT,
    c(
        "kilo-auto/frontier",
        "Auto: frontier",
        "Kilo picks a top model",
    ),
    c(
        "kilo-auto/balanced",
        "Auto: balanced",
        "Kilo balances quality and cost",
    ),
    c("kilo-auto/free", "Auto: free", "Kilo picks a free model"),
];

/// Qoder's model tiers.
const QODER_MODELS: &[Choice] = &[
    DEFAULT,
    c("auto", "Auto", "Qoder picks the tier"),
    c("lite", "Lite", ""),
    c("efficient", "Efficient", ""),
    c("performance", "Performance", ""),
    c("ultimate", "Ultimate", ""),
];

/// Agents that take `--model` without stable names to suggest: the list
/// comes from the agent itself where it can print one, from other
/// routines, or is typed.
const TYPED_MODELS: &[Choice] = &[DEFAULT];

/// Agents that take no model on their command line (or none is known):
/// amp chooses through its modes, the others only inside a session.
const NO_MODEL_FLAG: &[&str] = &["amp", "mastracode", "maki", "muse"];

/// The command that prints an agent's models, for agents that have one.
/// Run in the person's login shell with a time limit; the output is read
/// by `parse_models`.
pub fn list_command(agent: &str) -> Option<&'static str> {
    Some(match agent {
        "codex" => "codex debug models",
        "cursor" => "cursor-agent models",
        "opencode" => "opencode models",
        "kilo" => "kilo models",
        "kiro" => "kiro-cli chat --list-models",
        "devin" => "devin models list",
        "pi" => "pi --list-models",
        "grok" => "grok models",
        "agy" => "agy models",
        _ => return None,
    })
}

/// Model names from a list command's output: JSON (objects with an `id`,
/// `slug`, `model` or `name`) or text with the name first on each line.
/// Headers, bullets and markers such as "(current)" are skipped.
pub fn parse_models(out: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        let s = s.trim();
        let ok = (2..=100).contains(&s.len())
            && s.chars().any(|c| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:/@#+-".contains(c));
        if ok && !found.iter().any(|f| f == s) {
            found.push(s.to_string());
        }
    };
    let trimmed = out.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            fn walk(v: &serde_json::Value, add: &mut dyn FnMut(&str)) {
                match v {
                    serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, add)),
                    serde_json::Value::Object(o) => {
                        let key = ["slug", "id", "model_id", "model", "name"]
                            .iter()
                            .find_map(|k| o.get(*k).and_then(|x| x.as_str()));
                        if let Some(k) = key {
                            add(k);
                        } else {
                            o.values().for_each(|x| walk(x, add));
                        }
                    }
                    _ => {}
                }
            }
            walk(&v, &mut add);
            return found;
        }
    }
    for line in out.lines() {
        let line = line.trim().trim_start_matches(['*', '-', '•', '>', ' ']);
        let Some(first) = line.split_whitespace().next() else {
            continue;
        };
        // Headers end in a colon or are plain words like "Available".
        let plain_words = !first.contains(['-', '/', '.', ':', '_'])
            && !line.contains(" - ")
            && line.split_whitespace().count() > 1;
        if first.ends_with(':') || plain_words {
            continue;
        }
        add(first);
    }
    found
}

const CLAUDE_EFFORTS: &[Choice] = &[
    DEFAULT,
    c("low", "Low", "Fastest, least thinking"),
    c("medium", "Medium", ""),
    c("high", "High", ""),
    c("xhigh", "Extra high", ""),
    c("max", "Max", "Most thinking, slowest"),
];

/// The permission modes an agent offers; empty when the plugin does not
/// know its flags.
pub fn modes(agent: &str) -> &'static [Choice] {
    match agent {
        "claude" => CLAUDE_MODES,
        "codex" => CODEX_MODES,
        "gemini" => GEMINI_MODES,
        _ => &[],
    }
}

/// Suggested models; empty when the agent takes no model here. Any name
/// can be typed for every agent that takes one, and agents that print
/// their models add them (see `list_command`).
pub fn models(agent: &str) -> &'static [Choice] {
    match agent {
        crate::routine::SHELL => &[],
        a if NO_MODEL_FLAG.contains(&a) => &[],
        "claude" => CLAUDE_MODELS,
        "gemini" => GEMINI_MODELS,
        "kilo" => KILO_MODELS,
        "qodercli" => QODER_MODELS,
        _ => TYPED_MODELS,
    }
}

pub fn efforts(agent: &str) -> &'static [Choice] {
    match agent {
        "claude" => CLAUDE_EFFORTS,
        _ => &[],
    }
}

/// The entry for a value, or `None` for a value the list does not have.
pub fn find(list: &'static [Choice], value: &str) -> Option<&'static Choice> {
    list.iter().find(|c| c.value == value)
}

/// How a value shows in the list and the form.
pub fn label(list: &'static [Choice], value: &str) -> String {
    match find(list, value) {
        Some(c) => c.label.to_string(),
        None => value.to_string(),
    }
}

/// What a new routine for this agent starts with: Auto where there is one,
/// since a routine runs while nobody watches.
pub fn default_mode(agent: &str) -> &'static str {
    match agent {
        "claude" | "codex" => "auto",
        _ => "",
    }
}

/// The flags for the agent's command line: permission mode, model and
/// effort, then the routine's own extra arguments.
pub fn flags(file: &RoutineFile) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |items: &[&str]| out.extend(items.iter().map(|s| s.to_string()));
    let mode = file.permission_mode.as_str();
    match (file.agent.as_str(), mode) {
        (_, "") => {}
        ("claude", m) => add(&["--permission-mode", m]),
        // --full-auto is gone from current Codex; this is what it meant
        // for a run nobody watches.
        ("codex", "auto") => add(&[
            "--sandbox",
            "workspace-write",
            "--ask-for-approval",
            "never",
        ]),
        ("codex", "read-only") => add(&["--sandbox", "read-only"]),
        ("codex", "full-access") => add(&["--dangerously-bypass-approvals-and-sandbox"]),
        ("gemini", m) => add(&["--approval-mode", m]),
        _ => {}
    }
    if !file.model.is_empty() && !models(&file.agent).is_empty() {
        add(&["--model", &file.model]);
    }
    if !file.effort.is_empty() && !efforts(&file.agent).is_empty() {
        add(&["--effort", &file.effort]);
    }
    out.extend(file.args.iter().cloned());
    out
}

/// Flags that set how much an agent may do without asking. They belong in
/// the permission mode field, which the list shows, never in the extra
/// arguments, where they would quietly change the mode the list shows.
///
/// Long options are compared without dashes, underscores or case, so
/// `--approval-mode` and `--approvalMode` both match; one-letter options
/// match also when their value is attached (`-sdanger-full-access`) or
/// when they sit in a group (`-dy`). Also listed are options that grant
/// rights another way: tool allow lists, settings files and config
/// profiles; options that start other programs or load code or settings
/// (MCP servers, plugins, extensions, agent definitions), that switch
/// the person's own safeguards off (hooks, sandbox), that widen where the
/// agent may write, and that hand the session to somewhere else.
///
/// This is a list of what is known to be risky, not of what is safe: it
/// cannot be complete, and agents gain options. So the approval view also
/// warns whenever a routine has extra arguments at all. Claude Code's
/// options were checked against `claude --help` (2026-11-11); those of
/// Codex and Gemini are the ones known from their documentation and were
/// not checked against an installed version.
fn permission_options(agent: &str) -> (&'static [&'static str], &'static [char]) {
    match agent {
        "claude" => (
            &[
                "permissionmode",
                "dangerouslyskippermissions",
                "allowdangerouslyskippermissions",
                "allowedtools",
                "settings",
                "settingsources",
                "permissionprompttool",
                "permissionprompts",
                // Start programs or load code and definitions.
                "mcpconfig",
                "plugindir",
                "pluginurl",
                "agents",
                "agent",
                // Skip the person's hooks and other safeguards.
                "bare",
                "safemode",
                // Write a file anywhere, or run the session elsewhere.
                "debugfile",
                "remotecontrol",
                "cloud",
                "environment",
                "teleport",
                "desktop",
                "bg",
                "background",
                // Let it drive the browser.
                "chrome",
            ],
            &[],
        ),
        "codex" => (
            &[
                "fullauto",
                "dangerouslybypassapprovalsandsandbox",
                "yolo",
                "sandbox",
                "askforapproval",
                "profile",
                // Widen what the sandbox lets it write.
                "adddir",
                "cd",
                // Switch features (safeguards among them) on or off.
                "enable",
                "disable",
            ],
            &['s', 'a', 'p', 'C'],
        ),
        "gemini" => (
            &[
                "approvalmode",
                "yolo",
                "allowedtools",
                "allowedmcpservernames",
                "extensions",
                "includedirectories",
                "sandbox",
                "nosandbox",
            ],
            &['y', 'e', 's'],
        ),
        _ => (&[], &[]),
    }
}

fn normalized(name: &str) -> String {
    name.trim_start_matches('-')
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// A Codex config setting (`-c key=value`) that changes the sandbox, the
/// approvals or the profile, starts other programs (MCP servers, the
/// `notify` program, hooks) or sends the session to another provider.
fn codex_permission_setting(value: &str) -> bool {
    let key = value
        .trim_start_matches(['=', '"', '\''])
        .split('=')
        .next()
        .unwrap_or("")
        .to_lowercase();
    [
        "sandbox",
        "approval",
        "profile",
        "mcp_servers",
        "notify",
        "hook",
        "model_provider",
    ]
    .iter()
    .any(|k| key.contains(k))
}

/// The first extra argument that changes the agent's permissions, if any.
fn permission_arg(agent: &str, args: &[String]) -> Option<String> {
    let (long, short) = permission_options(agent);
    for (i, arg) in args.iter().enumerate() {
        let next = args.get(i + 1).map(String::as_str).unwrap_or("");
        if let Some(body) = arg.strip_prefix("--") {
            let (name, value) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (body, None),
            };
            if long.contains(&normalized(name).as_str()) {
                return Some(arg.clone());
            }
            if agent == "codex"
                && normalized(name) == "config"
                && codex_permission_setting(value.unwrap_or(next))
            {
                return Some(format!("{arg} {next}").trim().to_string());
            }
        } else if let Some(body) = arg.strip_prefix('-') {
            let first = body.chars().next();
            // An attached value (`-sdanger-full-access`) or a group (`-dy`).
            let group = !body.is_empty() && body.chars().all(|c| c.is_ascii_alphabetic());
            if first.is_some_and(|c| short.contains(&c))
                || (group && body.chars().any(|c| short.contains(&c)))
            {
                return Some(arg.clone());
            }
            if agent == "codex" && first == Some('c') {
                let attached = &body[1..];
                let value = if attached.is_empty() { next } else { attached };
                if codex_permission_setting(value) {
                    return Some(format!("{arg} {next}").trim().to_string());
                }
            }
        }
    }
    None
}

/// What is wrong with the agent settings, if anything.
pub fn check(file: &RoutineFile) -> Option<String> {
    if let Some(arg) = permission_arg(&file.agent, &file.args) {
        return Some(format!(
            "the extra options change permissions ({arg}); use the permission mode instead"
        ));
    }
    if file.model.starts_with('-') {
        return Some("the model name starts with -".into());
    }
    let mode = &file.permission_mode;
    if !mode.is_empty() && find(modes(&file.agent), mode).is_none() {
        return Some(format!("{} has no permission mode \"{mode}\"", file.agent));
    }
    if !file.model.is_empty() && models(&file.agent).is_empty() {
        return Some(format!(
            "no model can be set for {} here; use extra options",
            file.agent
        ));
    }
    if file.model.chars().any(|c| c.is_whitespace()) {
        return Some("the model name has a space".into());
    }
    if !file.effort.is_empty() && find(efforts(&file.agent), &file.effort).is_none() {
        return Some(format!("{} has no effort \"{}\"", file.agent, file.effort));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(agent: &str, mode: &str, model: &str, effort: &str) -> RoutineFile {
        let mut f: RoutineFile = toml::from_str("name = \"x\"\nschedule = \"0 9 * * *\"").unwrap();
        f.agent = agent.into();
        f.permission_mode = mode.into();
        f.model = model.into();
        f.effort = effort.into();
        f
    }

    #[test]
    fn claude_flags() {
        let mut f = file("claude", "auto", "opus", "high");
        f.args = vec!["--verbose".into()];
        assert_eq!(
            flags(&f),
            vec![
                "--permission-mode",
                "auto",
                "--model",
                "opus",
                "--effort",
                "high",
                "--verbose"
            ]
        );
        assert!(flags(&file("claude", "", "", "")).is_empty());
        assert_eq!(check(&f), None);
    }

    #[test]
    fn other_agents() {
        assert_eq!(
            flags(&file("codex", "auto", "", "")),
            vec![
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "never"
            ]
        );
        assert_eq!(
            flags(&file("opencode", "", "anthropic/some-model", "")),
            vec!["--model", "anthropic/some-model"]
        );
        assert_eq!(
            flags(&file("codex", "read-only", "o-model", "")),
            vec!["--sandbox", "read-only", "--model", "o-model"]
        );
        assert_eq!(
            flags(&file("gemini", "yolo", "", "")),
            vec!["--approval-mode", "yolo"]
        );
        assert!(check(&file("amp", "", "", "")).is_none());
        assert!(check(&file("amp", "", "big", "")).is_some());
        assert!(check(&file("codex", "plan", "", "")).is_some());
        assert!(check(&file("claude", "", "", "huge")).is_some());
        assert!(check(&file("claude", "", "my model", "")).is_some());
    }

    #[test]
    fn model_lists_from_agents() {
        let text = "Available models\nauto - Auto\ncomposer-2 - Composer 2 (current)\n* gpt-x.1 \n\nNAME  PROVIDER\n";
        assert_eq!(parse_models(text), vec!["auto", "composer-2", "gpt-x.1"]);
        let lines = "anthropic/model-a\nopenai/model-b\nanthropic/model-a\n";
        assert_eq!(
            parse_models(lines),
            vec!["anthropic/model-a", "openai/model-b"]
        );
        let json = r#"{"models":[{"slug":"m-one","display":"One"},{"id":"m-two"}]}"#;
        assert_eq!(parse_models(json), vec!["m-one", "m-two"]);
        assert!(parse_models("error: not logged in. Run login first").is_empty());
        assert!(models("amp").is_empty());
        assert!(models("shell").is_empty());
        assert_eq!(models("cursor").len(), 1);
    }

    #[test]
    fn extra_arguments_cannot_change_permissions() {
        let with = |agent: &str, args: &[&str]| {
            let mut f = file(agent, "", "", "");
            f.args = args.iter().map(|s| s.to_string()).collect();
            check(&f)
        };
        assert!(with("claude", &["--permission-mode", "bypassPermissions"]).is_some());
        assert!(with("claude", &["--permission-mode=bypassPermissions"]).is_some());
        assert!(with("claude", &["--dangerously-skip-permissions"]).is_some());
        assert!(with("codex", &["--dangerously-bypass-approvals-and-sandbox"]).is_some());
        assert!(with("codex", &["-s", "danger-full-access"]).is_some());
        assert!(with("codex", &["-c", "sandbox_mode=\"danger-full-access\""]).is_some());
        assert!(with("codex", &["--config=approval_policy=never"]).is_some());
        assert!(with("gemini", &["--yolo"]).is_some());
        // Attached values, groups, other spellings and indirect grants.
        assert!(with("codex", &["-sdanger-full-access"]).is_some());
        assert!(with("codex", &["-anever"]).is_some());
        assert!(with("codex", &["-csandbox_mode=danger-full-access"]).is_some());
        assert!(with("codex", &["-c", "profiles.x.approval_policy=never"]).is_some());
        assert!(with("codex", &["--profile", "yolo"]).is_some());
        assert!(with("gemini", &["--approvalMode=yolo"]).is_some());
        assert!(with("gemini", &["-dy"]).is_some());
        assert!(with("gemini", &["--allowed-tools", "run_shell_command"]).is_some());
        assert!(with("claude", &["--allowedTools", "Bash"]).is_some());
        assert!(with("claude", &["--settings", "{}"]).is_some());
        // Options that start programs, load code or settings, switch
        // safeguards off or widen the sandbox.
        assert!(with("claude", &["--mcp-config", "servers.json"]).is_some());
        assert!(with("claude", &["--plugin-dir=/tmp/p"]).is_some());
        assert!(with("claude", &["--plugin-url", "https://example.com/p.zip"]).is_some());
        assert!(with("claude", &["--agents", "{}"]).is_some());
        assert!(with("claude", &["--setting-sources", "project"]).is_some());
        assert!(with("claude", &["--bare"]).is_some());
        assert!(with("claude", &["--debug-file", "/tmp/x"]).is_some());
        assert!(with("claude", &["--remote-control"]).is_some());
        assert!(with("claude", &["--chrome"]).is_some());
        assert!(with("claude", &["--cloud"]).is_some());
        assert!(with("codex", &["-c", "mcp_servers.x.command=sh"]).is_some());
        assert!(with("codex", &["-c", "notify=[\"sh\",\"-c\",\"id\"]"]).is_some());
        assert!(with("codex", &["--config", "model_provider=other"]).is_some());
        assert!(with("codex", &["--add-dir", "/"]).is_some());
        assert!(with("codex", &["-C", "/"]).is_some());
        assert!(with("gemini", &["--extensions", "x"]).is_some());
        assert!(with("gemini", &["-e", "x"]).is_some());
        assert!(with("gemini", &["--no-sandbox"]).is_some());
        // Other arguments stay free.
        assert!(with("claude", &["--verbose", "--add-dir", "../docs"]).is_none());
        assert!(with("codex", &["-c", "model_reasoning_effort=high"]).is_none());
        assert!(with("amp", &["--dangerously-skip-permissions"]).is_none());
        // A model is a name, not a flag.
        assert!(check(&file("codex", "", "--yolo", "")).is_some());
    }

    #[test]
    fn labels() {
        assert_eq!(label(modes("claude"), "auto"), "Auto");
        assert_eq!(label(models("claude"), "claude-x-1"), "claude-x-1");
        assert_eq!(label(models("claude"), ""), "Agent default");
    }
}
