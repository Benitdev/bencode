//! Branch listing and switching for the composer's branch picker (MonoCode
//! `BranchPicker.tsx`: checkout, create-and-checkout, and the "Uncommitted
//! changes" stash path when a switch is blocked).

use anyhow::{Result, bail};

use super::{run_git, run_git_string};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Local name, or `remote/name` for a remote-tracking branch.
    pub name: String,
    pub remote: bool,
    pub current: bool,
}

/// Why a switch did not happen.
#[derive(Debug)]
pub enum SwitchError {
    /// Local changes would be overwritten; stashing first would let it through.
    BlockedByChanges,
    Failed(anyhow::Error),
}

fn ref_names(cwd: &str, args: &[&str]) -> Vec<String> {
    run_git_string(cwd, args)
        .map(|out| {
            out.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Local branches, then remote ones that have no local branch of the same
/// name (MonoCode lists both).
pub fn list_branches(cwd: &str) -> Vec<Branch> {
    let current = run_git_string(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let local = ref_names(cwd, &["branch", "--format=%(refname:short)"]);
    let remote = ref_names(cwd, &["branch", "-r", "--format=%(refname:short)"]);
    let mut branches: Vec<Branch> = local
        .iter()
        .map(|name| Branch {
            current: *name == current,
            name: name.clone(),
            remote: false,
        })
        .collect();
    let untracked_remote = |name: &String| {
        !name.ends_with("/HEAD")
            && local_name(name).is_some_and(|short| !local.iter().any(|l| l == short))
    };
    branches.extend(
        remote
            .iter()
            .filter(|name| untracked_remote(name))
            .map(|name| Branch {
                name: name.clone(),
                remote: true,
                current: false,
            }),
    );
    branches
}

/// `origin/feature` → `feature`.
fn local_name(remote: &str) -> Option<&str> {
    remote.split_once('/').map(|(_, name)| name)
}

/// Rejects names git would read as options or refuse as branch names.
fn validate_branch_name(cwd: &str, name: &str) -> Result<()> {
    let refused = name.is_empty()
        || name.starts_with('-')
        || run_git(cwd, &["check-ref-format", "--branch", name]).is_err();
    if refused {
        bail!("not a valid branch name: {name}");
    }
    Ok(())
}

fn classify(err: anyhow::Error) -> SwitchError {
    let text = format!("{err:#}");
    let blocked =
        text.contains("would be overwritten") || text.contains("commit your changes or stash them");
    if blocked {
        SwitchError::BlockedByChanges
    } else {
        SwitchError::Failed(err)
    }
}

/// Switches to `branch`; a remote branch gets a local tracking branch.
/// Returns the local branch name now checked out.
pub fn switch_branch(cwd: &str, branch: &Branch) -> Result<String, SwitchError> {
    validate_branch_name(cwd, &branch.name).map_err(SwitchError::Failed)?;
    if branch.remote {
        run_git(cwd, &["switch", "--track", &branch.name]).map_err(classify)?;
        let local = local_name(&branch.name).unwrap_or(&branch.name);
        return Ok(local.to_string());
    }
    run_git(cwd, &["switch", &branch.name]).map_err(classify)?;
    Ok(branch.name.clone())
}

/// Creates `name` from the current commit and switches to it.
pub fn create_branch(cwd: &str, name: &str) -> Result<String, SwitchError> {
    validate_branch_name(cwd, name).map_err(SwitchError::Failed)?;
    run_git(cwd, &["switch", "-c", name]).map_err(classify)?;
    Ok(name.to_string())
}

/// Stashes every local change, untracked files included.
pub fn stash_changes(cwd: &str, message: &str) -> Result<()> {
    run_git(
        cwd,
        &["stash", "push", "--include-untracked", "-m", message],
    )?;
    Ok(())
}
