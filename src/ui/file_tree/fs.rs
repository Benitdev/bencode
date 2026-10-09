//! The Explorer's disk side, after MonoCode's `src-tauri/src/fs.rs`:
//! listing a folder (`list_dir_sync`), and creating, renaming, copying,
//! moving and deleting entries. Nothing here replaces an existing entry.
//! Paths are workspace-relative; `root` is the workspace folder.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use super::FsEntry;
use super::name::{join, parent_of, segments};

/// `git check-ignore` exits 0 when something matched, 1 when nothing did.
const CHECK_IGNORE_MATCHED: i32 = 0;
const CHECK_IGNORE_NONE: i32 = 1;
/// MonoCode gives up looking for a free "copy N" name after this many.
const MAX_COPY_ATTEMPTS: u32 = 1000;

/// Refuses the root itself and any `..`: every change below names an
/// entry inside the project.
fn entry_rel(rel: &str) -> Result<&str, String> {
    if rel.is_empty() || rel.split('/').any(|p| p == ".." || p == ".") {
        return Err(format!("{rel:?} is not an entry inside the project."));
    }
    Ok(rel)
}

/// The deepest existing folder of `path`, resolved through symlinks, must
/// be inside `root` (so a symlinked folder cannot lead writes outside).
fn ensure_inside(root: &Path, path: &Path) -> Result<(), String> {
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut probe = path.to_path_buf();
    while probe.symlink_metadata().is_err() {
        if !probe.pop() {
            break;
        }
    }
    let resolved = std::fs::canonicalize(&probe).unwrap_or(probe);
    if resolved.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err("A file or folder must stay inside the project.".into())
    }
}

fn abs(root: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    }
}

/// MonoCode `list_dir_sync`: folders first, natural order, `.DS_Store`
/// hidden, `.git` and git-ignored names marked. A folder that cannot be read
/// is an error the tree shows under it.
pub fn list_dir(root: &Path, rel_dir: &str) -> Result<Vec<FsEntry>, String> {
    let dir = abs(root, rel_dir);
    let reader = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in reader.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name == ".DS_Store" {
            continue;
        }
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir() || (t.is_symlink() && path.is_dir()))
            .unwrap_or_else(|_| path.is_dir());
        out.push(FsEntry {
            rel_path: join(rel_dir, &name),
            name,
            is_dir,
            ignored: false,
        });
    }
    let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
    let ignored = git_ignored_names(&dir, &names).unwrap_or_else(|| gitignore_names(&dir, &names));
    for entry in &mut out {
        entry.ignored = entry.name == ".git" || ignored.contains(&entry.name);
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(out)
}

/// MonoCode `git_ignored_names`: names git ignores here, or `None` outside
/// a repository.
fn git_ignored_names(dir: &Path, names: &[&str]) -> Option<HashSet<String>> {
    if names.is_empty() {
        return Some(HashSet::new());
    }
    let mut child = crate::git::git_command(dir)
        .args(["check-ignore", "--stdin", "-z"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut input = Vec::new();
    for name in names {
        input.extend_from_slice(name.as_bytes());
        input.push(0);
    }
    let mut stdin = child.stdin.take()?;
    // A large listing can fill stdout while git still reads stdin.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().ok()?;
    writer.join().ok()?.ok()?;
    if !matches!(
        output.status.code(),
        Some(CHECK_IGNORE_MATCHED | CHECK_IGNORE_NONE)
    ) {
        return None;
    }
    Some(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

/// Outside git, MonoCode reads the folder's own `.gitignore` (plain names
/// and `*` globs, no negation).
fn gitignore_names(dir: &Path, names: &[&str]) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(".gitignore")) else {
        return HashSet::new();
    };
    let patterns: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .map(|l| l.trim_start_matches('/').trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty() && !l.contains('/'))
        .collect();
    names
        .iter()
        .filter(|name| patterns.iter().any(|p| glob_match(p, name)))
        .map(|name| name.to_string())
        .collect()
}

/// `*` matches any run of characters, `?` one.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// MonoCode `compare_natural_names`: digit runs compare by value
/// (`chapter-9` before `chapter-10`), letters ignore case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (sa, sb) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let strip = |d: &[u8]| -> Vec<u8> {
                let start = d.iter().position(|c| *c != b'0').unwrap_or(d.len());
                d[start..].to_vec()
            };
            let (va, vb) = (strip(&a[sa..i]), strip(&b[sb..j]));
            let order = va.len().cmp(&vb.len()).then_with(|| va.cmp(&vb));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| a.cmp(b))
}

