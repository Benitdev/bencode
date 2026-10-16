# Remote access

BenCode can run its agents on another Mac or Linux machine. The machine runs
**BenCode Host** (`crates/bencode-host`), which starts the agent CLIs there and
keeps each turn's events on its disk. A turn goes on when SSH drops, the
laptop sleeps or BenCode quits; the next launch follows it again. This is
MonoCode's remote access (`reference/monocode/docs/remote-access.md`), with a
Rust host instead of MonoCode's Node one.

A folder on a machine is a project in the rail, with the machine's name and a
globe whose dot is green while the machine is connected. Its threads, Explorer,
editor, Go to File, search, Changes, history, branches, worktrees, pull
requests and terminal tabs all work on the machine.

## Connect a machine

**Settings › Connections › Add machine**: an SSH address (`user@my-mac-mini`)
or an alias from your SSH config, an optional name and SSH port, **Connect**.

BenCode then, over one SSH login:

1. checks the machine (`uname`) and the host it has, if any;
2. installs this BenCode's version of the host under `~/.bencode-host`,
   downloaded from the matching GitHub release
   (`bencode-host-<os>-<arch>.tar.gz`) and checked against its `.sha256`;
3. on Linux, turns on lingering (`loginctl enable-linger`) so the host keeps
   running after you log out; Settings says so when it cannot;
4. opens the machine's connection: `ssh <machine> bencode-host connect`.

A new host key or a password is asked for in Settings (BenCode is OpenSSH's
askpass while it connects). Passwords are never saved. Later connections are
opened in the background and never ask: with keys or an agent they just work;
with a password, click **Reconnect**.

Prerequisites on the machine: SSH, `curl` or `wget`, `tar`, `shasum` or
`sha256sum`, and the agent CLIs you want, installed and signed in under the
account you connect as. The host finds them on the login shell's PATH and in
their usual install folders.

**Update Host** shows when the machine's host is another version than the
app. It installs this version and restarts the host, which interrupts the
turns running there.

**Remove** forgets the machine on this computer and closes its connection. The
host and its agents keep running; threads in its projects stay in BenCode, and
adding the machine again brings them back. To stop the host, run
`~/.bencode-host/bin/bencode-host stop` there (`status` says whether it runs).

## Open a folder

Rail **+** › **Open folder on a machine…**: pick the machine, browse to a
folder or type its path, **Open**.

Not available in these projects: Reveal in Finder, Open in editor, the session
review card (no checkpoints are taken on a machine), the in-app browser's tools
and provider accounts (a machine's CLIs use that machine's own sign-in), and
attaching folders (files are copied to the machine, up to 20 MB each).
Terminal tabs are SSH sessions in the folder; they do not outlive BenCode as
local ones do.

## How it works

- **Paths.** A folder on a machine is `remote://<machine id>/<path>`
  (`bencode_core::remote::path`). The machine id is the host's
  `environment-id`, so re-adding a machine keeps its projects.
- **Routing.** Everything that takes a path sends a `remote://` one to its
  machine: git and gh (`src/remote/exec.rs`, behind `git::git_command` and
  `github::gh`), the Explorer (`bencode_core::tree`, run by the host whole),
  the editor (`src/remote/files.rs`), Go to File and search (`bencode_core::files`
  and `search`, run by the host), and the agent's turn (`src/remote/spawn.rs`).
- **Protocol.** JSON lines over the SSH connection's stdio
  (`bencode_core::remote::protocol`). SSH authenticates, so there are no device
  tokens, ports or forwards.
- **Turns.** `remote::spawn` gives `app/agent.rs` the same handle and event
  stream `harness::spawn` does, so the transcript is folded here, in BenCode's
  database. The host numbers each run's events and keeps them on disk
  (`~/.bencode-host/runs`); a connection that comes back asks for the run
  again from the last event it had. A turn that is running when BenCode quits
  is recorded in `remote-runs.json`; at launch the thread goes back to where
  the turn began and the host replays it whole.
- **Threads** of a machine's projects live in this computer's database. Another
  computer connected to the same machine does not see them (MonoCode's host
  keeps them; BenCode's keeps only the turns).

## Development

The release workflow builds the host packages (`packaging/host/package.sh`);
an unreleased version has none to download. When the machine is the same
platform as this computer, `cargo build -p bencode-host` puts one beside the
app and Connect uploads it instead; `BENCODE_HOST_BINARY=/path/to/bencode-host`
uploads any other build (for Linux, `cargo zigbuild -p bencode-host --target
x86_64-unknown-linux-musl`).

Tests:

- `cargo test -p bencode-host` runs the daemon end to end with a fake Claude
  Code (`crates/bencode-host/tests/daemon.rs`).
- `remote::setup::loopback` drives the whole path over real OpenSSH. It is
  ignored; to run it, start a throwaway sshd on 127.0.0.1:2299 that sets the
  session's `HOME` (nothing is written to yours):

  ```bash
  T=/tmp/bencode-sshtest && rm -rf $T && mkdir -p $T/home && cd $T
  ssh-keygen -q -t ed25519 -N '' -f host_key
  ssh-keygen -q -t ed25519 -N '' -f client_key
  cp client_key.pub authorized_keys
  cat > sshd_config <<EOF
  Port 2299
  ListenAddress 127.0.0.1
  HostKey $T/host_key
  AuthorizedKeysFile $T/authorized_keys
  PasswordAuthentication no
  KbdInteractiveAuthentication no
  UsePAM no
  StrictModes no
  PidFile $T/sshd.pid
  SetEnv HOME=$T/home BENCODE_HOST_KEEP_PATH=1
  EOF
  /usr/sbin/sshd -f $T/sshd_config  # absolute: sshd re-runs itself from /
  echo "[127.0.0.1]:2299 $(cat host_key.pub)" > known_hosts
  cd - && cargo build -p bencode-host
  BENCODE_TEST_SSH_DIR=$T cargo test a_machine_is_added -- --ignored
  kill "$(cat $T/sshd.pid)"
  ```
