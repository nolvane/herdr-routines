//! The accounts of the Multiaccount plugin (`herdr-multiaccount`), when it
//! is installed: a Claude Code routine can name the account it runs with.
//!
//! Only the list of accounts is read (id, name and kind; no login is in
//! that file). Picking the account is Multiaccount's own doing: the run's
//! tab gets `HERDR_MULTIACCOUNT_USE=<id>`, which its `claude` function
//! reads when Herdr starts the agent there.

use crate::paths::{self, Paths};
use serde::Deserialize;

/// The variable Multiaccount reads for "this session on this account".
pub const ENV_USE: &str = "HERDR_MULTIACCOUNT_USE";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    /// `main`, `subscription` or `api-key`.
    #[serde(default)]
    pub kind: String,
}

impl Account {
    /// What kind of login it is, in words.
    pub fn kind_text(&self) -> &'static str {
        match self.kind.as_str() {
            "main" => "The main login of Claude Code",
            "subscription" => "A subscription",
            "api-key" => "An API key",
            _ => "An account",
        }
    }
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    accounts: Vec<Account>,
}

/// An account id as Multiaccount makes them: lower case letters, digits
/// and dashes. Anything else never goes into a tab's environment.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Multiaccount's accounts file: its config folder sits beside this
/// plugin's.
fn file(paths: &Paths) -> Option<std::path::PathBuf> {
    Some(
        paths
            .config
            .parent()?
            .join("multiaccount")
            .join("accounts.json"),
    )
}

/// The accounts, in Multiaccount's order. None when the plugin is not
/// installed (or has no accounts yet).
pub fn list(paths: &Paths) -> Option<Vec<Account>> {
    let bytes = paths::read_limited(&file(paths)?, paths::FILE_LIMIT).ok()?;
    let file: File = serde_json::from_slice(&bytes).ok()?;
    let accounts: Vec<Account> = file
        .accounts
        .into_iter()
        .filter(|a| valid_id(&a.id))
        .map(|a| Account {
            name: crate::routine::without_hidden(a.name.trim()),
            ..a
        })
        .collect();
    (!accounts.is_empty()).then_some(accounts)
}

/// The account's name for showing, or its id when Multiaccount no longer
/// has it.
pub fn name_of(paths: &Paths, id: &str) -> String {
    list(paths)
        .and_then(|all| all.into_iter().find(|a| a.id == id))
        .map(|a| a.name)
        .unwrap_or_else(|| format!("{id} (not in Multiaccount any more)"))
}

/// The environment of a run's tab for a routine that names an account.
pub fn env_for(account: &str) -> Vec<String> {
    if valid_id(account) {
        vec![format!("{ENV_USE}={account}")]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_accounts_beside_its_own_config() {
        let dir = std::env::temp_dir().join(format!("hr-accounts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config").join("herdr-routines"),
            state: dir.join("state"),
            socket: dir.join("sock"),
        };
        // Not installed.
        assert_eq!(list(&paths), None);
        let theirs = dir.join("config").join("multiaccount");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(
            theirs.join("accounts.json"),
            r#"{"active":"main","accounts":[
                {"id":"main","name":"Main login","kind":"main","fallback":true},
                {"id":"team-two","name":"Team two","kind":"subscription","fallback":false},
                {"id":"Bad Id; rm","name":"x","kind":"api-key"}],"spaces":[]}"#,
        )
        .unwrap();
        let all = list(&paths).unwrap();
        assert_eq!(
            all.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["main", "team-two"]
        );
        assert_eq!(all[1].kind_text(), "A subscription");
        assert_eq!(name_of(&paths, "team-two"), "Team two");
        assert_eq!(
            name_of(&paths, "gone"),
            "gone (not in Multiaccount any more)"
        );
        assert_eq!(env_for("team-two"), ["HERDR_MULTIACCOUNT_USE=team-two"]);
        assert!(env_for("").is_empty());
        assert!(env_for("x y").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
