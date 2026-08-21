use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub url: String,
    pub model: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "http://localhost:11434".into(),
            model: String::new(),
        }
    }
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagsModel>,
}

#[derive(Deserialize)]
struct TagsModel {
    name: String,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

fn client() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(180))
        .timeout_write(Duration::from_secs(30))
        .build()
}

/// Lists installed Ollama models. Errors surface as strings for the UI.
pub fn list_models(url: &str) -> Result<Vec<String>, String> {
    let tags: TagsResponse = client()
        .get(&format!("{url}/api/tags"))
        .call()
        .map_err(|e| format!("Ollama unreachable at {url}: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let mut names: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
    names.sort();
    Ok(names)
}

const SYSTEM_PROMPT: &str = "You are a precise assistant that polishes software development reports. You receive a structured markdown report of coding sessions and git commits and rewrite it as a clean, professional report.\n\nRules:\n- Keep every fact, number, timing, token count and commit message exactly as provided.\n- Do not invent anything that is not in the source data.\n- Improve only the prose: short intro paragraph, then keep the structured sections/bullets.\n- Output markdown only, no commentary.";

/// Sends the deterministic report to Ollama and returns a narrative version.
pub fn enhance_report(url: &str, model: &str, report_markdown: &str) -> Result<String, String> {
    if model.is_empty() {
        return Err("No Ollama model configured".into());
    }
    let req = ChatRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: SYSTEM_PROMPT.into(),
            },
            ChatMessage {
                role: "user",
                content: format!(
                    "Polish this development report:\n\n{report_markdown}"
                ),
            },
        ],
        stream: false,
    };

    let resp: ChatResponse = client()
        .post(&format!("{url}/api/chat"))
        .send_json(req)
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                format!("Ollama returned {code}: {body}")
            }
            other => format!("Ollama request failed: {other}"),
        })?
        .into_json()
        .map_err(|e| e.to_string())?;

    Ok(resp.message.content.trim().to_string())
}
