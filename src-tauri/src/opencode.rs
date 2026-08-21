use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use crate::db;
use crate::models::SyncStats;

const LAST_UPDATED_KEY: &str = "opencode_last_time_updated";
const BATCH_SIZE: i64 = 500;

pub fn source_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local/share/opencode/opencode.db"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn open_source(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())
}

/// Fallback when the WAL cannot be initialized read-only (e.g. opencode not
/// running): copy db + wal/shm sidecars into a temp dir and read that.
fn open_source_via_copy(path: &Path) -> Result<Connection, String> {
    let tmp_base = std::env::temp_dir().join("breadcrumbs-opencode-copy");
    let _ = std::fs::remove_dir_all(&tmp_base);
    std::fs::create_dir_all(&tmp_base).map_err(|e| e.to_string())?;
    for suffix in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{}", path.display(), suffix));
        if src.exists() {
            let dst = PathBuf::from(format!(
                "{}{}",
                tmp_base.join("opencode.db").display(),
                suffix
            ));
            std::fs::copy(&src, &dst).map_err(|e| format!("copy failed: {e}"))?;
        }
    }
    open_source(&tmp_base.join("opencode.db"))
}

fn table_columns(conn: &Connection, table: &str) -> HashSet<String> {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return HashSet::new(),
    };
    let rows = match stmt.query_map([], |r| r.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return HashSet::new(),
    };
    rows.filter_map(|r| r.ok()).collect()
}

struct SourceSchema {
    has_parent_id: bool,
    has_message_table: bool,
    select_cols: Vec<&'static str>,
}

const OPTIONAL_COLS: &[&str] = &[
    "agent",
    "model",
    "cost",
    "tokens_input",
    "tokens_output",
    "tokens_reasoning",
    "tokens_cache_read",
    "tokens_cache_write",
    "summary_additions",
    "summary_deletions",
    "summary_files",
];

fn inspect_source(conn: &Connection) -> Result<SourceSchema, String> {
    let cols = table_columns(conn, "session");
    let mut required = ["id", "directory", "title", "time_created", "time_updated"];
    required.sort_unstable();

    let missing: Vec<_> = required
        .iter()
        .filter(|c| !cols.contains(**c))
        .collect();
    if !missing.is_empty() {
        return Err(format!("opencode.db schema mismatch, missing: {missing:?}"));
    }

    let present: Vec<&'static str> = OPTIONAL_COLS
        .iter()
        .copied()
        .filter(|c| cols.contains(*c))
        .collect();

    Ok(SourceSchema {
        has_parent_id: cols.contains("parent_id"),
        has_message_table: table_columns(conn, "message").contains("session_id"),
        select_cols: present,
    })
}

fn parse_model(raw: Option<String>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        // not JSON — treat as plain model id
        return (None, Some(raw));
    };
    (
        v.get("providerID")
            .and_then(|x| x.as_str())
            .map(String::from),
        v.get("id").and_then(|x| x.as_str()).map(String::from),
    )
}

