//! MonoCode `ssh_askpass.rs`: OpenSSH asks for a host's trust or a password
//! by running `SSH_ASKPASS`, which is this binary in a small, non-GUI mode.
//! It hands the question to the app over a Unix socket only the app's
//! account can open, with a secret the app made, and prints the answer for
//! ssh. Answers never touch argv or a file.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const SOCKET_ENV: &str = "BENCODE_SSH_ASKPASS_SOCKET";
const SECRET_ENV: &str = "BENCODE_SSH_ASKPASS_SECRET";
/// The most a question or an answer may hold.
const MAX_LINE: u64 = 16 * 1024;

#[derive(Serialize, Deserialize)]
struct Question {
    secret: String,
    prompt: String,
    confirm: bool,
}

/// MonoCode `confirmation_prompt`: a yes / no question (a host to trust),
/// not a secret to type.
fn is_confirmation(prompt: &str, hint: &str) -> bool {
    hint == "confirm" || prompt.contains("(yes/no/[fingerprint])") || prompt.contains("(yes/no)")
}

/// Answers ssh's question when `main` was started as its askpass; returns
/// the exit code. `None`: this is the app.
pub fn run_from_env() -> Option<i32> {
    let socket = std::env::var_os(SOCKET_ENV)?;
    let ask = || -> Option<i32> {
        let mut stream = UnixStream::connect(PathBuf::from(&socket)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(130))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
        let prompt = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "SSH authentication".into());
        let confirm = is_confirmation(
            &prompt,
            &std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default(),
        );
        let question = Question {
            secret: std::env::var(SECRET_ENV).ok()?,
            prompt,
            confirm,
        };
        writeln!(stream, "{}", serde_json::to_string(&question).ok()?).ok()?;
        let answer: Option<String> = serde_json::from_str(&read_line(&mut stream)?).ok()?;
        let answer = answer?;
        println!("{answer}");
        Some(if confirm && answer != "yes" { 1 } else { 0 })
    };
    Some(ask().unwrap_or(1))
}

fn read_line(stream: &mut UnixStream) -> Option<String> {
    let mut line = String::new();
    BufReader::new(stream).take(MAX_LINE).read_line(&mut line).ok()?;
    line.ends_with('\n').then_some(line)
}

/// The app's end: questions from the ssh processes `configure` set up go to
/// `handler`, whose answer (`None`: refuse) goes back. Stops when dropped.
pub struct Askpass {
    socket: PathBuf,
    secret: String,
    stopped: Arc<AtomicBool>,
}

impl Askpass {
    pub fn start(
        handler: impl Fn(String, bool) -> Option<String> + Send + 'static,
    ) -> Result<Self, String> {
        let secret = random_hex(32).map_err(|err| err.to_string())?;
        let socket = std::env::temp_dir().join(format!("bencode-askpass-{}.sock", &secret[..12]));
        let listener = UnixListener::bind(&socket).map_err(|err| err.to_string())?;
        listener.set_nonblocking(true).map_err(|err| err.to_string())?;
        let stopped = Arc::new(AtomicBool::new(false));
        let shutdown = stopped.clone();
        let expected = secret.clone();
        std::thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                        let question = read_line(&mut stream)
                            .and_then(|line| serde_json::from_str::<Question>(&line).ok())
                            .filter(|q| q.secret == expected);
                        if let Some(question) = question {
                            let answer = handler(question.prompt, question.confirm);
                            let line = serde_json::to_string(&answer).unwrap_or_else(|_| "null".into());
                            let _ = writeln!(stream, "{line}");
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            socket,
            secret,
            stopped,
        })
    }

    /// Points an ssh launch at this bridge.
    pub fn configure(&self, command: &mut Command) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        command
            .env("SSH_ASKPASS", exe)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "bencode:0")
            .env(SOCKET_ENV, &self.socket)
            .env(SECRET_ENV, &self.secret);
        Ok(())
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// `bytes` random bytes as hex, from the system's random source.
pub fn random_hex(bytes: usize) -> std::io::Result<String> {
    let mut raw = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut raw)?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_trust_is_a_confirmation_and_a_password_is_not() {
        assert!(is_confirmation(
            "Are you sure you want to continue connecting (yes/no/[fingerprint])?",
            ""
        ));
        assert!(is_confirmation("Allow access?", "confirm"));
        assert!(!is_confirmation("Enter passphrase for key:", ""));
        assert!(!is_confirmation("user@host's password:", ""));
    }

    #[test]
    fn the_bridge_answers_only_with_the_secret() {
        let bridge = Askpass::start(|prompt, confirm| {
            assert_eq!(prompt, "Passphrase?");
            assert!(!confirm);
            Some("test secret".into())
        })
        .unwrap();
        let ask = |secret: &str| {
            let mut stream = UnixStream::connect(&bridge.socket).unwrap();
            let question = Question {
                secret: secret.into(),
                prompt: "Passphrase?".into(),
                confirm: false,
            };
            writeln!(stream, "{}", serde_json::to_string(&question).unwrap()).unwrap();
            read_line(&mut stream)
        };
        assert_eq!(ask("wrong"), None);
        let answer = ask(&bridge.secret.clone()).unwrap();
        assert_eq!(serde_json::from_str::<String>(&answer).unwrap(), "test secret");
    }
}
