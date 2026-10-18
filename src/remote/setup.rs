//! Settings › Connections' Connect, Reconnect and Update Host (MonoCode
//! `remote.rs` `remote_ssh_begin` / `remote_ssh_reconnect` and
//! `remote_ssh.rs` `Job`): over SSH, check the machine, install the host
//! that matches this BenCode, and open the machine's connection. SSH's
//! questions (a new host's key, a password) show in Settings while the job
//! waits for the answer.
//!
//! One SSH login serves every step (`ControlMaster`), so a password is
//! asked for once; the machine's connection rides it too, and it ends a
//! minute after that connection does (`ControlPersist`). Reconnecting later
//! is a login of its own.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::askpass::{Askpass, random_hex};
use super::{Connection, Machine};

/// How long one setup step may take (a slow download included).
const STEP_TIMEOUT: Duration = Duration::from_secs(360);
/// How long a question waits for the user.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/Benitdev/bencode/releases/download";

/// MonoCode `Prompt`: what SSH asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub id: String,
    pub message: String,
    /// A yes / no question (trust a host); otherwise a secret to type.
    pub confirm: bool,
}

/// MonoCode `JobView`: what Settings shows while a setup runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobView {
    pub message: String,
    pub prompt: Option<Prompt>,
    pub done: bool,
    pub error: Option<String>,
    /// The machine, once connected.
    pub machine: Option<Machine>,
    /// A note for after it connected (lingering could not be turned on).
    pub note: Option<String>,
}

struct JobData {
    view: JobView,
    answer: Option<String>,
}

