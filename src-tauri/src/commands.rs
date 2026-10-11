use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use rusqlite::Connection;
use tauri::{Emitter, Manager, State};

use crate::ai::{self, AiSettings};
use crate::claude;
use crate::db;
use crate::git;
use crate::models::{Commit, Session, SessionWithCommits, SyncStats, TimelineData};
use crate::opencode;
use crate::reports;

pub struct AppState(pub Mutex<Connection>);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn day_string(ms: i64) -> String {
    let dt = DateTime::from_timestamp_millis(ms)
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());
    dt.format("%Y-%m-%d").to_string()
}

#[tauri::command]
pub fn sync_now(state: State<'_, AppState>) -> Result<SyncStats, String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    let mut stats = opencode::sync(&conn).unwrap_or(SyncStats {
        new_sessions: 0,
        updated_sessions: 0,
    });
    if let Ok(c_stats) = claude::sync(&conn) {
        stats.new_sessions += c_stats.new_sessions;
        stats.updated_sessions += c_stats.updated_sessions;
    }
    let _ = git::discover_repos(&conn);
    let _ = git::sync_projects(&conn);
    Ok(stats)
}

fn today_title_and_range() -> (String, i64, i64) {
    let now = chrono::Local::now();
    let start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(now.timezone())
        .single()
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);
    let weekday = now.format("%A, %b %d %Y").to_string();
    (format!("Daily Report — {weekday}"), start, now_ms())
}

/// Tray menu action: build today's report and copy it to the clipboard.
pub fn copy_today_report(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let state: tauri::State<AppState> = app.state();
            let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
            let (title, from, to) = today_title_and_range();
            let md = reports::build(&conn, from, to, &title)?;
            use tauri_plugin_clipboard_manager::ClipboardExt;
            app.clipboard().write_text(md).map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => app.emit("report-copied", true).ok(),
            Err(e) => app.emit("sync-error", e).ok(),
        };
    });
}

#[tauri::command]
pub fn generate_report(
    from_ms: i64,
    to_ms: i64,
    title: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    reports::build(&conn, from_ms, to_ms, &title)
}

// ---------- AI / settings ----------

fn load_ai_settings(conn: &Connection) -> AiSettings {
    let mut s = AiSettings::default();
    if db::get_state(conn, "ai_enabled").as_deref() == Some("1") {
        s.enabled = true;
    }
    if let Some(p) = db::get_state(conn, "ai_provider") {
        s.provider = p;
    }
    if let Some(u) = db::get_state(conn, "ai_url") {
        s.url = u;
    } else if let Some(u) = db::get_state(conn, "ollama_url") {
        s.url = u;
    }
    if let Some(k) = db::get_state(conn, "ai_api_key") {
        s.api_key = Some(k);
    }
    if let Some(m) = db::get_state(conn, "ai_model") {
        s.model = m;
    } else if let Some(m) = db::get_state(conn, "ollama_model") {
        s.model = m;
    }
    if let Some(r) = db::get_state(conn, "ai_reasoning") {
        s.reasoning = Some(r);
    }
    s
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AiSettings, String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    Ok(load_ai_settings(&conn))
}

#[tauri::command]
pub fn set_ai_settings(
    settings: AiSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    db::set_state(
        &conn,
        "ai_enabled",
        if settings.enabled { "1" } else { "0" },
    );
    db::set_state(&conn, "ai_provider", settings.provider.trim());
    db::set_state(&conn, "ai_url", settings.url.trim());
    db::set_state(&conn, "ollama_url", settings.url.trim());
    if let Some(k) = &settings.api_key {
        db::set_state(&conn, "ai_api_key", k.trim());
    }
    db::set_state(&conn, "ai_model", settings.model.trim());
    db::set_state(&conn, "ollama_model", settings.model.trim());
    if let Some(r) = &settings.reasoning {
        db::set_state(&conn, "ai_reasoning", r.trim());
    }
    Ok(())
}

#[tauri::command]
pub fn list_ai_models(settings: AiSettings) -> Result<Vec<String>, String> {
    ai::list_models(&settings)
}

#[tauri::command]
pub fn list_ollama_models(url: String) -> Result<Vec<String>, String> {
    let s = AiSettings {
        enabled: true,
        provider: "ollama".into(),
        url,
        api_key: None,
        model: String::new(),
        reasoning: None,
    };
    ai::list_models(&s)
}

#[tauri::command]
pub fn enhance_report(
    from_ms: i64,
    to_ms: i64,
    title: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    let cfg = load_ai_settings(&conn);
    if !cfg.enabled {
        return Err("AI summaries are disabled in Settings".into());
    }
    let base = reports::build(&conn, from_ms, to_ms, &title)?;
    ai::enhance_report(&cfg, &base)
}

