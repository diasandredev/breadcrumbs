use rusqlite::Connection;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct ReportRow {
    pub project: String,
    pub title: String,
    pub agent: Option<String>,
    pub model_id: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub message_count: i64,
    pub additions: i64,
    pub deletions: i64,
    pub files_changed: i64,
    pub duration_ms: i64,
    pub commits: Vec<String>,
}

/// Builds a deterministic markdown report for [from_ms, to_ms].
pub fn build(conn: &Connection, from_ms: i64, to_ms: i64, title: &str) -> Result<String, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, p.name, s.title, s.agent, s.model_id,
                    s.tokens_input, s.tokens_output, s.message_count,
                    s.additions, s.deletions, s.files_changed,
                    s.started_at_ms, s.ended_at_ms
             FROM sessions s JOIN projects p ON p.id = s.project_id
             WHERE s.ended_at_ms >= ?1 AND s.started_at_ms <= ?2
             ORDER BY p.name ASC, s.started_at_ms ASC",
        )
        .map_err(|e| e.to_string())?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    )> = stmt
        .query_map([from_ms, to_ms], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // attach commits matched by time window per project
    let mut proj_ids: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, name FROM projects")
            .map_err(|e| e.to_string())?;
        let it = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        for x in it.filter_map(|x| x.ok()) {
            proj_ids.push(x);
        }
    }

    const WINDOW_MS: i64 = 2 * 60_000;
    let mut items: Vec<ReportRow> = Vec::new();
    for (id, project, stitle, agent, model_id, tin, tout, msgs, adds, dels, files_changed, t0, t1) in &rows {
        let commits = commit_messages(conn, &proj_ids, project, t0 - WINDOW_MS, t1 + WINDOW_MS)?;
                items.push(ReportRow {
                    project: project.clone(),
                    title: stitle.clone(),
                    agent: agent.clone(),
                    model_id: model_id.clone(),
                    tokens_in: *tin,
                    tokens_out: *tout,
                    message_count: *msgs,
                    additions: *adds,
                    deletions: *dels,
                    files_changed: *files_changed,
                    duration_ms: t1 - t0,
                    commits,
                });
        let _ = id;
    }

    // unmatched standalone commits inside the range
    let mut stmt = conn
        .prepare(
            "SELECT p.name, c.message FROM commits c JOIN projects p ON p.id = c.project_id
             WHERE c.committed_at_ms >= ?1 AND c.committed_at_ms <= ?2",
        )
        .map_err(|e| e.to_string())?;
    let all_commits: Vec<(String, String)> = stmt
        .query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let used: HashSet<String> = items
        .iter()
        .flat_map(|i| i.commits.iter().cloned())
        .collect();
    for (project, msg) in &all_commits {
        if !used.contains(msg) {
            if let Some(item) = items.iter_mut().find(|i| &i.project == project) {
                item.commits.push(msg.clone());
            } else {
                items.push(ReportRow {
                    project: project.clone(),
                    title: String::new(),
                    agent: None,
                    model_id: None,
                    tokens_in: 0,
                    tokens_out: 0,
                    message_count: 0,
                    additions: 0,
                    deletions: 0,
                    files_changed: 0,
                    duration_ms: 0,
                    commits: vec![msg.clone()],
                });
            }
        }
    }

    render(title, &items)
}

fn commit_messages(
    conn: &Connection,
    proj_ids: &[(String, String)],
    project_name: &str,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<String>, String> {
    let Some((pid, _)) = proj_ids.iter().find(|(_, n)| n == project_name) else {
        return Ok(vec![]);
    };
    let mut stmt = conn
        .prepare(
            "SELECT message FROM commits
             WHERE project_id = ?1 AND committed_at_ms BETWEEN ?2 AND ?3
             ORDER BY committed_at_ms ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![pid, from_ms, to_ms], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

use std::collections::HashSet;

fn fmt_dur(ms: i64) -> String {
    let m = (ms / 60_000).max(0) as usize;
    if m < 60 {
        format!("{m}m")
    } else {
        format!("{}h{:02}m", m / 60, m % 60)
    }
}

fn fmt_tok(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn render(title: &str, items: &[ReportRow]) -> Result<String, String> {
    if items.is_empty() {
        return Ok(format!("# {title}\n\n_No activity recorded._\n"));
    }

    let mut out = String::new();
    out.push_str(&format!("# {title}\n"));

    let mut by_project: std::collections::BTreeMap<&str, Vec<&ReportRow>> =
        std::collections::BTreeMap::new();
    for i in items {
        by_project.entry(i.project.as_str()).or_default().push(i);
    }

    let mut total_sessions = 0usize;
    let mut total_commits = 0usize;
    let mut total_tokens = 0i64;

    for (project, entries) in &by_project {
        out.push_str(&format!("\n## {project}\n"));
        let commits_count: usize = entries.iter().map(|e| e.commits.len()).sum();
        let tokens: i64 = entries.iter().map(|e| e.tokens_in + e.tokens_out).sum();
        out.push_str(&format!(
            "_{} session{}, {} commit{}, {} tokens_\n\n",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" },
            commits_count,
            if commits_count == 1 { "" } else { "s" },
            fmt_tok(tokens),
        ));

        for e in entries {
            if !e.title.is_empty() {
                let mut line = format!("- {}", e.title);
                if e.duration_ms > 0 {
                    line.push_str(&format!(" ({})", fmt_dur(e.duration_ms)));
                }
                if e.additions + e.deletions > 0 {
                    line.push_str(&format!(
                        ", +{}/−{} lines across {} files",
                        e.additions, e.deletions, e.files_changed
                    ));
                }
                out.push_str(&line);
                out.push('\n');
                total_sessions += 1;
                total_tokens += e.tokens_in + e.tokens_out;
            }
            for c in &e.commits {
                out.push_str(&format!("  - {c}\n"));
                total_commits += 1;
            }
        }
    }

    out.push_str(&format!(
        "\n---\n**Totals:** {total_sessions} session{}, {total_commits} commit{}, {} tokens\n",
        if total_sessions == 1 { "" } else { "s" },
        if total_commits == 1 { "" } else { "s" },
        fmt_tok(total_tokens),
    ));
    Ok(out)
}

#[allow(dead_code)]
pub fn today_range_ms() -> (i64, i64) {
    let now = chrono::Local::now();
    let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let start_ms = start.and_local_timezone(now.timezone()).single().map_or(0, |t| {
        t.timestamp_millis()
    });
    (start_ms, now_ms())
}
