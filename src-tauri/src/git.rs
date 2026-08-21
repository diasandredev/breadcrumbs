use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;

use crate::db;
use crate::models::Commit;

/// Default workspace root scanned for repositories.
fn default_roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|h| vec![h.join("Git")])
        .unwrap_or_default()
}

/// Workspace roots from settings (JSON array), falling back to ~/Git.
fn workspace_roots(conn: &Connection) -> Vec<PathBuf> {
    match db::get_state(conn, "workspace_roots") {
        Some(json) => serde_json::from_str::<Vec<String>>(&json)
            .map(|v| v.into_iter().map(PathBuf::from).collect())
            .unwrap_or_else(|_| default_roots()),
        None => default_roots(),
    }
}

/// Walks workspace roots up to 2 levels deep registering every git repository
/// as a project so its commits show up even without OpenCode sessions there.
pub fn discover_repos(diary: &Connection) -> Result<usize, String> {
    let roots = workspace_roots(diary);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut count = 0usize;
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            if p.join(".git").exists() {
                if register_repo(diary, &p, now_ms) {
                    count += 1;
                }
                continue;
            }
            // one nesting level deeper (e.g. ~/Git/org/repo)
            if let Ok(subs) = std::fs::read_dir(&p) {
                for sub in subs.flatten() {
                    let sp = sub.path();
                    if sp.is_dir() && sp.join(".git").exists() && register_repo(diary, &sp, now_ms)
                    {
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(count)
}

fn register_repo(diary: &Connection, path: &Path, now_ms: i64) -> bool {
    let path_str = path.to_string_lossy();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let before: i64 = diary
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE path = ?1",
            [&path_str],
            |r| r.get(0),
        )
        .unwrap_or(0);
    match db::upsert_project(diary, &path_str, name, now_ms) {
        Ok(_) => before == 0,
        Err(_) => false,
    }
}

/// Fetch recent commits for every known project and upsert them into the
/// diary. Missing dirs or non-repos are skipped silently.
pub fn sync_projects(diary: &Connection) -> Result<u32, String> {
    let mut stmt = diary
        .prepare("SELECT id, path FROM projects")
        .map_err(|e| e.to_string())?;
    let projects: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut total = 0u32;
    for (project_id, path) in &projects {
        let commits = match log_recent(Path::new(path), 300) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for c in commits {
            let inserted = diary
                .execute(
                    "INSERT OR IGNORE INTO commits(hash, project_id, message, author, committed_at_ms)
                     VALUES(?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![c.hash, project_id, c.message, c.author, c.committed_at_ms],
                )
                .map_err(|e| e.to_string())?;
            total += inserted as u32;
        }
    }
    Ok(total)
}

fn log_recent(repo: &Path, limit: u32) -> Result<Vec<Commit>, String> {
    if !repo.join(".git").exists() {
        return Err("not a git repository".into());
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            &format!("-n{limit}"),
            "--pretty=format:%H%x1f%an%x1f%at%x1f%s",
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let sep = '\u{1f}';
    let mut commits = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split(sep).collect();
        if parts.len() != 4 || parts[0].is_empty() {
            continue;
        }
        let secs: i64 = parts[2].parse().unwrap_or(0);
        commits.push(Commit {
            hash: parts[0].to_string(),
            author: Some(parts[1].to_string()).filter(|a| !a.is_empty()),
            committed_at_ms: secs * 1000,
            message: parts[3].to_string(),
            project_name: None,
        });
    }
    Ok(commits)
}

/// Commits for one project within a time window [from_ms, to_ms].
pub fn in_window(
    diary: &Connection,
    project_id: &str,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<Commit>, String> {
    let mut stmt = diary
        .prepare(
            "SELECT hash, message, author, committed_at_ms
             FROM commits
             WHERE project_id = ?1 AND committed_at_ms BETWEEN ?2 AND ?3
             ORDER BY committed_at_ms ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![project_id, from_ms, to_ms], |r| {
            Ok(Commit {
                hash: r.get(0)?,
                message: r.get(1)?,
                author: r.get(2)?,
                committed_at_ms: r.get(3)?,
                project_name: None,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