pub struct Job {
    inner: Mutex<JobData>,
    cancelled: AtomicBool,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

fn lock(job: &Job) -> MutexGuard<'_, JobData> {
    job.inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Job {
    fn new(on_change: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(JobData {
                view: JobView {
                    message: "Connecting to SSH and setting up BenCode Host…".into(),
                    ..JobView::default()
                },
                answer: None,
            }),
            cancelled: AtomicBool::new(false),
            on_change,
        })
    }

    pub fn view(&self) -> JobView {
        lock(self).view.clone()
    }

    fn message(&self, text: &str) {
        lock(self).view.message = text.into();
        (self.on_change)();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        if !lock(self).view.done {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// MonoCode `Job::answer`: the reply to the prompt now shown.
    pub fn answer(&self, id: &str, answer: String) -> Result<(), String> {
        let mut inner = lock(self);
        let prompt = inner
            .view
            .prompt
            .as_ref()
            .filter(|p| p.id == id)
            .ok_or("This SSH prompt has expired")?;
        if answer.len() > 8192
            || answer.contains(['\n', '\r', '\0'])
            || (prompt.confirm && answer != "yes" && answer != "no")
        {
            return Err("Invalid SSH prompt response".into());
        }
        inner.answer = Some(answer);
        Ok(())
    }

    /// Shows SSH's question and waits for the user's answer.
    fn prompt(&self, message: String, confirm: bool) -> Option<String> {
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        {
            let mut inner = lock(self);
            inner.answer = None;
            inner.view.prompt = Some(Prompt {
                id: random_hex(8).unwrap_or_default(),
                message,
                confirm,
            });
        }
        (self.on_change)();
        let answer = loop {
            let mut inner = lock(self);
            if self.is_cancelled() || Instant::now() >= deadline || inner.view.done {
                inner.view.prompt = None;
                inner.answer = None;
                break None;
            }
            if let Some(answer) = inner.answer.take() {
                inner.view.prompt = None;
                break Some(answer);
            }
            drop(inner);
            std::thread::sleep(Duration::from_millis(50));
        };
        (self.on_change)();
        answer
    }

    fn finish(&self, result: Result<(Machine, Option<String>), String>) {
        {
            let mut inner = lock(self);
            let result = if self.is_cancelled() {
                Err("Connection cancelled".to_string())
            } else {
                result
            };
            inner.view.done = true;
            inner.view.prompt = None;
            inner.answer = None;
            match result {
                Ok((machine, note)) => {
                    inner.view.message = "Connected".into();
                    inner.view.machine = Some(machine);
                    inner.view.note = note;
                }
                Err(error) => inner.view.error = Some(error),
            }
        }
        (self.on_change)();
    }
}

/// What a setup is for.
#[derive(Clone, Debug)]
pub enum Purpose {
    /// Add machine: `name` is empty for the host's own name.
    Add {
        target: String,
        port: Option<u16>,
        name: String,
    },
    /// Reconnect: sign in again (and install the host if it went missing).
    Reconnect(Machine),
    /// Update Host: install this version and restart the host.
    Update(Machine),
}

/// Starts a setup on a thread of its own; `adopt` gets the opened
/// connection (the app adds it to the registry and saves the machine).
pub fn begin(
    purpose: Purpose,
    on_change: Arc<dyn Fn() + Send + Sync>,
    adopt: impl FnOnce(Arc<Connection>) + Send + 'static,
) -> Arc<Job> {
    let job = Job::new(on_change);
    let worker = job.clone();
    std::thread::spawn(move || {
        let result = run(&worker, purpose);
        match result {
            Ok((conn, note)) => {
                let machine = conn.machine();
                adopt(conn);
                worker.finish(Ok((machine, note)));
            }
            Err(error) => worker.finish(Err(error)),
        }
    });
    job
}

fn run(job: &Arc<Job>, purpose: Purpose) -> Result<(Arc<Connection>, Option<String>), String> {
    let (mut machine, upgrade) = match purpose {
        Purpose::Add { target, port, name } => {
            let target = super::ssh::validate_target(&target, port)?;
            (
                Machine {
                    id: String::new(),
                    name: name
                        .trim()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(80)
                        .collect(),
                    target,
                    port,
                },
                false,
            )
        }
        Purpose::Reconnect(machine) => (machine, false),
        Purpose::Update(machine) => (machine, true),
    };
    let asker = job.clone();
    let askpass = Askpass::start(move |message, confirm| asker.prompt(message, confirm))?;
    let master = std::env::temp_dir().join(format!(
        "bencode-ssh-{}",
        random_hex(6).map_err(|e| e.to_string())?
    ));

    job.message("Checking the remote machine…");
    let probe = run_remote(
        job,
        &askpass,
        &machine,
        &master,
        "uname -s; uname -m; \"$HOME/.bencode-host/bin/bencode-host\" --version 2>/dev/null || true",
        Vec::new(),
    )?;
    let mut lines = probe.lines().map(str::trim);
    let os = lines.next().unwrap_or_default().to_ascii_lowercase();
    let arch = lines.next().unwrap_or_default().to_ascii_lowercase();
    let installed = lines.next().unwrap_or_default().to_string();
    if !matches!(os.as_str(), "linux" | "darwin") {
        return Err(format!(
            "BenCode Host runs on Linux and macOS; this machine runs {}.",
            if os.is_empty() { "something else" } else { &os }
        ));
    }

    if installed != VERSION || upgrade {
        if let Some(binary) = local_host_binary(&os, &arch) {
            job.message("Copying BenCode Host to the machine…");
            let bytes =
                std::fs::read(&binary).map_err(|err| format!("{}: {err}", binary.display()))?;
            let script = format!(
                "set -eu; umask 077; D=\"$HOME/.bencode-host/versions/{VERSION}\"; mkdir -p \"$D\"; cat > \"$D/bencode-host.part\"; chmod 700 \"$D/bencode-host.part\"; mv -f \"$D/bencode-host.part\" \"$D/bencode-host\""
            );
            run_remote(
                job,
                &askpass,
                &machine,
                &master,
                &format!("sh -c {}", super::ssh::shell_quote(&script)),
                bytes,
            )?;
        }
        job.message(if upgrade {
            "Updating BenCode Host…"
        } else {
            "Installing BenCode Host…"
        });
    } else {
        job.message("Starting BenCode Host…");
    }
    let script = bootstrap_script(upgrade);
    let output = run_remote(
        job,
        &askpass,
        &machine,
        &master,
        "sh -s",
        script.into_bytes(),
    )?;
    if !output.contains("BENCODE_HOST_READY") {
        return Err("BenCode Host did not finish installing. Try again.".into());
    }
    let note = output.contains("BENCODE_LINGER_FAILED").then(|| {
        "BenCode could not turn on lingering for your account, so agents may stop when you log out of the machine. Run `sudo loginctl enable-linger $USER` there to keep them running.".to_string()
    });

    job.message("Connecting to BenCode Host…");
    if job.is_cancelled() {
        return Err("Connection cancelled".into());
    }
    // Reconnect and Update open the registry's own connection again, so
    // the runs it follows carry on.
    let conn = match super::connection(&machine.id).filter(|_| !machine.id.is_empty()) {
        Some(conn) => {
            conn.drop_link();
            conn
        }
        None => super::new_connection(machine.clone()),
    };
    let info = conn.connect(Some(&askpass), Some(&master))?;
    if machine.id.is_empty() {
        machine.id = info.environment_id.clone();
        if machine.name.is_empty() {
            machine.name = info.name.clone();
        }
        conn.set_machine(machine.clone());
    }
    if !bencode_core::remote::path::valid_machine_id(&machine.id) {
        conn.close();
        return Err("The host answered with an id BenCode cannot use.".into());
    }
    Ok((conn, note))
}

fn bootstrap_script(upgrade: bool) -> String {
    include_str!("bootstrap.sh")
        .replace("\r\n", "\n")
        .replace("@@VERSION@@", &super::ssh::shell_quote(VERSION))
        .replace(
            "@@RELEASE@@",
            &super::ssh::shell_quote(&format!("{RELEASES}/v{VERSION}")),
        )
        .replace("@@UPGRADE@@", if upgrade { "1" } else { "0" })
}

/// A `bencode-host` of this version for the machine's platform found on
/// this computer: `BENCODE_HOST_BINARY`, or one built beside the app (a
/// development build, before the release that publishes it).
fn local_host_binary(os: &str, arch: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BENCODE_HOST_BINARY") {
        return Some(PathBuf::from(path));
    }
    let same_platform = os == std::env::consts::OS.replace("macos", "darwin")
        && matches!(
            (arch, std::env::consts::ARCH),
            ("arm64" | "aarch64", "aarch64") | ("x86_64" | "amd64", "x86_64")
        );
    if !same_platform {
        return None;
    }
    let beside = std::env::current_exe().ok()?.with_file_name("bencode-host");
    let version = Command::new(&beside).arg("--version").output().ok()?;
    (String::from_utf8_lossy(&version.stdout).trim() == VERSION).then_some(beside)
}