#[tauri::command]
pub fn set_note(
    session_id: String,
    note: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    conn.execute(
        "UPDATE sessions SET note = ?1 WHERE id = ?2",
        rusqlite::params![note.trim(), session_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_timeline(
    days: u32,
    state: State<'_, AppState>,
) -> Result<TimelineData, String> {
    let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
    let since_ms = now_ms() - (days as i64) * 86_400_000;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, p.id, p.name, p.path, s.title, s.agent, s.model_provider, s.model_id,
                    s.tokens_input, s.tokens_output, s.tokens_reasoning, s.cost,
                    s.additions, s.deletions, s.files_changed, s.message_count,
                    s.started_at_ms, s.ended_at_ms, s.note
             FROM sessions s JOIN projects p ON p.id = s.project_id
             WHERE s.ended_at_ms >= ?1
             ORDER BY s.ended_at_ms DESC",
        )
        .map_err(|e| e.to_string())?;

    const WINDOW_MS: i64 = 2 * 60_000;
    let mut matched: HashSet<String> = HashSet::new();
    let mut sessions = Vec::new();

    let rows = stmt
        .query_map([since_ms], |r| {
            let t0: i64 = r.get(16)?;
            let t1: i64 = r.get(17)?;
            let started_date = day_string(t0);
            let ended_date = day_string(t1);
            let is_multiday = started_date != ended_date;
            let day_span = ((t1 - t0).max(0) / 86_400_000) + 1;
            let note_str: String = r.get::<_, Option<String>>(18)?.unwrap_or_default();
            let note = if note_str.is_empty() { None } else { Some(note_str) };

            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                Session {
                    id: r.get(0)?,
                    project_name: r.get(2)?,
                    project_path: r.get(3)?,
                    title: r.get(4)?,
                    agent: r.get(5)?,
                    model_provider: r.get(6)?,
                    model_id: r.get(7)?,
                    tokens_input: r.get(8)?,
                    tokens_output: r.get(9)?,
                    tokens_reasoning: r.get(10)?,
                    cost: r.get(11)?,
                    additions: r.get(12)?,
                    deletions: r.get(13)?,
                    files_changed: r.get(14)?,
                    message_count: r.get(15)?,
                    started_at_ms: t0,
                    ended_at_ms: t1,
                    is_multiday,
                    day_span,
                    started_date,
                    ended_date,
                    note,
                },
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows.filter_map(|r| r.ok()) {
        let (project_id, _, session) = row;
        let commits = git::in_window(
            &conn,
            &project_id,
            session.started_at_ms - WINDOW_MS,
            session.ended_at_ms + WINDOW_MS,
        )
        .unwrap_or_default();
        for c in &commits {
            matched.insert(c.hash.clone());
        }
        sessions.push(SessionWithCommits { session, commits });
    }

    // standalone commits in the whole window
    let mut stmt_c = conn
        .prepare(
            "SELECT c.hash, c.message, c.author, c.committed_at_ms, p.name
             FROM commits c JOIN projects p ON p.id = c.project_id
             WHERE c.committed_at_ms >= ?1
             ORDER BY c.committed_at_ms DESC",
        )
        .map_err(|e| e.to_string())?;
    let standalone = stmt_c
        .query_map([since_ms], |r| {
            Ok(Commit {
                hash: r.get(0)?,
                message: r.get(1)?,
                author: r.get(2)?,
                committed_at_ms: r.get(3)?,
                project_name: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|c| !matched.contains(&c.hash))
        .collect();

    Ok(TimelineData {
        sessions,
        standalone_commits: standalone,
    })
}

/// Runs a background sync (OpenCode + Claude Code + git commits) and notifies the
/// panel when done.
pub fn spawn_sync(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let result = (|| -> Result<SyncStats, String> {
            let state: tauri::State<AppState> = app.state();
            let conn = state.0.lock().map_err(|_| "db lock poisoned".to_string())?;
            let mut stats = opencode::sync(&conn).unwrap_or(SyncStats {
                new_sessions: 0,
                updated_sessions: 0,
            });
            if let Ok(c_stats) = claude::sync(&conn) {
                stats.new_sessions += c_stats.new_sessions;
                stats.updated_sessions += c_stats.updated_sessions;
            }
            match git::discover_repos(&conn) {
                Ok(n) if n > 0 => eprintln!("repo discovery: +{n} new repos"),
                Err(e) => eprintln!("repo discovery failed: {e}"),
                _ => {}
            }
            match git::sync_projects(&conn) {
                Ok(n) => eprintln!("git sync: +{n} commits"),
                Err(e) => eprintln!("git sync failed: {e}"),
            }
            Ok(stats)
        })();
        match result {
            Ok(stats) => app.emit("sync-done", &stats).ok(),
            Err(e) => app.emit("sync-error", e).ok(),
        };
    });
}
