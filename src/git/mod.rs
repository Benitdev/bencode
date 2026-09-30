use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Renamed,
}

impl GitFileStatus {
    pub fn badge_char(&self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Untracked => "U",
            Self::Renamed => "R",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GitFileChange {
    pub path: String,
    pub status: GitFileStatus,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug)]
pub enum DiffLineKind {
    Header(String),
    Addition(String),
    Deletion(String),
    Context(String),
}

/// Retrieves list of modified, added, or untracked files for a workspace path
pub fn get_workspace_changes(cwd: &str) -> Vec<GitFileChange> {
    let p = Path::new(cwd);
    if !p.exists() {
        return Vec::new();
    }

    let output = match Command::new("git")
        .args(["-C", cwd, "status", "--porcelain=v1", "-uall"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }

        let (status_code, file_path) = line.split_at(3);
        let file_path = file_path.trim().to_string();

        let status = match status_code.trim() {
            "M" | "MM" | "AM" => GitFileStatus::Modified,
            "A" => GitFileStatus::Added,
            "D" => GitFileStatus::Deleted,
            "??" => GitFileStatus::Untracked,
            "R" => GitFileStatus::Renamed,
            _ => GitFileStatus::Modified,
        };

        // Determine additions / deletions
        let (additions, deletions) = get_file_numstat(cwd, &file_path, &status);

        files.push(GitFileChange {
            path: file_path,
            status,
            additions,
            deletions,
        });
    }

    files
}

fn get_file_numstat(cwd: &str, file_path: &str, status: &GitFileStatus) -> (usize, usize) {
    if *status == GitFileStatus::Untracked {
        let full_path = Path::new(cwd).join(file_path);
        if let Ok(content) = std::fs::read_to_string(full_path) {
            return (content.lines().count(), 0);
        }
        return (1, 0);
    }

    let output = match Command::new("git")
        .args(["-C", cwd, "diff", "--numstat", "HEAD", "--", file_path])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return (0, 0),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let adds = parts[0].parse::<usize>().unwrap_or(0);
            let dels = parts[1].parse::<usize>().unwrap_or(0);
            return (adds, dels);
        }
    }

    (0, 0)
}

/// Retrieves parsed diff lines for a specific file
pub fn get_file_diff(cwd: &str, file_path: &str) -> Vec<DiffLineKind> {
    let output = match Command::new("git")
        .args(["-C", cwd, "diff", "-U3", "HEAD", "--", file_path])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => {
            // Fallback for untracked file
            return get_untracked_diff(cwd, file_path);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return get_untracked_diff(cwd, file_path);
    }

    let mut diff_lines = Vec::new();
    let mut in_hunk = false;

    for line in stdout.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            diff_lines.push(DiffLineKind::Header(line.to_string()));
        } else if in_hunk {
            if line.starts_with('+') && !line.starts_with("+++") {
                diff_lines.push(DiffLineKind::Addition(line[1..].to_string()));
            } else if line.starts_with('-') && !line.starts_with("---") {
                diff_lines.push(DiffLineKind::Deletion(line[1..].to_string()));
            } else if line.starts_with(' ') {
                diff_lines.push(DiffLineKind::Context(line[1..].to_string()));
            } else if line.is_empty() {
                diff_lines.push(DiffLineKind::Context(String::new()));
            }
        }
    }

    diff_lines
}

fn get_untracked_diff(cwd: &str, file_path: &str) -> Vec<DiffLineKind> {
    let full_path = Path::new(cwd).join(file_path);
    if let Ok(content) = std::fs::read_to_string(&full_path) {
        let mut lines = vec![DiffLineKind::Header(format!("@@ -0,0 +1,{} @@ (new file)", content.lines().count()))];
        for line in content.lines() {
            lines.push(DiffLineKind::Addition(line.to_string()));
        }
        lines
    } else {
        vec![DiffLineKind::Header(format!("@@ New untracked file: {} @@", file_path))]
    }
}
