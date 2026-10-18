//! Where the host keeps its state: `~/.bencode-host` (MonoCode's is
//! `~/.monocode-host`), readable by its owner alone. `BENCODE_HOST_DIR`
//! moves it, for tests.

use std::io::{self, Read as _};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

pub struct Dirs {
    pub root: PathBuf,
}

impl Dirs {
    pub fn locate() -> io::Result<Self> {
        let root = match std::env::var_os("BENCODE_HOST_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => {
                let home =
                    std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;
                PathBuf::from(home).join(".bencode-host")
            }
        };
        let dirs = Self { root };
        for dir in [dirs.root.clone(), dirs.runs(), dirs.attachments()] {
            private_dir(&dir)?;
        }
        Ok(dirs)
    }

    pub fn socket(&self) -> PathBuf {
        self.root.join("daemon.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join("daemon.lock")
    }

    pub fn log(&self) -> PathBuf {
        self.root.join("host.log")
    }

    pub fn runs(&self) -> PathBuf {
        self.root.join("runs")
    }

    pub fn attachments(&self) -> PathBuf {
        self.root.join("attachments")
    }

    /// This host's id, made on first use: the machine's part of the app's
    /// `remote://` paths, so it must not change.
    pub fn environment_id(&self) -> io::Result<String> {
        let path = self.root.join("environment-id");
        match std::fs::read_to_string(&path) {
            Ok(id) if bencode_core::remote::path::valid_machine_id(id.trim()) => {
                return Ok(id.trim().to_string());
            }
            Ok(_) => log::warn!("{} is not an id; making a new one", path.display()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let id = random_id()?;
        write_private(&path, id.as_bytes())?;
        Ok(id)
    }
}

/// `xxxxxxxx-xxxx-xxxx`, from the system's random source.
pub fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..]))
}

fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Writes `bytes` to `path` through a temporary file, mode 0600.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