fn already_exists(label: &str) -> String {
    format!(
        "A file or folder {label} already exists at this location. Please choose a different name."
    )
}

/// `name` (well-formed, may nest) under `parent`, refusing to leave it.
fn resolve_under(root: &Path, parent: &str, name: &str) -> Result<String, String> {
    let parts = segments(name);
    if parts.is_empty() || parts.iter().any(|p| p == "." || p == "..") {
        return Err(format!(
            "The name {name} is not valid as a file or folder name. Please choose a different name."
        ));
    }
    let rel = parts
        .iter()
        .fold(parent.to_string(), |acc, part| join(&acc, part));
    ensure_inside(root, &abs(root, &rel))?;
    Ok(rel)
}

/// MonoCode `create_path`: a file or folder (nested folders made on the
/// way). Returns the new entry's relative path.
pub fn create(root: &Path, parent: &str, name: &str, is_dir: bool) -> Result<String, String> {
    let rel = resolve_under(root, parent, name)?;
    let dest = abs(root, &rel);
    let label = segments(name).pop().unwrap_or_default();
    if dest.symlink_metadata().is_ok() {
        return Err(already_exists(&label));
    }
    if is_dir {
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    } else {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::File::create_new(&dest).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                already_exists(&label)
            } else {
                e.to_string()
            }
        })?;
    }
    Ok(rel)
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// MonoCode `rename_path_sync`: renames within the parent (`/` nests); a
/// case-only rename goes through a temporary name.
pub fn rename(root: &Path, rel: &str, name: &str) -> Result<String, String> {
    let rel = entry_rel(rel)?;
    let from = abs(root, rel);
    if from.symlink_metadata().is_err() {
        return Err(format!("{}: No such file or directory", from.display()));
    }
    let parent = parent_of(rel);
    let dest_rel = resolve_under(root, &parent, name)?;
    let dest = abs(root, &dest_rel);
    if dest_rel == rel {
        return Ok(rel.to_string());
    }
    // A case-only rename of the same entry (by name, never by following
    // symlinks) goes through a temporary name.
    if dest_rel.to_lowercase() == rel.to_lowercase() {
        let tmp = abs(root, &parent).join(format!(".bencode-rename-{}", stamp()));
        std::fs::rename(&from, &tmp).map_err(|e| e.to_string())?;
        if let Err(err) = std::fs::rename(&tmp, &dest) {
            if let Err(back) = std::fs::rename(&tmp, &from) {
                log::error!(
                    "could not restore {} after a failed rename: {back}",
                    from.display()
                );
            }
            return Err(err.to_string());
        }
        return Ok(dest_rel);
    }
    if dest.symlink_metadata().is_ok() {
        return Err(already_exists(&segments(name).pop().unwrap_or_default()));
    }
    if dest.starts_with(&from) {
        return Err("Cannot move a folder into itself.".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&from, &dest).map_err(|e| e.to_string())?;
    Ok(dest_rel)
}

/// MonoCode `delete_path_sync` (permanent, like MonoCode).
pub fn delete(root: &Path, rel: &str) -> Result<(), String> {
    let path = abs(root, entry_rel(rel)?);
    if path.symlink_metadata().is_err() {
        return Err(format!("{}: No such file or directory", path.display()));
    }
    let result = if path.is_dir() && !path.is_symlink() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    result.map_err(|e| format!("{}: {e}", path.display()))
}

fn split_stem_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// MonoCode `unique_name_in`: `name`, then `name copy`, `name copy 2`, …
fn unique_name_in(dir: &Path, name: &str) -> String {
    let (stem, ext) = split_stem_ext(name);
    for n in 0..=MAX_COPY_ATTEMPTS {
        let candidate = match n {
            0 => name.to_string(),
            1 => format!("{stem} copy{ext}"),
            _ => format!("{stem} copy {n}{ext}"),
        };
        if dir.join(&candidate).symlink_metadata().is_err() {
            return candidate;
        }
    }
    format!("{stem} copy {}{ext}", stamp())
}

/// Copies `from` to `to`; a symlink is copied as a link, never followed,
/// so a link to an ancestor cannot recurse.
fn copy_recursive(from: &Path, to: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from).map_err(|e| format!("{}: {e}", from.display()))?;
        #[cfg(unix)]
        return std::os::unix::fs::symlink(&target, to)
            .map_err(|e| format!("{}: {e}", to.display()));
        #[cfg(not(unix))]
        return Err(format!(
            "{}: cannot copy a symbolic link here ({})",
            from.display(),
            target.display()
        ));
    }
    if meta.is_dir() {
        std::fs::create_dir(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)
            .map(drop)
            .map_err(|e| format!("{}: {e}", to.display()))
    }
}

