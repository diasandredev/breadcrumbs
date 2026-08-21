use std::path::{Path, PathBuf};

use rusqlite::Connection;

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    first_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL,
    agent TEXT,
    model_provider TEXT,
    model_id TEXT,
    tokens_input INTEGER NOT NULL DEFAULT 0,
    tokens_output INTEGER NOT NULL DEFAULT 0,
    tokens_reasoning INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    files_changed INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL,
    synced_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_ended_idx ON sessions(ended_at_ms);

CREATE TABLE IF NOT EXISTS commits (
    hash TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    message TEXT NOT NULL,
    author TEXT,
    committed_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS commits_time_idx ON commits(committed_at_ms);

CREATE TABLE IF NOT EXISTS sync_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub fn open_diary(app_data_dir: &Path) -> Result<Connection, rusqlite::Error> {
    std::fs::create_dir_all(app_data_dir).ok();
    let conn = Connection::open(app_data_dir.join("diary.db"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(MIGRATIONS)?;
    Ok(conn)
}

pub fn get_state(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM sync_state WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn set_state(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO sync_state(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .ok();
}

pub fn upsert_project(
    conn: &Connection,
    path: &str,
    name: &str,
    now_ms: i64,
) -> Result<String, rusqlite::Error> {
    let id = hash_path(path);
    conn.execute(
        "INSERT INTO projects(id, path, name, first_seen_at) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO NOTHING",
        rusqlite::params![id, path, name, now_ms],
    )?;
    // ensure id matches existing row if it was already there under same path
    let stored: String = conn.query_row(
        "SELECT id FROM projects WHERE path = ?1",
        [path],
        |r| r.get(0),
    )?;
    Ok(stored)
}

fn hash_path(path: &str) -> String {
    // stable simple hash (FNV-1a 64) as hex; collisions unlikely for a handful of repos
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("prj_{h:016x}")
}

pub fn resolve_git_root(from: &Path) -> PathBuf {
    let mut cur = from.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return cur;
        }
        if !cur.pop() {
            return from.to_path_buf();
        }
    }
}
