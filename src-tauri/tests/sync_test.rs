use breadcrumbs_lib::{db, opencode};

#[test]
fn sync_real_opencode_db() {
    let dir = std::env::temp_dir().join("breadcrumbs-test-diary");
    let _ = std::fs::remove_dir_all(&dir);
    let conn = db::open_diary(&dir).expect("open diary");

    let stats = opencode::sync(&conn).expect("sync");
    println!("stats: {stats:?}");

    let mut stmt = conn
        .prepare(
            "SELECT p.name, s.title, s.model_id, s.agent, s.tokens_input, s.tokens_output,
                    s.message_count, s.started_at_ms, s.ended_at_ms
             FROM sessions s JOIN projects p ON p.id = s.project_id
             ORDER BY s.ended_at_ms DESC",
        )
        .unwrap();
    let rows: Vec<(String, String, Option<String>, Option<String>, i64, i64, i64, i64, i64)> = stmt
        .query_map([], |r| {
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
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for (proj, title, model, agent, tin, tout, msgs, t0, t1) in &rows {
        println!(
            "{proj} | {title} | {model:?} | {agent:?} | in={tin} out={tout} msgs={msgs} | {}s",
            (t1 - t0) / 1000
        );
    }

    // incremental second run must not duplicate anything
    let again = opencode::sync(&conn).unwrap();
    assert_eq!(again.new_sessions, 0);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count as usize, rows.len());
}
