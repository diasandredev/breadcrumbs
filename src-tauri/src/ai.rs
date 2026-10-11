use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String, // "gateway" | "ollama"
    pub url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub reasoning: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "gateway".into(),
            url: "http://localhost:11434".into(),
            api_key: None,
            model: String::new(),
            reasoning: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

fn client() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(180))
        .timeout_write(Duration::from_secs(30))
        .build()
}

/// Lists installed models if Ollama, or validates gateway connectivity.
pub fn list_models(settings: &AiSettings) -> Result<Vec<String>, String> {
    let url = &settings.url;
    if settings.provider == "ollama" {
        let tags: TagsResponse = client()
            .get(&format!("{url}/api/tags"))
            .call()
            .map_err(|e| format!("Ollama unreachable at {url}: {e}"))?
            .into_json()
            .map_err(|e| e.to_string())?;
        let mut names: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
        names.sort();
        Ok(names)
    } else {
        // OpenAI-compatible /models endpoint
        let endpoint = if url.ends_with("/v1") {
            format!("{url}/models")
        } else if url.ends_with('/') {
            format!("{url}v1/models")
        } else {
            format!("{url}/v1/models")
        };

        let mut req = client().get(&endpoint);
        if let Some(key) = &settings.api_key {
            if !key.trim().is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", key.trim()));
            }
        }

        match req.call() {
            Ok(res) => {
                #[derive(Deserialize)]
                struct ModelItem {
                    id: String,
                }
                #[derive(Deserialize)]
                struct ModelListResponse {
                    data: Option<Vec<ModelItem>>,
                }
                if let Ok(json) = res.into_json::<ModelListResponse>() {
                    if let Some(data) = json.data {
                        let mut models: Vec<String> = data.into_iter().map(|m| m.id).collect();
                        models.sort();
                        return Ok(models);
                    }
                }
                Ok(vec![])
            }
            Err(_) => {
                // If /models is restricted or unavailable on custom gateway, fallback gracefully
                Ok(vec![])
            }
        }
    }
}

pub const SYSTEM_PROMPT: &str = "You are a precise developer assistant that writes polished development diary reports.\n\nRules:\n- Keep every fact, number, timing, token count and commit message accurate.\n- Do not invent work that is not in the source data.\n- Group work clearly by project and highlight active vs multi-day sessions.\n- Output clean Markdown only, no meta-commentary.";

pub const DISTILL_PROMPT: &str = "You are given the log of one developer AI coding session (project, user request, touched files and assistant notes). Reply with EXACTLY two lines:\nTitle: <a specific 4-8 word title in past tense; no trailing period>\nSummary: <1-2 sentences, past tense, naming concrete changes and purpose>";

/// Dispatches a prompt to either the OpenAI-compatible Gateway or local Ollama
pub fn generate_completion(
    settings: &AiSettings,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    if settings.model.trim().is_empty() {
        return Err("No AI model configured. Check Settings.".into());
    }

    let reasoning_val = settings
        .reasoning
        .as_deref()
        .filter(|r| !r.is_empty() && *r != "none")
        .map(String::from);

    let chat_req = ChatRequest {
        model: settings.model.trim().to_string(),
        messages: vec![
            ChatMessage {
                role: "system",
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user",
                content: user_prompt.to_string(),
            },
        ],
        stream: false,
        reasoning_effort: reasoning_val,
    };

    if settings.provider == "ollama" {
        let endpoint = format!("{}/api/chat", settings.url.trim_end_matches('/'));
        let resp: OllamaChatResponse = client()
            .post(&endpoint)
            .send_json(chat_req)
            .map_err(|e| format!("Ollama request failed: {e}"))?
            .into_json()
            .map_err(|e| e.to_string())?;
        Ok(resp.message.content.trim().to_string())
    } else {
        // Gateway / OpenAI-compatible endpoint
        let base = settings.url.trim_end_matches('/');
        let endpoint = if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        };

        let mut req = client().post(&endpoint);
        if let Some(key) = &settings.api_key {
            let k = key.trim();
            if !k.is_empty() {
                req = req.set("Authorization", &format!("Bearer {k}"));
            }
        }

        let resp: OpenAiChatResponse = req
            .send_json(chat_req)
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => {
                    let body = r.into_string().unwrap_or_default();
                    format!("AI Gateway returned HTTP {code}: {body}")
                }
                other => format!("AI Gateway request failed: {other}"),
            })?
            .into_json()
            .map_err(|e| format!("Failed to parse response JSON: {e}"))?;

        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
    }
}

/// Enhances a structured report into narrative markdown
pub fn enhance_report(
    settings: &AiSettings,
    report_markdown: &str,
) -> Result<String, String> {
    generate_completion(
        settings,
        SYSTEM_PROMPT,
        &format!("Polish this development report:\n\n{report_markdown}"),
    )
}

/// Distills one session into a concise title and summary
pub fn distill_session(
    settings: &AiSettings,
    session_text: &str,
) -> Result<String, String> {
    generate_completion(
        settings,
        DISTILL_PROMPT,
        session_text,
    )
}