pub fn sync(diary: &Connection) -> Result<SyncStats, String> {
    let src_path =
        source_db_path().ok_or_else(|| "cannot resolve home directory".to_string())?;
    if !src_path.exists() {
        return Err(format!("OpenCode database not found at {}", src_path.display()));
    }

    let src = open_source(&src_path)
        .or_else(|_| open_source_via_copy(&src_path))?;
    let schema = inspect_source(&src)?;

    let last: i64 = db::get_state(diary, LAST_UPDATED_KEY)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let col_list = {
        let mut v: Vec<String> = vec![
            "id".into(),
            "directory".into(),
            "title".into(),
            "time_created".into(),
            "time_updated".into(),
        ];
        v.extend(schema.select_cols.iter().map(|c| c.to_string()));
        if schema.has_parent_id {
            v.push("parent_id".into());
        }
        v.join(", ")
    };

    let msg_count_expr = if schema.has_message_table {
        "(SELECT COUNT(*) FROM message m WHERE m.session_id = s.id)".to_string()
    } else {
        "0".to_string()
    };

    let base_sql = format!(
        "SELECT {col_list}, {msg_count} AS msg_count FROM session s \
         WHERE time_updated > ?1 AND time_created != time_updated \
         ORDER BY time_updated ASC LIMIT {BATCH_SIZE}",
        col_list = col_list,
        msg_count = msg_count_expr,
    );

    let mut stats = SyncStats {
        new_sessions: 0,
        updated_sessions: 0,
    };
    let mut cursor = last;

    loop {
        let mut stmt = src.prepare(&base_sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([cursor], row_to_raw)
            .map_err(|e| e.to_string())?;

        let mut batch_count = 0i64;
        for raw in rows {
            let raw = raw.map_err(|e| e.to_string())?;
            batch_count += 1;
            cursor = cursor.max(raw.time_updated);

            if raw.is_subagent || is_noise(&raw) {
                continue;
            }

            let (provider, model_id) = parse_model(raw.model.clone());
            let root = git_root_for(&raw.directory);
            let project_id = db::upsert_project(diary, &root.1, &root.0, now_ms())
                .map_err(|e| e.to_string())?;

            let existed: bool = diary
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                    [&raw.id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);

            diary
                .execute(
                    "INSERT INTO sessions(id, project_id, title, agent, model_provider, model_id,
                        tokens_input, tokens_output, tokens_reasoning, cost,
                        additions, deletions, files_changed, message_count,
                        started_at_ms, ended_at_ms, synced_at_ms)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
                     ON CONFLICT(id) DO UPDATE SET
                        title=excluded.title,
                        agent=excluded.agent,
                        model_provider=excluded.model_provider,
                        model_id=excluded.model_id,
                        tokens_input=excluded.tokens_input,
                        tokens_output=excluded.tokens_output,
                        tokens_reasoning=excluded.tokens_reasoning,
                        cost=excluded.cost,
                        additions=excluded.additions,
                        deletions=excluded.deletions,
                        files_changed=excluded.files_changed,
                        message_count=excluded.message_count,
                        ended_at_ms=excluded.ended_at_ms,
                        synced_at_ms=excluded.synced_at_ms",
                    rusqlite::params![
                        raw.id,
                        project_id,
                        raw.title,
                        raw.agent,
                        provider,
                        model_id,
                        raw.tokens_input,
                        raw.tokens_output,
                        raw.tokens_reasoning,
                        raw.cost,
                        raw.additions,
                        raw.deletions,
                        raw.files_changed,
                        raw.message_count,
                        raw.time_created,
                        raw.time_updated,
                        now_ms(),
                    ],
                )
                .map_err(|e| e.to_string())?;

            if existed {
                stats.updated_sessions += 1;
            } else {
                stats.new_sessions += 1;
            }
        }

        if batch_count < BATCH_SIZE {
            break;
        }
    }

    db::set_state(diary, LAST_UPDATED_KEY, &cursor.to_string());
    Ok(stats)
}

#[derive(Default)]
struct RawSession {
    id: String,
    directory: String,
    title: String,
    time_created: i64,
    time_updated: i64,
    agent: Option<String>,
    model: Option<String>,
    cost: f64,
    tokens_input: i64,
    tokens_output: i64,
    tokens_reasoning: i64,
    additions: i64,
    deletions: i64,
    files_changed: i64,
    message_count: i64,
    is_subagent: bool,
}

fn row_to_raw(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawSession> {
    let get_str = |name: &str| -> Option<String> {
        r.get::<_, Option<String>>(name).ok().flatten()
    };
    let get_i64 = |name: &str, default: i64| -> i64 {
        r.get::<_, Option<i64>>(name)
            .ok()
            .flatten()
            .unwrap_or(default)
    };

    Ok(RawSession {
        id: get_str("id").unwrap_or_default(),
        directory: get_str("directory").unwrap_or_default(),
        title: get_str("title").unwrap_or_default(),
        time_created: get_i64("time_created", 0),
        time_updated: get_i64("time_updated", 0),
        agent: get_str("agent"),
        model: get_str("model"),
        cost: r
            .get::<_, Option<f64>>("cost")
            .ok()
            .flatten()
            .unwrap_or(0.0),
        tokens_input: get_i64("tokens_input", 0),
        tokens_output: get_i64("tokens_output", 0),
        tokens_reasoning: get_i64("tokens_reasoning", 0),
        additions: get_i64("summary_additions", 0),
        deletions: get_i64("summary_deletions", 0),
        files_changed: get_i64("summary_files", 0),
        message_count: get_i64("msg_count", 0),
        is_subagent: get_str("parent_id")
            .map(|p| !p.is_empty())
            .unwrap_or(false),
    })
}

fn is_noise(s: &RawSession) -> bool {
    s.id.is_empty()
        || s.directory.is_empty()
        || (s.tokens_input == 0 && s.tokens_output == 0 && s.message_count == 0)
}

/// Returns (project_name, project_path) resolving the session directory to its
/// git root when possible.
fn git_root_for(directory: &str) -> (String, String) {
    let dir = Path::new(directory);
    let root = db::resolve_git_root(dir);
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    (name, root.to_string_lossy().into_owned())
}
