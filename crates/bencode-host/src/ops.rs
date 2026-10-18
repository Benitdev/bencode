//! What the host does for the app's views (MonoCode `host/workspace.ts`,
//! `workspace-commands.ts`, `attachments.ts`): run git and gh, read and
//! write files, list the CLIs and their models, keep attachments. The app
//! connects as this account over SSH, so it may do what the account may;
//! these only refuse what could never be meant (a relative path, a program
//! other than git or gh).

use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use bencode_core::harness::catalog::ModelOption;
use bencode_core::harness::{ALL_HARNESSES, HarnessKind, HarnessResolver, discovery};
use bencode_core::remote::protocol::{
    B64, EntryKind, EntryWire, ExecOutput, FileWire, HarnessWire, MAX_READ_BYTES, MAX_UPLOAD_BYTES,
    MetaWire, Program, ServerFrame, TreeOp, exec_env_allowed,
};

/// How long `Exec` may run when the app names no limit.
const EXEC_TIMEOUT: Duration = Duration::from_secs(120);
/// What `Exec` keeps of each stream.
const MAX_EXEC_OUTPUT: usize = 24 * 1024 * 1024;

/// `path`, which must be absolute and free of `..`.
pub fn absolute(path: &str) -> Result<String> {
    let parsed = Path::new(path);
    if !parsed.is_absolute() || parsed.components().any(|c| c == Component::ParentDir) {
        bail!("not an absolute path: {path}");
    }
    Ok(path.to_string())
}

/// `Request::Tree`: the Explorer's own functions, on this machine.
pub fn tree(id: u64, root: &str, op: TreeOp) -> Result<ServerFrame> {
    use bencode_core::tree;
    let root = PathBuf::from(absolute(root)?);
    let failed = anyhow::Error::msg;
    Ok(match op {
        TreeOp::List { rel } => ServerFrame::ok(id, tree::list_dir(&root, &rel).map_err(failed)?),
        TreeOp::Create {
            parent,
            name,
            is_dir,
        } => ServerFrame::ok(
            id,
            tree::create(&root, &parent, &name, is_dir).map_err(failed)?,
        ),
        TreeOp::Rename { rel, name } => {
            ServerFrame::ok(id, tree::rename(&root, &rel, &name).map_err(failed)?)
        }
        TreeOp::Delete { rel } => {
            tree::delete(&root, &rel).map_err(failed)?;
            ServerFrame::ok(id, ())
        }
        TreeOp::MoveInto { rel, dest_parent } => ServerFrame::ok(
            id,
            tree::move_into(&root, &rel, &dest_parent).map_err(failed)?,
        ),
        TreeOp::CopyInto { from, dest_parent } => {
            let from = PathBuf::from(absolute(&from)?);
            ServerFrame::ok(
                id,
                tree::copy_into(&root, &from, &dest_parent).map_err(failed)?,
            )
        }
    })
}

pub fn harnesses() -> Vec<HarnessWire> {
    let found = HarnessResolver::discover();
    ALL_HARNESSES
        .iter()
        .map(|&harness| {
            let info = found.iter().find(|info| info.id == harness.id());
            HarnessWire {
                harness,
                available: info.is_some_and(|info| info.available),
                binary_path: info
                    .and_then(|info| info.binary_path.as_ref())
                    .map(|path| path.display().to_string()),
            }
        })
        .collect()
}

pub fn models(harness: HarnessKind) -> Result<Vec<ModelOption>> {
    discovery::discover(harness)
}

pub fn exec(
    program: Program,
    args: &[String],
    cwd: Option<&str>,
    stdin: Option<B64>,
    env: &[(String, String)],
    timeout_ms: Option<u64>,
) -> Result<ExecOutput> {
    let mut command = Command::new(program.binary());
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(absolute(cwd)?);
    }
    for (name, value) in env {
        if exec_env_allowed(name) {
            command.env(name, value);
        }
    }
    command
        // Nothing on the host may stop to ask: there is no one at its terminal.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("could not start {} on the host", program.binary()))?;
    if let Some(B64(bytes)) = stdin {
        let mut pipe = child.stdin.take().context("no stdin")?;
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let out = child.stdout.take().context("no stdout")?;
    let err = child.stderr.take().context("no stderr")?;
    let stdout = std::thread::spawn(move || capture(out));
    let stderr = std::thread::spawn(move || capture(err));
    let limit = timeout_ms.map_or(EXEC_TIMEOUT, Duration::from_millis);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(ExecOutput {
        status,
        stdout: B64(stdout.join().unwrap_or_default()),
        stderr: B64(stderr.join().unwrap_or_default()),
    })
}

fn capture(mut reader: impl std::io::Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    while let Ok(n) = reader.read(&mut buffer) {
        if n == 0 {
            break;
        }
        let room = MAX_EXEC_OUTPUT.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..n.min(room)]);
    }
    kept
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |age| age.as_millis() as i64)
}

fn meta_of(path: &Path, meta: &std::fs::Metadata) -> MetaWire {
    let kind = if meta.file_type().is_symlink() {
        EntryKind::Symlink
    } else if meta.is_dir() {
        EntryKind::Dir
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    MetaWire {
        kind,
        target_is_dir: kind == EntryKind::Symlink && path.is_dir(),
        size: meta.len(),
        modified_ms: modified_ms(meta),
        readonly: meta.permissions().readonly(),
    }
}

pub fn stat(path: &str) -> Option<MetaWire> {
    let path = absolute(path).ok()?;
    let path = Path::new(&path);
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(meta_of(path, &meta))
}

pub fn read_dir(path: &str) -> Result<Vec<EntryWire>> {
    let dir = PathBuf::from(absolute(path)?);
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("cannot read {path}"))? {
        let entry = entry?;
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        entries.push(EntryWire {
            name: entry.file_name().to_string_lossy().into_owned(),
            meta: meta_of(&entry.path(), &meta),
        });
    }
    Ok(entries)
}

