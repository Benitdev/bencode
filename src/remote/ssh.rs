//! OpenSSH as BenCode drives it (MonoCode `remote_ssh.rs` `command`,
//! `validate_target`): the user's own client, config, keys and agent. A
//! connection kept open in the background never asks for anything
//! (`BatchMode`); Settings' Connect and Reconnect may, through the askpass
//! bridge (`askpass.rs`).

use std::path::Path;
use std::process::{Command, Stdio};

use super::Machine;

/// MonoCode `validate_target`: an SSH host or alias, maybe with a user,
/// that cannot be read as an option or a shell command.
pub fn validate_target(target: &str, port: Option<u16>) -> Result<String, String> {
    let target = target.trim();
    let plain = target
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-@:[]".contains(&b));
    if target.is_empty()
        || target.len() > 255
        || target.starts_with('-')
        || port == Some(0)
        || !plain
        || target.matches('@').count() > 1
        || target.starts_with('@')
        || target.ends_with('@')
    {
        return Err(
            "Enter an SSH hostname or alias, such as user@my-mac-mini, and a valid port.".into(),
        );
    }
    Ok(target.into())
}

/// MonoCode `shell_quote`: `value` as one word for a POSIX shell.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// `ssh` for `machine`, up to the destination: the caller adds `--`, the
/// target and the remote command (`remote_command`). `master` shares one
/// authenticated connection between a setup's steps, so a password is
/// asked for once.
pub fn command(machine: &Machine, interactive: bool, master: Option<&Path>) -> Command {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ForwardX11=no",
        "-o",
        "PermitLocalCommand=no",
        "-o",
        "StrictHostKeyChecking=ask",
        "-o",
        "NumberOfPasswordPrompts=3",
        "-o",
        if interactive {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        },
    ]);
    match master {
        Some(socket) => {
            command
                .args(["-o", "ControlMaster=auto", "-o", "ControlPersist=60", "-o"])
                .arg(format!("ControlPath={}", socket.display()));
        }
        None => {
            command.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
        }
    }
    if let Some(port) = machine.port {
        command.args(["-p", &port.to_string()]);
    }
    #[cfg(test)]
    command.args(
        TEST_OPTIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter(),
    );
    command
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Options the loopback test adds (its key, its known hosts).
#[cfg(test)]
pub static TEST_OPTIONS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// What runs on the host to reach the daemon. Through `sh` so the user's
/// own login shell (fish, nushell, …) does not have to read it.
pub fn connect_command() -> &'static str {
    "exec sh -c 'exec \"$HOME/.bencode-host/bin/bencode-host\" connect'"
}

/// `value` in double quotes for `sh`, where `$`, `` ` ``, `"` and `\` are
/// still special.
fn double_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        if matches!(c, '$' | '`' | '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// What a terminal tab runs for a folder on `machine`: an interactive SSH
/// session (it may ask for a password there) in that folder, in the
/// account's login shell.
pub fn terminal_command(machine: &Machine, host_path: &str) -> (String, Vec<String>) {
    let mut args = vec![
        "-t".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
    ];
    if let Some(port) = machine.port {
        args.extend(["-p".to_string(), port.to_string()]);
    }
    // Through `sh`, as `connect_command`: the login shell may be fish.
    let inner = format!(
        "cd {} 2>/dev/null; exec \"${{SHELL:-/bin/sh}}\" -l",
        double_quote(host_path)
    );
    let remote = format!("exec sh -c {}", shell_quote(&inner));
    args.extend(["--".to_string(), machine.target.clone(), remote]);
    ("ssh".to_string(), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_targets_cannot_inject_options_or_shell_commands() {
        for target in [
            "home",
            "me@mac-mini.local",
            "user@192.168.1.4",
            "user@[::1]",
        ] {
            assert!(validate_target(target, None).is_ok(), "{target}");
        }
        for target in [
            "",
            "-oProxyCommand=bad",
            "host;touch /tmp/x",
            "host\nname",
            "$(whoami)",
            "user@host command",
            "host/../../x",
            "ssh://user@host",
            "a@b@c",
        ] {
            assert!(validate_target(target, None).is_err(), "{target}");
        }
        assert!(validate_target("host", Some(0)).is_err());
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn a_terminal_opens_in_the_folder_with_the_login_shell() {
        let machine = Machine {
            id: "m".into(),
            name: "Mini".into(),
            target: "me@mini".into(),
            port: Some(2222),
        };
        let (program, args) = terminal_command(&machine, "/home/me/$it");
        assert_eq!(program, "ssh");
        assert_eq!(
            args[..5],
            ["-t", "-o", "ServerAliveInterval=15", "-p", "2222"]
        );
        assert_eq!(args[5..7], ["--", "me@mini"]);
        assert_eq!(
            args[7],
            r#"exec sh -c 'cd "/home/me/\$it" 2>/dev/null; exec "${SHELL:-/bin/sh}" -l'"#
        );
    }
}
