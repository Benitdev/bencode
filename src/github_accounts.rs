//! The GitHub accounts the `gh` CLI is signed in with, and the one a
//! project's `gh` commands run as. BenCode's own: MonoCode always runs
//! `gh` as its active account, so a project only another account can see
//! never reaches the Inbox.
//!
//! A project runs as the account picked for it in Settings › Integrations.
//! With none picked it runs as the active account, or, when that one
//! cannot see the repository, as the first signed-in account that can.
//!
//! BenCode keeps no token: it asks `gh auth token` for the account's and
//! hands it to the command in `GH_TOKEN`, never on an argv.
//!
//! Every call here runs `gh` or `git` and blocks; use the background
//! executor.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

const HOST: &str = "github.com";
/// How long "no account sees this repository" stands before it is asked
/// again (offline at launch, or a folder that is not on GitHub).
const RETRY_AFTER: Duration = Duration::from_secs(300);

/// An account `gh` is signed in with on github.com.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub login: String,
    /// The one `gh` uses when BenCode names none.
    pub active: bool,
}

#[derive(Default)]
struct State {
    /// Saved: the account picked for a project, by project folder.
    choices: BTreeMap<String, String>,
    /// A working copy's main checkout (a worktree's project), by folder.
    roots: HashMap<String, String>,
    /// What Automatic found for a main checkout: the account that sees
    /// its repository (`None` is the active one), and when to ask again.
    found: HashMap<String, (Option<String>, Option<Instant>)>,
    tokens: HashMap<String, String>,
}

fn state() -> Option<MutexGuard<'static, State>> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    match STATE.get_or_init(Default::default).lock() {
        Ok(state) => Some(state),
        Err(err) => {
            log::warn!("github accounts lock poisoned: {err}");
            None
        }
    }
}

/// The accounts picked in Settings, as saved. What Automatic found is
/// asked again, and so is each token (an account may have signed in anew).
pub fn set_choices(choices: BTreeMap<String, String>) {
    if let Some(mut state) = state() {
        state.choices = choices;
        state.found.clear();
        state.tokens.clear();
    }
}

/// `gh auth status --json hosts`, as far as BenCode reads it.
fn parse_accounts(json: &str) -> Result<Vec<Account>, String> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(default)]
        state: String,
        #[serde(default)]
        active: bool,
        login: String,
    }
    #[derive(Deserialize)]
    struct Status {
        #[serde(default)]
        hosts: HashMap<String, Vec<Row>>,
    }
    let status: Status = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let mut accounts: Vec<Account> = status
        .hosts
        .into_iter()
        .filter(|(host, _)| host == HOST)
        .flat_map(|(_, rows)| rows)
        // A sign-in whose token no longer works cannot run anything.
        .filter(|row| row.state == "success" && valid_login(&row.login))
        .map(|row| Account {
            login: row.login,
            active: row.active,
        })
        .collect();
    accounts.sort_by(|a, b| (!a.active, &a.login).cmp(&(!b.active, &b.login)));
    Ok(accounts)
}

/// The accounts `gh` is signed in with, the active one first.
pub fn list() -> Result<Vec<Account>, String> {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
    let json = crate::github::run_gh(
        Path::new(&home),
        &["auth", "status", "--json", "hosts"],
        None,
    )
    // gh exits non-zero when one account's token has expired, and
    // still prints them all.
    .or_else(|out| {
        if out.trim_start().starts_with('{') {
            Ok(out)
        } else {
            Err(out)
        }
    })?;
    parse_accounts(&json)
}

