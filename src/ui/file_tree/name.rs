//! MonoCode `fileName.ts`: what the Explorer's inline name field accepts.
//! A name may hold `/` to create nested folders; a trailing `/` makes a
//! folder; only tabs are trimmed.

/// MonoCode `NameIssue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameIssue {
    Empty,
    Slash,
    Exists(String),
    Invalid(String),
    /// A warning: the name still works.
    Whitespace,
}

impl NameIssue {
    pub fn is_error(&self) -> bool {
        !matches!(self, NameIssue::Whitespace)
    }

    /// Where [`Self::message`] names the entry, which MonoCode sets in
    /// `font-semibold`.
    pub fn emphasis(&self) -> Option<std::ops::Range<usize>> {
        let (prefix, name) = match self {
            NameIssue::Exists(name) => ("A file or folder ", name),
            NameIssue::Invalid(name) => ("The name ", name),
            _ => return None,
        };
        Some(prefix.len()..prefix.len() + name.len())
    }

    /// MonoCode `NameIssueView` text.
    pub fn message(&self) -> String {
        match self {
            NameIssue::Empty => "A file or folder name must be provided.".into(),
            NameIssue::Slash => "A file or folder name cannot start with a slash.".into(),
            NameIssue::Exists(name) => format!(
                "A file or folder {name} already exists at this location. Please choose a different name."
            ),
            NameIssue::Invalid(name) => format!(
                "The name {name} is not valid as a file or folder name. Please choose a different name."
            ),
            NameIssue::Whitespace => {
                "Leading or trailing whitespace detected in file or folder name.".into()
            }
        }
    }
}

/// MonoCode `wellFormedFileName`: tabs off both ends, trailing slashes off.
pub fn well_formed(raw: &str) -> String {
    raw.trim_matches('\t')
        .trim_end_matches(['/', '\\'])
        .to_string()
}

/// MonoCode `pathSegments`.
pub fn segments(raw: &str) -> Vec<String> {
    well_formed(raw)
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// MonoCode `leafName`: the last segment, for the row's live icon.
pub fn leaf(raw: &str) -> String {
    segments(raw).pop().unwrap_or_default()
}

fn valid_segment(name: &str) -> bool {
    !(name.trim().is_empty()
        || name.contains(['/', '\\', '\0'])
        || name == "."
        || name == ".."
        || name.len() > 255)
}

/// MonoCode `validateFileName` against the target folder's names.
pub fn validate(raw: &str, siblings: &[String]) -> Option<NameIssue> {
    let name = well_formed(raw);
    if name.trim().is_empty() {
        return Some(NameIssue::Empty);
    }
    if name.starts_with(['/', '\\']) {
        return Some(NameIssue::Slash);
    }
    let lower = name.to_lowercase();
    if siblings.iter().any(|s| s.to_lowercase() == lower) {
        return Some(NameIssue::Exists(name));
    }
    let parts = segments(&name);
    if parts.iter().any(|s| !valid_segment(s)) {
        return Some(NameIssue::Invalid(name));
    }
    if parts
        .iter()
        .any(|s| s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace))
    {
        return Some(NameIssue::Whitespace);
    }
    None
}

/// MonoCode `dirsTouchedByCreate`: `parent` and every folder a nested name
/// creates under it (relative paths, `""` for the root).
pub fn dirs_touched_by_create(parent: &str, raw: &str) -> Vec<String> {
    let parts = segments(raw);
    let mut out = vec![parent.to_string()];
    let mut current = parent.to_string();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        current = join(&current, part);
        out.push(current.clone());
    }
    out
}

/// `parent/name`, or `name` at the root.
pub fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// The folder holding `rel` (`""` for a root entry).
pub fn parent_of(rel: &str) -> String {
    rel.rsplit_once('/').map_or(String::new(), |(dir, _)| dir.to_string())
}

/// MonoCode `rebasePath`: `path` moved along when `from` became `to`.
pub fn rebase(path: &str, from: &str, to: &str) -> String {
    if path == from {
        return to.to_string();
    }
    match path.strip_prefix(from).and_then(|rest| rest.strip_prefix('/')) {
        Some(rest) => join(to, rest),
        None => path.to_string(),
    }
}

/// Whether `path` is `dir` or inside it.
pub fn is_within(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_covers_the_name_in_the_message() {
        let issue = NameIssue::Exists("file".into());
        let range = issue.emphasis().expect("exists names the entry");
        assert_eq!(&issue.message()[range], "file");
        let issue = NameIssue::Invalid("a:b".into());
        assert_eq!(&issue.message()[issue.emphasis().unwrap()], "a:b");
        assert!(NameIssue::Empty.emphasis().is_none());
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn validation_matches_monocode() {
        let siblings = names(&["Main.rs", "src"]);
        assert_eq!(validate("", &siblings), Some(NameIssue::Empty));
        assert_eq!(validate("\t \t", &siblings), Some(NameIssue::Empty));
        assert_eq!(validate("/abs", &siblings), Some(NameIssue::Slash));
        assert_eq!(validate("main.rs", &siblings), Some(NameIssue::Exists("main.rs".into())));
        assert_eq!(validate("a/../b", &siblings), Some(NameIssue::Invalid("a/../b".into())));
        assert_eq!(validate(" padded", &siblings), Some(NameIssue::Whitespace));
        assert!(!NameIssue::Whitespace.is_error());
        assert_eq!(validate("lib/util.rs", &siblings), None);
        assert_eq!(validate("newdir/", &siblings), None);
    }

    #[test]
    fn nested_names_touch_every_new_folder() {
        assert_eq!(dirs_touched_by_create("", "a/b/c.ts"), ["", "a", "a/b"]);
        assert_eq!(dirs_touched_by_create("src", "x.rs"), ["src"]);
        assert_eq!(leaf("a/b/c.ts"), "c.ts");
        assert_eq!(well_formed("\tdir//"), "dir");
    }

    #[test]
    fn paths_rebase_under_a_moved_folder() {
        assert_eq!(rebase("src/a/b.rs", "src/a", "src/z"), "src/z/b.rs");
        assert_eq!(rebase("src/a", "src/a", "lib"), "lib");
        assert_eq!(rebase("src/ab", "src/a", "x"), "src/ab");
        assert!(is_within("src/a/b", "src/a") && !is_within("src/ab", "src/a"));
        assert_eq!(parent_of("a/b/c"), "a/b");
        assert_eq!(parent_of("c"), "");
    }
}
