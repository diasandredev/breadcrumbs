use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use rusqlite::Connection;
use serde_json::Value;

use crate::db;
use crate::models::SyncStats;

const LAST_SYNC_KEY: &str = "claude_last_sync_ms";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn claude_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude"))
}

/// Reads ~/.claude/history.jsonl to map sessionId -> display prompt / title
fn load_history_prompts(claude_base: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let history_file = claude_base.join("history.jsonl");
    if !history_file.exists() {
        return map;
    }

    if let Ok(file) = File::open(history_file) {
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if let (Some(sid), Some(display)) = (
                    v.get("sessionId").and_then(|s| s.as_str()),
                    v.get("display").and_then(|s| s.as_str()),
                ) {
                    let clean = display.trim();
                    if !clean.is_empty() && !clean.starts_with('/') {
                        // Keep the earliest non-command prompt as session title if not already present
                        map.entry(sid.to_string()).or_insert_with(|| clean.to_string());
                    }
                }
            }
        }
    }
    map
}

pub fn sync(diary: &Connection) -> Result<SyncStats, String> {
    let base = match claude_dir() {
        Some(p) if p.exists() => p,
        _ => {
            return Ok(SyncStats {
                new_sessions: 0,
                updated_sessions: 0,
            })
        }
    };

    let projects_dir = base.join("projects");
    if !projects_dir.exists() {
        return Ok(SyncStats {
            new_sessions: 0,
            updated_sessions: 0,
        });
    }

    let prompts = load_history_prompts(&base);
    let last_sync: i64 = db::get_state(diary, LAST_SYNC_KEY)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut stats = SyncStats {
        new_sessions: 0,
        updated_sessions: 0,
    };

    let mut max_seen_ms = last_sync;

    // Scan all *.jsonl files in subdirectories of ~/.claude/projects/
    let Ok(proj_entries) = std::fs::read_dir(&projects_dir) else {
        return Ok(stats);
    };

    for p_entry in proj_entries.flatten() {
        let p_path = p_entry.path();
        if !p_path.is_dir() {
            continue;
        }

        let Ok(files) = std::fs::read_dir(&p_path) else {
            continue;
        };

        for f_entry in files.flatten() {
            let f_path = f_entry.path();
            if f_path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }

            // Check file modification time to skip ancient files quickly
            if let Ok(meta) = f_path.metadata() {
                if let Ok(modified) = meta.modified() {
                    let mod_ms = modified
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    if mod_ms < last_sync && last_sync > 0 {
                        continue;
                    }
                }
            }

            if let Some((sess_id, cwd, title, model, tin, tout, treasons, msgs, t0, t1)) =
                parse_session_file(&f_path, &prompts)
            {
                if t1 > max_seen_ms {
                    max_seen_ms = t1;
                }

                let now = now_ms();
                let proj_name = Path::new(&cwd)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("project");

                let project_id = match db::upsert_project(diary, &cwd, proj_name, now) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                let exists: bool = diary
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM sessions WHERE id = ?1",
                        [&sess_id],
                        |r| r.get(0),
                    )
                    .unwrap_or(false);

                let res = diary.execute(
                    "INSERT INTO sessions(
                        id, project_id, title, agent, model_provider, model_id,
                        tokens_input, tokens_output, tokens_reasoning, cost,
                        additions, deletions, files_changed, message_count,
                        started_at_ms, ended_at_ms, synced_at_ms
                    ) VALUES(?1, ?2, ?3, 'claude-code', 'anthropic', ?4, ?5, ?6, ?7, 0, 0, 0, 0, ?8, ?9, ?10, ?11)
                    ON CONFLICT(id) DO UPDATE SET
                        title = excluded.title,
                        model_id = excluded.model_id,
                        tokens_input = excluded.tokens_input,
                        tokens_output = excluded.tokens_output,
                        tokens_reasoning = excluded.tokens_reasoning,
                        message_count = excluded.message_count,
                        started_at_ms = excluded.started_at_ms,
                        ended_at_ms = excluded.ended_at_ms,
                        synced_at_ms = excluded.synced_at_ms",
                    rusqlite::params![
                        sess_id,
                        project_id,
                        title,
                        model,
                        tin,
                        tout,
                        treasons,
                        msgs,
                        t0,
                        t1,
                        now
                    ],
                );

                if res.is_ok() {
                    if exists {
                        stats.updated_sessions += 1;
                    } else {
                        stats.new_sessions += 1;
                    }
                }
            }
        }
    }

    db::set_state(diary, LAST_SYNC_KEY, &max_seen_ms.to_string());
    Ok(stats)
}