pub fn read_file(path: &str, max: u64) -> Result<FileWire> {
    let path = absolute(path)?;
    let mut file = std::fs::File::open(&path).with_context(|| format!("cannot open {path}"))?;
    let meta = file.metadata()?;
    if meta.is_dir() {
        bail!("{path} is a folder");
    }
    let max = max.min(MAX_READ_BYTES);
    let mut bytes = Vec::new();
    (&mut file).take(max).read_to_end(&mut bytes)?;
    Ok(FileWire {
        truncated: meta.len() > bytes.len() as u64,
        meta: meta_of(Path::new(&path), &meta),
        bytes: B64(bytes),
    })
}

/// Through a temporary file beside it, so a reader never sees half a file;
/// an existing file keeps its permissions.
pub fn write_file(path: &str, bytes: &[u8], create_new: bool) -> Result<()> {
    let path = PathBuf::from(absolute(path)?);
    let existing = std::fs::metadata(&path).ok();
    if create_new && existing.is_some() {
        bail!("{} already exists", path.display());
    }
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.bencode-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(meta) = &existing {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", path.display()))
}

pub fn create_dir(path: &str) -> Result<()> {
    let path = absolute(path)?;
    std::fs::create_dir(&path).with_context(|| format!("cannot create {path}"))
}

pub fn rename(from: &str, to: &str) -> Result<()> {
    let (from, to) = (absolute(from)?, absolute(to)?);
    if Path::new(&to).exists() {
        bail!("{to} already exists");
    }
    std::fs::rename(&from, &to).with_context(|| format!("cannot move {from} to {to}"))
}

pub fn remove(path: &str) -> Result<()> {
    let path = absolute(path)?;
    if Path::new(&path).parent().is_none() {
        bail!("refusing to remove /");
    }
    let meta = std::fs::symlink_metadata(&path).with_context(|| format!("cannot find {path}"))?;
    if meta.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    }
    .with_context(|| format!("cannot remove {path}"))
}

pub fn copy(from: &str, to: &str) -> Result<()> {
    let (from, to) = (PathBuf::from(absolute(from)?), PathBuf::from(absolute(to)?));
    if to.exists() {
        bail!("{} already exists", to.display());
    }
    if to.starts_with(&from) {
        bail!("cannot copy a folder into itself");
    }
    copy_tree(&from, &to)
        .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        std::os::unix::fs::symlink(target, to)
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Keeps an attachment under `dir/<random>/<name>` and answers that path.
pub fn upload(dir: &Path, name: &str, bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        bail!("attachments are limited to 20 MB a file");
    }
    let name: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\0' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = match name.trim_start_matches('.') {
        "" => "attachment".to_string(),
        _ => name,
    };
    let folder = dir.join(crate::dirs::random_id()?);
    std::fs::create_dir(&folder)?;
    let path = folder.join(&name);
    crate::dirs::write_private(&path, bytes)?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_absolute_plain_paths_pass() {
        assert!(absolute("/home/me/a").is_ok());
        assert!(absolute("home/me").is_err());
        assert!(absolute("/home/me/../../etc").is_err());
    }

    #[test]
    fn files_round_trip_through_the_ops() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let file = format!("{root}/a.txt");
        write_file(&file, b"hello", true).unwrap();
        assert!(write_file(&file, b"again", true).is_err());
        write_file(&file, b"hello world", false).unwrap();
        let read = read_file(&file, 5).unwrap();
        assert_eq!(read.bytes.0, b"hello");
        assert!(read.truncated);
        assert_eq!(stat(&file).unwrap().kind, EntryKind::File);
        create_dir(&format!("{root}/d")).unwrap();
        copy(&file, &format!("{root}/d/b.txt")).unwrap();
        copy(&format!("{root}/d"), &format!("{root}/e")).unwrap();
        assert!(copy(&format!("{root}/d"), &format!("{root}/d/inner")).is_err());
        rename(&format!("{root}/e"), &format!("{root}/f")).unwrap();
        let mut names: Vec<String> = read_dir(&root)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "d", "f"]);
        assert_eq!(
            read_file(&format!("{root}/f/b.txt"), 100).unwrap().bytes.0,
            b"hello world"
        );
        remove(&format!("{root}/f")).unwrap();
        assert!(stat(&format!("{root}/f")).is_none());
    }

    #[test]
    fn exec_runs_git_with_stdin_and_reports_status() {
        let out = exec(
            Program::Git,
            &["hash-object".into(), "--stdin".into()],
            None,
            Some(B64(b"x".to_vec())),
            &[("PATH".into(), "/nowhere".into())],
            None,
        )
        .unwrap();
        assert_eq!(out.status, Some(0));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout.0).trim(),
            "c1b0730e0133447badcfd47fd144e254807b06e1"
        );
        let failed = exec(
            Program::Git,
            &["no-such-command".into()],
            None,
            None,
            &[],
            None,
        )
        .unwrap();
        assert_ne!(failed.status, Some(0));
    }

    #[test]
    fn uploads_keep_a_safe_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = upload(dir.path(), "../a/b.png", b"png").unwrap();
        assert!(Path::new(&path).starts_with(dir.path()));
        assert!(path.ends_with("_a_b.png"), "{path}");
        assert!(upload(dir.path(), "big", &vec![0; MAX_UPLOAD_BYTES + 1]).is_err());
    }
}
