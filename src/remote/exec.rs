//! git and gh, here or on a machine (MonoCode routes its git commands
//! through the host's `workspace.run` when their path is remote). `Cmd`
//! is the part of `std::process::Command` the git module uses; a `cwd`
//! under `remote://` runs the command on that machine (`Request::Exec`),
//! with every argument naming a path there turned into the host's path.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write as _};
use std::os::unix::process::ExitStatusExt as _;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use bencode_core::remote::protocol::{B64, ExecOutput, Program, Request};

/// How long a remote command may run unless the caller says.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(120);

pub struct Cmd {
    program: Program,
    /// git's `-C` folder; for gh, its working folder.
    cwd: Option<OsString>,
    /// Options git takes before `-C` (`-c core.quotepath=false`, …).
    leading: Vec<OsString>,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
}

impl Cmd {
    /// `git` with BenCode's options (`bencode_core::git_cli`).
    pub fn git(cwd: Option<&OsStr>) -> Self {
        Self {
            program: Program::Git,
            cwd: cwd.map(OsStr::to_os_string),
            leading: ["-c", "core.quotepath=false", "-c", "color.ui=never"]
                .map(OsString::from)
                .to_vec(),
            args: Vec::new(),
            // Status polling from the UI must not fight other git processes for index.lock.
            env: vec![("GIT_OPTIONAL_LOCKS".into(), "0".into())],
            stdin: None,
        }
    }

    /// `gh`, run in `cwd`. The caller sets its token for a local run.
    pub fn gh(cwd: &OsStr) -> Self {
        Self {
            program: Program::Gh,
            cwd: Some(cwd.to_os_string()),
            leading: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
            stdin: None,
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    /// Runs it and collects what it printed, as `Command::output` does.
    pub fn output(self) -> io::Result<Output> {
        match self.cwd.as_deref().and_then(super::route) {
            Some(route) => self.remote_output(route.map_err(io::Error::other)?),
            None => self.local_output(),
        }
    }

    fn local_output(self) -> io::Result<Output> {
        let mut command = match self.program {
            Program::Git => Command::new(bencode_core::git_cli::program()),
            Program::Gh => Command::new(self.program.binary()),
        };
        command.args(&self.leading);
        match (self.program, &self.cwd) {
            (Program::Git, Some(cwd)) => {
                command.arg("-C").arg(cwd);
            }
            (Program::Gh, Some(cwd)) => {
                command.current_dir(cwd);
            }
            _ => {}
        }
        command.args(&self.args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let Some(input) = self.stdin else {
            return command.output();
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        // A large answer can fill stdout while the command still reads stdin.
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let output = child.wait_with_output()?;
        match writer.join() {
            Ok(Ok(())) => {}
            // The command stopped reading early; what it printed still counts.
            Ok(Err(err)) if err.kind() == io::ErrorKind::BrokenPipe => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err(io::Error::other("stdin writer panicked")),
        }
        Ok(output)
    }

    fn remote_output(self, route: super::Route) -> io::Result<Output> {
        let text = |value: &OsStr| -> io::Result<String> {
            value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| io::Error::other("a path that is not UTF-8 cannot go to a machine"))
        };
        let mut args = Vec::with_capacity(self.args.len() + self.leading.len() + 2);
        for arg in &self.leading {
            args.push(text(arg)?);
        }
        let cwd = Some(route.host_path.clone());
        if self.program == Program::Git {
            args.push("-C".into());
            args.push(route.host_path.clone());
        }
        for arg in &self.args {
            let arg = text(arg)?;
            // A path under this machine's `remote://` is its host path there.
            args.push(
                match bencode_core::remote::path::host_path_on(&arg, &route.machine) {
                    Some(host) => host,
                    None => arg,
                },
            );
        }
        let env = self
            .env
            .iter()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v.to_str()?.to_string())))
            .collect();
        let timeout = REMOTE_TIMEOUT;
        let out: ExecOutput = route
            .call_for(
                Request::Exec {
                    program: self.program,
                    args,
                    cwd: (self.program == Program::Gh).then_some(cwd).flatten(),
                    stdin: self.stdin.map(B64),
                    env,
                    timeout_ms: Some(timeout.as_millis() as u64),
                },
                timeout + Duration::from_secs(15),
            )
            .map_err(io::Error::other)?;
        Ok(Output {
            // A command the host had to kill reads as killed by a signal.
            status: match out.status {
                Some(code) => std::process::ExitStatus::from_raw((code & 0xff) << 8),
                None => std::process::ExitStatus::from_raw(9),
            },
            stdout: out.stdout.0,
            stderr: out.stderr.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_git_runs_with_stdin_and_reports_its_status() {
        let mut cmd = Cmd::git(None).args(["hash-object", "--stdin"]);
        cmd.stdin = Some(b"x".to_vec());
        let out = cmd.output().unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "c1b0730e0133447badcfd47fd144e254807b06e1"
        );
        let failed = Cmd::git(None).args(["no-such-command"]).output().unwrap();
        assert!(!failed.status.success());
    }

    #[test]
    fn exit_codes_survive_the_round_trip() {
        let status = std::process::ExitStatus::from_raw((1 & 0xff) << 8);
        assert_eq!(status.code(), Some(1));
        assert!(std::process::ExitStatus::from_raw(9).code().is_none());
    }

    #[test]
    fn an_unknown_machine_is_an_error() {
        let err = Cmd::git(Some(OsStr::new("remote://nowhere-1/a")))
            .args(["status"])
            .output()
            .unwrap_err();
        assert!(err.to_string().contains("not connected"), "{err}");
    }
}
