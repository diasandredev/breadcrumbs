use breadcrumbs_lib::{db, opencode, reports};

#[test]
fn daily_report_smoke() {
    let dir = std::env::temp_dir().join("breadcrumbs-test-diary-report");
    let _ = std::fs::remove_dir_all(&dir);
    let conn = db::open_diary(&dir).unwrap();
    opencode::sync(&conn).unwrap();

    let (title, from, to) = {
        let now = chrono::Local::now();
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(now.timezone())
            .single()
            .unwrap();
        (
            format!("Daily Report — {}", now.format("%A, %b %d %Y")),
            start.timestamp_millis(),
            chrono::Utc::now().timestamp_millis(),
        )
    };

    let md = reports::build(&conn, from, to, &title).unwrap();
    println!("---REPORT---\n{md}---END---");
    assert!(md.contains("Daily Report"));
    assert!(md.contains("breadcrumbs"));
}