/// Runs `remote` on the machine with `input` on its stdin and returns what
/// it printed; a failure is what it said on stderr.
fn run_remote(
    job: &Arc<Job>,
    askpass: &Askpass,
    machine: &Machine,
    master: &std::path::Path,
    remote: &str,
    input: Vec<u8>,
) -> Result<String, String> {
    if job.is_cancelled() {
        return Err("Connection cancelled".into());
    }
    let mut command = super::ssh::command(machine, true, Some(master));
    askpass.configure(&mut command)?;
    command.arg("--").arg(&machine.target).arg(remote);
    let mut child = command
        .spawn()
        .map_err(|err| format!("Could not start OpenSSH: {err}"))?;
    let stdout = capture(child.stdout.take().ok_or("ssh has no stdout")?);
    let stderr = capture(child.stderr.take().ok_or("ssh has no stderr")?);
    let mut stdin = child.stdin.take().ok_or("ssh has no stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let deadline = Instant::now() + STEP_TIMEOUT;
    let status = loop {
        if job.is_cancelled() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err(if job.is_cancelled() {
                "Connection cancelled".to_string()
            } else {
                "SSH setup timed out. Check the machine's network connection and try again."
                    .to_string()
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(err.to_string());
            }
        }
    };
    let _ = writer.join();
    let output = String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned();
    let errors = String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned();
    if !status?.success() {
        let said: String = errors.trim().chars().take(4000).collect();
        return Err(if said.is_empty() {
            "SSH setup failed.".into()
        } else {
            format!("SSH setup failed: {said}")
        });
    }
    Ok(output)
}

fn capture(mut reader: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let room = (64 * 1024usize).saturating_sub(output.len());
            output.extend_from_slice(&buffer[..count.min(room)]);
        }
        output
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bootstrap_is_filled_in_and_checks_its_download() {
        let script = bootstrap_script(false);
        assert!(!script.contains("@@"));
        assert!(!script.contains('\r'));
        assert!(script.contains("--proto '=https'"));
        assert!(script.contains("does not match its checksum"));
        assert!(script.contains(&format!("VERSION='{VERSION}'")));
        assert!(script.contains("UPGRADE=0"));
        assert!(bootstrap_script(true).contains("UPGRADE=1"));
    }

    #[test]
    fn the_bootstrap_is_valid_sh() {
        let script = bootstrap_script(false);
        let check = std::process::Command::new("sh")
            .args(["-n", "-c", &script])
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    #[test]
    fn answers_must_match_the_current_prompt() {
        let job = Job::new(Arc::new(|| {}));
        assert!(job.answer("old", "yes".into()).is_err());
        let waiter = job.clone();
        let thread = std::thread::spawn(move || waiter.prompt("Trust this host?".into(), true));
        while job.view().prompt.is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let prompt = job.view().prompt.unwrap();
        assert!(job.answer(&prompt.id, "arbitrary".into()).is_err());
        job.answer(&prompt.id, "yes".into()).unwrap();
        assert_eq!(thread.join().unwrap(), Some("yes".into()));
        assert!(job.answer(&prompt.id, "yes".into()).is_err());
    }
}

/// The whole path over real OpenSSH: Add machine (the host built here
/// uploaded), the machine's git, the Explorer, and a turn of a fake Claude
/// Code. Needs a loopback sshd on port 2299 that sets `HOME` to
/// `$BENCODE_TEST_SSH_DIR/home` and trusts `client_key` there (see the
/// remote section of `docs/remote-access.md`), and `cargo build -p
/// bencode-host` first.
#[cfg(test)]
mod loopback {
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use bencode_core::harness::{AgentEvent, HarnessKind, PermissionPolicy, SpawnRequest};
    use bencode_core::remote::protocol::{Request, TreeOp};
    use bencode_core::tree::FsEntry;

    use super::*;

    const FAKE_CLAUDE: &str = r#"#!/bin/sh
read line
echo '{"type":"system","subtype":"init","session_id":"loop-session","model":"fake"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi from the machine"}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"hi","session_id":"loop-session"}'
"#;

    #[test]
    #[ignore = "needs the loopback sshd fixture"]
    fn a_machine_is_added_and_runs_a_turn_over_ssh() {
        let dir =
            PathBuf::from(std::env::var("BENCODE_TEST_SSH_DIR").expect("BENCODE_TEST_SSH_DIR"));
        let home = dir.join("home");
        *super::super::ssh::TEST_OPTIONS.lock().unwrap() = vec![
            "-F".into(),
            "/dev/null".into(),
            "-i".into(),
            dir.join("client_key").display().to_string(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-o".into(),
            format!("UserKnownHostsFile={}", dir.join("known_hosts").display()),
        ];
        let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/bencode-host");
        assert!(host.is_file(), "cargo build -p bencode-host first");
        // SAFETY: the one test that sets it; nothing else reads it at once.
        unsafe { std::env::set_var("BENCODE_HOST_BINARY", &host) };

        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let work = home.join("work");
        std::fs::create_dir_all(work.join("src")).unwrap();
        std::fs::write(work.join("src/a.rs"), "fn main() {}\n").unwrap();

        let job = begin(
            Purpose::Add {
                target: "127.0.0.1".into(),
                port: Some(2299),
                name: "Loopback".into(),
            },
            Arc::new(|| {}),
            crate::remote::adopt,
        );
        let started = Instant::now();
        while !job.view().done {
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "setup hung: {:?}",
                job.view()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let view = job.view();
        assert_eq!(view.error, None);
        let machine = view.machine.unwrap();
        assert_eq!(machine.name, "Loopback");
        assert!(home.join(".bencode-host/bin/bencode-host").exists());

        let root = crate::remote::remote_path(&machine.id, &work.to_string_lossy());
        let init = crate::remote::exec::Cmd::git(Some(root.as_ref()))
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        let status = crate::git::get_detailed_status(&root);
        assert!(
            status.unstaged.iter().any(|c| c.path == "src/a.rs"),
            "{status:?}"
        );

        let route = crate::remote::route(&root).unwrap().unwrap();
        let listed: Vec<FsEntry> = route
            .call(Request::Tree {
                root: route.host_path.clone(),
                op: TreeOp::List { rel: String::new() },
            })
            .unwrap();
        assert!(listed.iter().any(|e| e.name == "src" && e.is_dir));
        assert_eq!(
            crate::workspace::list_project_files(root.as_ref()),
            ["src/a.rs"]
        );

        let turn = crate::remote::spawn::spawn(&SpawnRequest {
            harness: HarnessKind::Claude,
            cwd: root.clone(),
            prompt: "hi".into(),
            model: None,
            permission: PermissionPolicy::AutoApprove,
            resume_id: None,
            disable_hooks: true,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            browser_mcp: None,
            app: None,
        })
        .unwrap();
        let mut events = turn.events;
        let mut seen = Vec::new();
        while let Some(event) = events.blocking_recv() {
            seen.push(event);
        }
        assert!(
            seen.iter()
                .any(|e| matches!(e, AgentEvent::TextDelta(t) if t.contains("machine"))),
            "{seen:?}"
        );
        assert!(seen.iter().any(|e| matches!(e, AgentEvent::Done(_))));

        let _: Result<(), String> = route.call(Request::Shutdown);
    }
}
