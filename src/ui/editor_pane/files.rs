//! Pure file IO for the editor: guarded reads and atomic, permission-preserving saves.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Largest file the native editor opens.
pub const MAX_EDITOR_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// How much of a file is scanned for NUL bytes to spot binaries.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Why a file was not opened in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    NotAFile,
    TooLarge { bytes: u64, limit: u64 },
    Binary,
    NotUtf8,
    Io(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAFile => write!(f, "Not a regular file."),
            Self::TooLarge { bytes, limit } => write!(
                f,
                "File is {} MB; the editor opens files up to {} MB.",
                bytes.div_ceil(1024 * 1024),
                limit / (1024 * 1024)
            ),
            Self::Binary => write!(f, "Binary files cannot be edited."),
            Self::NotUtf8 => write!(f, "File is not valid UTF-8 text."),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

/// Detects the syntax highlighter language from a file extension.
pub fn detect_language(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "Rust",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" => "JavaScript",
        "py" => "Python",
        "json" => "JSON",
        "md" | "markdown" => "Markdown",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "html" | "htm" => "HTML",
        "css" | "scss" => "CSS",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        _ => "Plain text",
    }
}

/// Resolves a workspace-relative or absolute path.
pub fn resolve_path(cwd: &str, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        Path::new(cwd).join(candidate)
    }
}

/// The last path component, for tab titles and messages.
pub fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Lines as the editor counts them: an empty buffer holds one.
pub fn count_lines(text: &str) -> usize {
    text.bytes().filter(|b| *b == b'\n').count() + 1
}

/// Reads a text file, refusing directories, oversized files, binaries and non-UTF-8.
pub fn read_text_file(path: &Path, limit: u64) -> Result<String, ReadError> {
    let meta = fs::metadata(path).map_err(|e| ReadError::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(ReadError::NotAFile);
    }
    check_size(meta.len(), limit)?;
    let bytes = fs::read(path).map_err(|e| ReadError::Io(e.to_string()))?;
    check_size(bytes.len() as u64, limit)?;
    decode_text(bytes)
}

fn check_size(bytes: u64, limit: u64) -> Result<(), ReadError> {
    if bytes > limit {
        return Err(ReadError::TooLarge { bytes, limit });
    }
    Ok(())
}

/// Decodes bytes as editable text.
pub fn decode_text(bytes: Vec<u8>) -> Result<String, ReadError> {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return Err(ReadError::Binary);
    }
    String::from_utf8(bytes).map_err(|_| ReadError::NotUtf8)
}

/// Writes through a sibling temp file and a rename, keeping the original's permissions.
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = dir.join(format!(
        ".{}.bencode-save-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = fs::metadata(&target).ok().map(|m| m.permissions());
    let result = write_temp(&temp, contents, permissions).and_then(|()| fs::rename(&temp, &target));
    if result.is_err()
        && let Err(err) = fs::remove_file(&temp)
        && err.kind() != io::ErrorKind::NotFound
    {
        log::warn!("could not remove temp file {}: {err}", temp.display());
    }
    result
}

fn write_temp(temp: &Path, contents: &str, permissions: Option<fs::Permissions>) -> io::Result<()> {
    let mut file = File::create_new(temp)?;
    file.write_all(contents.as_bytes())?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-editor-{tag}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn decode_refuses_binaries_and_invalid_utf8() {
        assert_eq!(
            decode_text(b"fn main() {}\n".to_vec()).unwrap(),
            "fn main() {}\n"
        );
        assert_eq!(decode_text(vec![0x89, b'P', 0, 1]), Err(ReadError::Binary));
        assert_eq!(decode_text(vec![0xff, 0xfe, b'a']), Err(ReadError::NotUtf8));
    }

    #[test]
    fn read_refuses_oversized_missing_and_directories() {
        let dir = temp_dir("read");
        let file = dir.join("big.txt");
        fs::write(&file, "0123456789").unwrap();
        assert_eq!(
            read_text_file(&file, 4),
            Err(ReadError::TooLarge {
                bytes: 10,
                limit: 4
            })
        );
        assert_eq!(read_text_file(&file, 64).unwrap(), "0123456789");
        assert_eq!(read_text_file(&dir, 64), Err(ReadError::NotAFile));
        assert!(matches!(
            read_text_file(&dir.join("missing"), 64),
            Err(ReadError::Io(_))
        ));
        let bin = dir.join("x.bin");
        fs::write(&bin, [1u8, 0, 2]).unwrap();
        assert_eq!(read_text_file(&bin, 64), Err(ReadError::Binary));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_replaces_contents_and_leaves_no_temp() {
        let dir = temp_dir("save");
        let file = dir.join("a.rs");
        fs::write(&file, "old").unwrap();
        atomic_write(&file, "new contents").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new contents");
        let entries: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "temp file left behind");
        atomic_write(&dir.join("fresh.txt"), "created").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("fresh.txt")).unwrap(),
            "created"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = temp_dir("perm");
        let file = dir.join("run.sh");
        fs::write(&file, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        atomic_write(&file, "#!/bin/sh\necho hi\n").unwrap();
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);

        let link = dir.join("link.sh");
        symlink(&file, &link).unwrap();
        atomic_write(&link, "via link").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "via link");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_write_fails_in_missing_directory() {
        let dir = temp_dir("missing");
        assert!(atomic_write(&dir.join("nope/a.txt"), "x").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn helpers_resolve_paths_and_count_lines() {
        assert_eq!(
            resolve_path("/ws", "src/a.rs"),
            PathBuf::from("/ws/src/a.rs")
        );
        assert_eq!(resolve_path("/ws", "/abs/b.rs"), PathBuf::from("/abs/b.rs"));
        assert_eq!(file_name("src/ui/a.rs"), "a.rs");
        assert_eq!(count_lines(""), 1);
        assert_eq!(count_lines("a\nb\n"), 3);
        assert_eq!(detect_language("x.tsx"), "TypeScript");
    }
}
