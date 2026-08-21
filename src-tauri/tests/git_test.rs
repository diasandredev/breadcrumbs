use breadcrumbs_lib::{db, git};

#[test]
fn git_sync_with_real_repo() {
    // disposable fixture repo
    let repo = std::env::temp_dir().join("breadcrumbs-fixture-repo");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &[
        "-c", "user.name=Fixture",
        "-c", "user.email=fixture@test",
        "commit", "-q", "-m", "initial commit",
    ]);
    std::fs::write(repo.join("b.txt"), "world").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &[
        "-c", "user.name=Fixture",
        "-c", "user.email=fixture@test",
        "commit", "-q", "-m", "add world file",
    ]);

    let dir = std::env::temp_dir().join("breadcrumbs-test-diary-git2");
    let _ = std::fs::remove_dir_all(&dir);
    let conn = db::open_diary(&dir).unwrap();
    db::upsert_project(&conn, &repo.to_string_lossy(), "fixture-repo", 0).unwrap();

    let n = git::sync_projects(&conn).unwrap();
    println!("new commits: {n}");
    assert_eq!(n, 2);

    let n2 = git::sync_projects(&conn).unwrap();
    assert_eq!(n2, 0, "second run must be idempotent");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM commits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);

    let _ = std::fs::remove_dir_all(&repo);
}

fn run(repo: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?} failed");
}