/// Whether folder `dir` holds `inner`. A symlink is itself, not its target.
fn dir_contains(dir: &Path, inner: &Path) -> bool {
    if dir.is_symlink() {
        return false;
    }
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let inner = std::fs::canonicalize(inner).unwrap_or_else(|_| inner.to_path_buf());
    inner.starts_with(&dir)
}

/// MonoCode `copy_path_sync`: copies `from` (absolute, so Finder files
/// work too) into `dest_parent` under a free name. Returns the copy.
pub fn copy_into(root: &Path, from: &Path, dest_parent: &str) -> Result<String, String> {
    if from.symlink_metadata().is_err() {
        return Err(format!("{}: No such file or directory", from.display()));
    }
    let dest_dir = abs(root, dest_parent);
    if !dest_dir.is_dir() {
        return Err(format!("{} is not a folder", dest_dir.display()));
    }
    if from.is_dir() && dir_contains(from, &dest_dir) {
        return Err("Cannot paste a folder into itself.".into());
    }
    let label = from
        .file_name()
        .map_or_else(|| "copy".to_string(), |n| n.to_string_lossy().into_owned());
    ensure_inside(root, &dest_dir)?;
    let name = unique_name_in(&dest_dir, &label);
    let dest = dest_dir.join(&name);
    if let Err(err) = copy_recursive(from, &dest) {
        // Leave nothing half-copied behind.
        let cleanup = if dest.is_dir() && !dest.is_symlink() {
            std::fs::remove_dir_all(&dest)
        } else {
            std::fs::remove_file(&dest)
        };
        if let Err(clean_err) = cleanup
            && dest.symlink_metadata().is_ok()
        {
            log::error!(
                "could not remove partial copy {}: {clean_err}",
                dest.display()
            );
        }
        return Err(err);
    }
    Ok(join(dest_parent, &name))
}