#[allow(clippy::type_complexity)]
fn parse_session_file(
    path: &Path,
    prompts: &HashMap<String, String>,
) -> Option<(
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
)> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let session_id_from_file = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let mut detected_session_id = session_id_from_file.clone();
    let mut cwd = String::new();
    let mut model = String::from("claude");
    let mut tokens_in: i64 = 0;
    let mut tokens_out: i64 = 0;
    let mut tokens_reasoning: i64 = 0;
    let mut message_count: i64 = 0;
    let mut first_user_prompt = String::new();

    let mut t_min: Option<i64> = None;
    let mut t_max: Option<i64> = None;

    for line in reader.lines().map_while(Result::ok) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|s| s.as_str()) {
                cwd = c.to_string();
            }
        }

        if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
            detected_session_id = sid.to_string();
        }

        // Parse timestamps
        if let Some(ts_str) = v.get("timestamp").and_then(|s| s.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts_str) {
                let ms = dt.timestamp_millis();
                t_min = Some(t_min.map_or(ms, |m| m.min(ms)));
                t_max = Some(t_max.map_or(ms, |m| m.max(ms)));
            }
        }

        let typ = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
        if typ == "user" {
            message_count += 1;
            if first_user_prompt.is_empty() {
                if let Some(msg) = v.get("message").and_then(|s| s.as_str()) {
                    let clean = msg.trim();
                    if !clean.is_empty() && !clean.starts_with('/') {
                        first_user_prompt = clean.chars().take(80).collect();
                    }
                }
            }
        } else if typ == "assistant" {
            message_count += 1;
            if let Some(msg) = v.get("message") {
                if let Some(m) = msg.get("model").and_then(|s| s.as_str()) {
                    model = m.to_string();
                }
                if let Some(u) = msg.get("usage") {
                    tokens_in += u.get("input_tokens").and_then(|n| n.as_i64()).unwrap_or(0);
                    tokens_out += u.get("output_tokens").and_then(|n| n.as_i64()).unwrap_or(0);
                    tokens_reasoning += u
                        .get("cache_creation_input_tokens")
                        .and_then(|n| n.as_i64())
                        .unwrap_or(0);
                }
            }
        }
    }

    let t0 = t_min.unwrap_or_else(now_ms);
    let t1 = t_max.unwrap_or(t0);

    // If cwd wasn't found in events, infer from parent dir name if possible
    if cwd.is_empty() {
        if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
            cwd = parent.replace("--", "/").replace('-', "/");
        }
    }

    if cwd.is_empty() {
        cwd = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
    }

    // Determine clean title:
    // 1. History prompt from history.jsonl
    // 2. First user prompt from transcript
    // 3. Fallback to session ID snippet
    let title = prompts
        .get(&detected_session_id)
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !first_user_prompt.is_empty() {
                Some(first_user_prompt)
            } else {
                None
            }
        })
        .unwrap_or_else(|| format!("Claude session {}", &detected_session_id.chars().take(8).collect::<String>()));

    Some((
        detected_session_id,
        cwd,
        title,
        model,
        tokens_in,
        tokens_out,
        tokens_reasoning,
        message_count,
        t0,
        t1,
    ))
}
