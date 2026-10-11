use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String, // "gateway" | "ollama"
    pub url: String,
    pub auth_mode: String, // "token_file" | "api_key"
    pub token_file: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
    pub reasoning: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "gateway".into(),
            url: "https://generative-ai-platform-development.ifoodcorp.com.br/api/v2".into(),
            auth_mode: "token_file".into(),
            token_file: Some("~/.config/tompero/requester_token".into()),
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

fn expand_home_path(raw: &str) -> PathBuf {
    if raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&raw[2..]);
        }
    }
    PathBuf::from(raw)
}

/// Reads the authentication token fresh on every request without keeping a long-lived memory cache.
/// Supports dynamic rotation from local token daemons (e.g. ~/.config/tompero/requester_token).
pub fn resolve_token(settings: &AiSettings) -> Result<Option<String>, String> {
    if settings.auth_mode == "token_file" {
        let raw_path = settings
            .token_file
            .as_deref()
            .unwrap_or("~/.config/tompero/requester_token");
        let path = expand_home_path(raw_path);
        if !path.exists() {
            return Err(format!(
                "Token file not found at {}. Ensure the token service (tompero) is running.",
                path.display()
            ));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read token file ({}): {e}", path.display()))?;
        let clean = content.trim().to_string();
        if clean.is_empty() {
            return Err(format!("Token file at {} is empty.", path.display()));
        }
        Ok(Some(clean))
    } else {
        Ok(settings
            .api_key
            .as_ref()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty()))
    }
}

/// Formats the chat completions endpoint path cleanly for standard gateways and /api/v2
fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

/// Formats the model discovery endpoint path (GenPlat /api/v2/model/info or standard /models)
fn model_info_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/model/info") {
        base.to_string()
    } else {
        format!("{base}/model/info")
    }
}

/// Attaches both Authorization Bearer and x-requester-token along with platform user-agent
fn apply_headers(mut req: ureq::Request, token: Option<&str>) -> ureq::Request {
    if let Some(t) = token {
        if !t.is_empty() {
            // OpenAI-compatible surfaces (chat-completions, responses)
            req = req.set("Authorization", &format!("Bearer {t}"));
            // Anthropic surface and /model/info header requirement
            req = req.set("x-requester-token", t);
        }
    }
    // Tracing user agent
    req = req.set("x-genplat-user-agent", "breadcrumbs");
    req = req.set("User-Agent", "breadcrumbs/2.0");
    req
}

/// Lists available models: queries /model/info first (GenPlat), then fallbacks to /models (OpenAI) or /api/tags (Ollama).
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
        return Ok(names);
    }

    let token = resolve_token(settings)?;

    // 1. Try GenPlat / Gateway /model/info endpoint
    let info_endpoint = model_info_url(url);
    let mut req = client().get(&info_endpoint);
    req = apply_headers(req, token.as_deref());

    if let Ok(res) = req.call() {
        if let Ok(val) = res.into_json::<serde_json::Value>() {
            let mut list = Vec::new();

            let items = if let Some(arr) = val.as_array() {
                Some(arr)
            } else if let Some(arr) = val.get("data").and_then(|d| d.as_array()) {
                Some(arr)
            } else if let Some(arr) = val.get("models").and_then(|m| m.as_array()) {
                Some(arr)
            } else {
                None
            };

            if let Some(arr) = items {
                for item in arr {
                    // Filter: only mode == "chat" or mode == "responses" as specified
                    if let Some(mode) = item.get("mode").and_then(|m| m.as_str()) {
                        if mode != "chat" && mode != "responses" {
                            continue;
                        }
                    }

                    if let Some(name) = item.get("name").or_else(|| item.get("id")).and_then(|s| s.as_str()) {
                        // Filter out deprecated models or internal aliases with "/"
                        if name.ends_with("-deprecated") || name.contains('/') {
                            continue;
                        }
                        list.push(name.to_string());
                    }
                }
            }

            if !list.is_empty() {
                list.sort();
                list.dedup();
                return Ok(list);
            }
        }
    }

    // 2. Fallback to standard OpenAI /models endpoint
    let openai_endpoint = if url.ends_with('/') {
        format!("{url}models")
    } else {
        format!("{url}/models")
    };
    let mut req2 = client().get(&openai_endpoint);
    req2 = apply_headers(req2, token.as_deref());

    if let Ok(res) = req2.call() {
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
                let mut models: Vec<String> = data
                    .into_iter()
                    .map(|m| m.id)
                    .filter(|id| !id.ends_with("-deprecated"))
                    .collect();
                models.sort();
                models.dedup();
                return Ok(models);
            }
        }
    }

    Ok(vec![])
}

pub const SYSTEM_PROMPT: &str = "You are a precise developer assistant that writes polished development diary reports.\n\nRules:\n- Keep every fact, number, timing, token count and commit message accurate.\n- Do not invent work that is not in the source data.\n- Group work clearly by project and highlight active vs multi-day sessions.\n- Output clean Markdown only, no meta-commentary.";

pub const DISTILL_PROMPT: &str = "You are given the log of one developer AI coding session (project, user request, touched files and assistant notes). Reply with EXACTLY two lines:\nTitle: <a specific 4-8 word title in past tense; no trailing period>\nSummary: <1-2 sentences, past tense, naming concrete changes and purpose>";

/// Dispatches a prompt to either the Gateway or local Ollama
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
        let token = resolve_token(settings)?;
        let endpoint = chat_completions_url(&settings.url);

        let mut req = client().post(&endpoint);
        req = apply_headers(req, token.as_deref());

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