/// A GitHub login: what may follow `--user` on an argv.
fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 64
        && !login.starts_with('-')
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn token(login: &str) -> Option<String> {
    if !valid_login(login) {
        return None;
    }
    if let Some(token) = state()?.tokens.get(login).cloned() {
        return Some(token);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
    let args = ["auth", "token", "--hostname", HOST, "--user", login];
    match crate::github::run_gh(Path::new(&home), &args, None) {
        Ok(token) if !token.is_empty() => {
            state()?.tokens.insert(login.to_string(), token.clone());
            Some(token)
        }
        Ok(_) => None,
        Err(err) => {
            log::warn!("github: no token for account {login}: {err}");
            None
        }
    }
}

/// The main checkout of the repository `cwd` is in: a worktree answers
/// for its project. `cwd` itself outside a repository.
fn root_of(cwd: &str) -> String {
    if let Some(root) = state().and_then(|state| state.roots.get(cwd).cloned()) {
        return root;
    }
    let root = crate::git::git_command(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .and_then(|dir| dir.strip_suffix("/.git").map(str::to_string))
        .filter(|root| !root.is_empty())
        .unwrap_or_else(|| cwd.to_string());
    if let Some(mut state) = state() {
        state.roots.insert(cwd.to_string(), root.clone());
    }
    root
}

/// The account picked for the project `root` belongs to: the innermost
/// project folder that holds it.
fn choice_for(choices: &BTreeMap<String, String>, root: &str) -> Option<String> {
    choices
        .iter()
        .filter(|(project, _)| crate::app::is_path_in_project(root, project))
        .max_by_key(|(project, _)| project.len())
        .map(|(_, login)| login.clone())
}

/// Automatic: `None` when the active account sees the repository in
/// `cwd`, else the first other account that does.
fn find_account(cwd: &Path) -> (Option<String>, bool) {
    let sees = |token: Option<&str>| {
        crate::github::run_gh(cwd, &["repo", "view", "--json", "nameWithOwner"], token).is_ok()
    };
    if sees(None) {
        return (None, true);
    }
    let others = match list() {
        Ok(accounts) => accounts.into_iter().filter(|account| !account.active),
        Err(err) => {
            log::debug!("github: could not list accounts: {err}");
            return (None, false);
        }
    };
    for account in others {
        if token(&account.login).is_some_and(|token| sees(Some(&token))) {
            log::info!("github: {} is read as {}", cwd.display(), account.login);
            return (Some(account.login), true);
        }
    }
    (None, false)
}

/// The login `gh` runs as in `cwd`; `None` is its active account.
fn account_for(cwd: &Path) -> Option<String> {
    let root = root_of(&cwd.to_string_lossy());
    let (choice, found) = {
        let state = state()?;
        (
            choice_for(&state.choices, &root),
            state.found.get(&root).cloned(),
        )
    };
    if choice.is_some() {
        return choice;
    }
    match found {
        Some((login, retry)) if retry.is_none_or(|at| Instant::now() < at) => login,
        _ => {
            let (login, seen) = find_account(cwd);
            let retry = (!seen).then(|| Instant::now() + RETRY_AFTER);
            state()?.found.insert(root, (login.clone(), retry));
            login
        }
    }
}

/// The `GH_TOKEN` for a `gh` command in `cwd`; `None` leaves `gh` on its
/// active account.
pub(crate) fn token_for(cwd: &Path) -> Option<String> {
    token(&account_for(cwd)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_are_read_from_gh_with_the_active_one_first() {
        let json = r#"{"hosts":{
            "github.com":[
                {"state":"success","active":false,"host":"github.com","login":"work-me","tokenSource":"keyring"},
                {"state":"success","active":true,"host":"github.com","login":"me"},
                {"state":"error","active":false,"host":"github.com","login":"expired"}
            ],
            "ghe.example.com":[{"state":"success","active":true,"login":"elsewhere"}]
        }}"#;
        assert_eq!(
            parse_accounts(json).unwrap(),
            [
                Account {
                    login: "me".into(),
                    active: true
                },
                Account {
                    login: "work-me".into(),
                    active: false
                },
            ]
        );
        assert_eq!(parse_accounts(r#"{"hosts":{}}"#).unwrap(), []);
        assert!(parse_accounts("not json").is_err());
    }

    #[test]
    fn only_a_login_goes_on_an_argv() {
        assert!(valid_login("Kozocom-ThienPV"));
        assert!(valid_login("me_2"));
        for bad in ["", "-x", "a b", "a;b", "a/b", "--hostname"] {
            assert!(!valid_login(bad), "{bad}");
        }
    }

    #[test]
    fn a_project_choice_covers_what_is_inside_it() {
        let choices = BTreeMap::from([
            ("/work".to_string(), "work-me".to_string()),
            ("/work/side".to_string(), "me".to_string()),
        ]);
        assert_eq!(
            choice_for(&choices, "/work/api").as_deref(),
            Some("work-me")
        );
        assert_eq!(choice_for(&choices, "/work/side").as_deref(), Some("me"));
        assert_eq!(choice_for(&choices, "/workshop"), None);
        assert_eq!(choice_for(&choices, "/home/x"), None);
    }
}
