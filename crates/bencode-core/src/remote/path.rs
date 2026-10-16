//! MonoCode `shared/lib/remotePaths.ts` and `remoteProjects.ts`
//! (`remotePath`, `parseRemotePath`): a path on a connected machine is
//! `remote://<machine>/<path on the host>` in the app, so every view that
//! takes a path can tell where it lives, and the routing below it (git,
//! files, the agent's turn) sends it to that machine.
//!
//! `<machine>` is the host's environment id (`HostInfo::environment_id`),
//! which stays the same when the machine is added again.

/// MonoCode `REMOTE_PATH_PREFIX`.
pub const REMOTE_PREFIX: &str = "remote://";

/// Whether `path` names something on a connected machine.
pub fn is_remote(path: &str) -> bool {
    path.starts_with(REMOTE_PREFIX)
}

/// MonoCode `remotePath`: how the app addresses `host_path` (absolute, on
/// a Unix host) on `machine`.
pub fn remote_path(machine: &str, host_path: &str) -> String {
    format!(
        "{REMOTE_PREFIX}{machine}/{}",
        host_path.trim_start_matches('/')
    )
}

/// A `remote://` path taken apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePath<'a> {
    pub machine: &'a str,
    /// The absolute path on the host, `/` for the host's root.
    pub host_path: String,
}

/// MonoCode `parseRemotePath`: the machine and host path behind a
/// `remote://` path; `None` for a local path or one with no machine.
pub fn parse(path: &str) -> Option<RemotePath<'_>> {
    let rest = path.strip_prefix(REMOTE_PREFIX)?;
    let (machine, host) = match rest.split_once('/') {
        Some((machine, host)) => (machine, host),
        None => (rest, ""),
    };
    if machine.is_empty() {
        return None;
    }
    Some(RemotePath {
        machine,
        host_path: format!("/{}", host.trim_end_matches('/')).replace("//", "/"),
    })
}

/// The host path of `path` when it is on `machine`.
pub fn host_path_on(path: &str, machine: &str) -> Option<String> {
    parse(path)
        .filter(|parsed| parsed.machine == machine)
        .map(|parsed| parsed.host_path)
}

/// Whether a machine id could be one: the host makes them (`[a-z0-9-]`),
/// and they land in paths and on argv.
pub fn valid_machine_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip() {
        let path = remote_path("m-1", "/home/me/app");
        assert_eq!(path, "remote://m-1/home/me/app");
        assert!(is_remote(&path));
        let parsed = parse(&path).unwrap();
        assert_eq!(parsed.machine, "m-1");
        assert_eq!(parsed.host_path, "/home/me/app");
        assert_eq!(host_path_on(&path, "m-1").as_deref(), Some("/home/me/app"));
        assert_eq!(host_path_on(&path, "other"), None);
    }

    #[test]
    fn the_root_and_trailing_slashes_parse() {
        assert_eq!(parse("remote://m").unwrap().host_path, "/");
        assert_eq!(parse("remote://m/").unwrap().host_path, "/");
        assert_eq!(parse("remote://m/a/b/").unwrap().host_path, "/a/b");
        assert_eq!(parse("remote:///a"), None);
        assert_eq!(parse("/home/me"), None);
    }

    #[test]
    fn machine_ids_are_plain() {
        assert!(valid_machine_id("3f2a-91bc"));
        assert!(!valid_machine_id(""));
        assert!(!valid_machine_id("a/b"));
        assert!(!valid_machine_id("-oproxycommand"));
        assert!(!valid_machine_id("UPPER"));
    }
}
