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
        "Works in the folder without asking (--full-auto)",
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

/// Agents that take `--model` but whose model names change too often to
/// list: the name is typed.
const TYPED_MODELS: &[Choice] = &[DEFAULT];

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

/// Suggested models; empty when the agent has no model choice here. A model
/// not in the list can be typed for every agent that has one.
pub fn models(agent: &str) -> &'static [Choice] {
    match agent {
        "claude" => CLAUDE_MODELS,
        "codex" | "gemini" => TYPED_MODELS,
        _ => &[],
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
        ("codex", "auto") => add(&["--full-auto"]),
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

/// What is wrong with the agent settings, if anything.
pub fn check(file: &RoutineFile) -> Option<String> {
    let mode = &file.permission_mode;
    if !mode.is_empty() && find(modes(&file.agent), mode).is_none() {
        return Some(format!("{} has no permission mode \"{mode}\"", file.agent));
    }
    if !file.model.is_empty() && models(&file.agent).is_empty() {
        return Some(format!(
            "no model can be set for {} here; use extra arguments",
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
        assert_eq!(flags(&file("codex", "auto", "", "")), vec!["--full-auto"]);
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
    fn labels() {
        assert_eq!(label(modes("claude"), "auto"), "Auto");
        assert_eq!(label(models("claude"), "claude-x-1"), "claude-x-1");
        assert_eq!(label(models("claude"), ""), "Agent default");
    }
}
