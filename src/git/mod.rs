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

/// Retrieves list of local git branches for a workspace path
pub fn get_branches(cwd: &str) -> Vec<String> {
    let output = match Command::new("git")
        .args(["-C", cwd, "branch", "--format=%(refname:short)"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return vec!["main".to_string()],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut branches: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if branches.is_empty() {
        branches.push("main".to_string());
    }
    branches
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

#[derive(Clone, Debug)]
pub struct GitCommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub relative_time: String,
}

#[derive(Clone, Debug, Default)]
pub struct GitDetailedStatus {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged: Vec<GitFileChange>,
    pub unstaged: Vec<GitFileChange>,
}

pub fn get_detailed_status(cwd: &str) -> GitDetailedStatus {
    let p = Path::new(cwd);
    if !p.exists() {
        return GitDetailedStatus::default();
    }

    // 1. Current branch
    let branch = Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .output()
        .ok()
        .and_then(|out| {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
        .unwrap_or_else(|| "main".to_string());

    // 2. Ahead / Behind
    let (ahead, behind) = Command::new("git")
        .args(["-C", cwd, "rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
        .output()
        .ok()
        .map(|out| {
            let s = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() == 2 {
                (parts[1].parse::<usize>().unwrap_or(0), parts[0].parse::<usize>().unwrap_or(0))
            } else {
                (0, 0)
            }
        })
        .unwrap_or((0, 0));

    // 3. Status porcelain
    let output = match Command::new("git")
        .args(["-C", cwd, "status", "--porcelain=v1", "-uall"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return GitDetailedStatus { branch, ahead, behind, staged: Vec::new(), unstaged: Vec::new() },
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }

        let staged_char = line.chars().next().unwrap_or(' ');
        let unstaged_char = line.chars().nth(1).unwrap_or(' ');
        let file_path = line[3..].trim().to_string();

        // Staged files (staged_char != ' ' and staged_char != '?')
        if staged_char != ' ' && staged_char != '?' {
            let status = match staged_char {
                'M' => GitFileStatus::Modified,
                'A' => GitFileStatus::Added,
                'D' => GitFileStatus::Deleted,
                'R' => GitFileStatus::Renamed,
                _ => GitFileStatus::Modified,
            };
            let (additions, deletions) = get_file_numstat(cwd, &file_path, &status);
            staged.push(GitFileChange {
                path: file_path.clone(),
                status,
                additions,
                deletions,
            });
        }

        // Unstaged files (unstaged_char != ' ' or staged_char == '?')
        if unstaged_char != ' ' || staged_char == '?' {
            let status = if staged_char == '?' {
                GitFileStatus::Untracked
            } else {
                match unstaged_char {
                    'M' => GitFileStatus::Modified,
                    'D' => GitFileStatus::Deleted,
                    'A' => GitFileStatus::Added,
                    _ => GitFileStatus::Modified,
                }
            };
            let (additions, deletions) = get_file_numstat(cwd, &file_path, &status);
            unstaged.push(GitFileChange {
                path: file_path,
                status,
                additions,
                deletions,
            });
        }
    }

    GitDetailedStatus {
        branch,
        ahead,
        behind,
        staged,
        unstaged,
    }
}

pub fn stage_file(cwd: &str, file: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["-C", cwd, "add", "--", file])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn unstage_file(cwd: &str, file: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["-C", cwd, "reset", "HEAD", "--", file])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn discard_file(cwd: &str, file: &str) -> Result<(), String> {
    let full = Path::new(cwd).join(file);
    if full.exists() {
        // Try checkout first
        let _ = Command::new("git")
            .args(["-C", cwd, "checkout", "--", file])
            .output();
        // If untracked file still exists, remove it
        if full.is_file() {
            let _ = std::fs::remove_file(&full);
        } else if full.is_dir() {
            let _ = std::fs::remove_dir_all(&full);
        }
    }
    Ok(())
}

pub fn stage_all(cwd: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["-C", cwd, "add", "-A"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn unstage_all(cwd: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["-C", cwd, "reset"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn discard_all(cwd: &str) -> Result<(), String> {
    let _ = Command::new("git")
        .args(["-C", cwd, "checkout", "--", "."])
        .output();
    let _ = Command::new("git")
        .args(["-C", cwd, "clean", "-fd"])
        .output();
    Ok(())
}

pub fn commit(cwd: &str, message: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["-C", cwd, "commit", "-m", message])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

pub fn get_recent_commits(cwd: &str, count: usize) -> Vec<GitCommitInfo> {
    let out = match Command::new("git")
        .args(["-C", cwd, "log", &format!("-n{}", count), "--format=%H|%h|%s|%an|%cr"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 5 {
                Some(GitCommitInfo {
                    hash: parts[0].to_string(),
                    short_hash: parts[1].to_string(),
                    message: parts[2].to_string(),
                    author: parts[3].to_string(),
                    relative_time: parts[4].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}