/// MonoCode `move_path_sync`: moves `rel` into `dest_parent`, keeping its
/// name. Returns where it went.
pub fn move_into(root: &Path, rel: &str, dest_parent: &str) -> Result<String, String> {
    let rel = entry_rel(rel)?;
    let from = abs(root, rel);
    if from.symlink_metadata().is_err() {
        return Err(format!("{}: No such file or directory", from.display()));
    }
    let dest_dir = abs(root, dest_parent);
    if !dest_dir.is_dir() {
        return Err(format!("{} is not a folder", dest_dir.display()));
    }
    if from.is_dir() && dir_contains(&from, &dest_dir) {
        return Err("Cannot paste a folder into itself.".into());
    }
    let name = from
        .file_name()
        .map_or_else(|| "item".to_string(), |n| n.to_string_lossy().into_owned());
    let dest_rel = join(dest_parent, &name);
    let dest = abs(root, &dest_rel);
    if dest_rel == rel {
        return Ok(rel.to_string());
    }
    ensure_inside(root, &dest_dir)?;
    if dest.symlink_metadata().is_ok() {
        return Err(already_exists(&name));
    }
    std::fs::rename(&from, &dest).map_err(|e| e.to_string())?;
    Ok(dest_rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bencode-fs-{tag}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_followed_into_damage() {
        let dir = temp_dir("links");
        std::fs::write(dir.join("real.txt"), "keep").unwrap();
        std::os::unix::fs::symlink(dir.join("real.txt"), dir.join("link")).unwrap();
        assert!(
            rename(&dir, "link", "real.txt").is_err(),
            "must not replace the target"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("real.txt")).unwrap(),
            "keep"
        );
        std::fs::create_dir_all(dir.join("loop")).unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("loop/up")).unwrap();
        assert_eq!(copy_into(&dir, &dir.join("loop"), "").unwrap(), "loop copy");
        assert!(dir.join("loop copy/up").is_symlink());
        let outside = temp_dir("outside");
        std::os::unix::fs::symlink(&outside, dir.join("out")).unwrap();
        assert!(create(&dir, "out", "x.txt", false).is_err());
        assert!(!outside.join("x.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut names = vec!["chapter-10", "Chapter-9", "chapter-1", "b", "A"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["A", "b", "chapter-1", "Chapter-9", "chapter-10"]);
    }

    #[test]
    fn listing_puts_folders_first_and_marks_ignored_names() {
        let dir = temp_dir("list");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("dist")).unwrap();
        std::fs::write(dir.join(".gitignore"), "dist/\n*.log\n").unwrap();
        std::fs::write(dir.join("a.log"), "").unwrap();
        std::fs::write(dir.join("main.rs"), "").unwrap();
        std::fs::write(dir.join(".DS_Store"), "").unwrap();
        let entries = list_dir(&dir, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dist", "src", ".gitignore", "a.log", "main.rs"]);
        let ignored: Vec<&str> = entries
            .iter()
            .filter(|e| e.ignored)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(ignored, ["dist", "a.log"]);
        assert!(list_dir(&dir, "missing").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_nests_and_never_overwrites() {
        let dir = temp_dir("create");
        assert_eq!(create(&dir, "", "a/b/c.ts", false).unwrap(), "a/b/c.ts");
        assert!(dir.join("a/b/c.ts").is_file());
        assert!(create(&dir, "a/b", "c.ts", false).is_err());
        assert_eq!(create(&dir, "a", "d", true).unwrap(), "a/d");
        assert!(create(&dir, "", "../escape", false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_refuses_to_replace_and_allows_case_changes() {
        let dir = temp_dir("rename");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        assert!(rename(&dir, "a.txt", "b.txt").is_err());
        assert_eq!(rename(&dir, "a.txt", "A.txt").unwrap(), "A.txt");
        assert_eq!(rename(&dir, "A.txt", "sub/c.txt").unwrap(), "sub/c.txt");
        assert_eq!(std::fs::read_to_string(dir.join("sub/c.txt")).unwrap(), "a");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copies_get_free_names_and_folders_cannot_enter_themselves() {
        let dir = temp_dir("copy");
        std::fs::create_dir_all(dir.join("f")).unwrap();
        std::fs::write(dir.join("x.rs"), "x").unwrap();
        assert_eq!(copy_into(&dir, &dir.join("x.rs"), "").unwrap(), "x copy.rs");
        assert_eq!(
            copy_into(&dir, &dir.join("x.rs"), "").unwrap(),
            "x copy 2.rs"
        );
        assert_eq!(copy_into(&dir, &dir.join("x.rs"), "f").unwrap(), "f/x.rs");
        assert!(copy_into(&dir, &dir.join("f"), "f").is_err());
        assert!(move_into(&dir, "x.rs", "f").is_err(), "f/x.rs exists");
        assert_eq!(move_into(&dir, "x copy.rs", "f").unwrap(), "f/x copy.rs");
        assert!(delete(&dir, "").is_err(), "never the root");
        assert!(move_into(&dir, "../x", "").is_err());
        delete(&dir, "f").unwrap();
        assert!(!dir.join("f").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
