use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub project_name: String,
    pub project_path: String,
    pub title: String,
    pub agent: Option<String>,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub tokens_reasoning: i64,
    pub cost: f64,
    pub additions: i64,
    pub deletions: i64,
    pub files_changed: i64,
    pub message_count: i64,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct SyncStats {
    pub new_sessions: u32,
    pub updated_sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub message: String,
    pub author: Option<String>,
    pub committed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWithCommits {
    #[serde(flatten)]
    pub session: Session,
    pub commits: Vec<Commit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineData {
    pub sessions: Vec<SessionWithCommits>,
    /// commits not matched to any session window
    pub standalone_commits: Vec<Commit>,
}
