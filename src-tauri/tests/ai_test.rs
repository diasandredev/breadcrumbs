use breadcrumbs_lib::ai;

#[test]
fn ollama_list_and_enhance() {
    let models = ai::list_models("http://localhost:11434").expect("list models");
    println!("models: {models:?}");
    assert!(!models.is_empty());

    // use a small local model for the pipeline test
    let model = "llama3.2:latest";
    if !models.iter().any(|m| m.starts_with("llama3.2")) {
        eprintln!("llama3.2 not installed; skipping enhance call");
        return;
    }

    let out = ai::enhance_report(
        "http://localhost:11434",
        model,
        "# Daily Report — Test\n\n## breadcrumbs\n_1 session, 0 commits, 5 tokens_\n\n- Fixed login bug (12m)\n",
    )
    .expect("enhance");
    println!("---ENHANCED---\n{out}\n---");
    assert!(!out.is_empty());
}
