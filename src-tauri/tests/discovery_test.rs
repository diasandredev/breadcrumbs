use breadcrumbs_lib::{db, git};

#[test]
fn discovers_real_repos_and_syncs_commits() {
    let dir = std::env::temp_dir().join("breadcrumbs-test-discovery");
    let _ = std::fs::remove_dir_all(&dir);
    let conn = db::open_diary(&dir).unwrap();

    let n = git::discover_repos(&conn).unwrap();
    println!("new repos discovered: {n}");

    let names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM projects ORDER BY name").unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    println!("projects: {names:?}");
    assert!(names.contains(&"breadcrumbs".to_string()));

    // idempotent
    assert_eq!(git::discover_repos(&conn).unwrap(), 0);

    let total = git::sync_projects(&conn).unwrap();
    println!("commits synced: {total}");

    let recent: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT p.name, c.message FROM commits c
                 JOIN projects p ON p.id = c.project_id
                 WHERE c.committed_at_ms >= (strftime('%s','now','-7 days')*1000)
                 ORDER BY c.committed_at_ms DESC",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    println!("commits from last 7 days:");
    for (p, m) in &recent {
        println!("  [{p}] {m}");
    }
}
