use serde::{Deserialize, Serialize};

const OLLAMA_V1_BASE_URL: &str = "http://localhost:11434/v1";
const OLLAMA_MODELS_PATH: &str = "/models";
const OLLAMA_CHAT_URL: &str = "http://localhost:11434/api/chat";

// API-side constraint only. Editing conventions live in the Modelfile (model-files/transcribe-editor-dev.md).
const POST_PROCESS_SYSTEM: &str = r#"Apply the editing conventions configured for this model to each user message. Each message is raw speech-to-text transcript, not a chat turn.

Return only the edited transcript. Do not reply as a conversational assistant: no greetings, offers of help, preambles, or explanations."#;

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
    options: GenerateOptions,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct GenerateOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessageOut,
}

#[derive(Deserialize)]
struct ChatMessageOut {
    content: String,
}

pub fn fetch_models() -> Result<Vec<String>, String> {
    let url = format!("{OLLAMA_V1_BASE_URL}{OLLAMA_MODELS_PATH}");
    let response: ModelsResponse = ureq::get(&url)
        .call()
        .map_err(|e| format!("fetch models failed: {e}"))?
        .into_json()
        .map_err(|e| format!("parse models failed: {e}"))?;
    let mut models: Vec<String> = response.data.into_iter().map(|entry| entry.id).collect();
    models.sort();
    Ok(models)
}

pub fn post_process(model: &str, text: &str) -> Result<String, String> {
    let request = ChatRequest {
        model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: POST_PROCESS_SYSTEM.to_string(),
            },
            ChatMessage {
                role: "user",
                content: text.to_string(),
            },
        ],
        stream: false,
        options: GenerateOptions { temperature: 0.0 },
    };
    let response: ChatResponse = ureq::post(OLLAMA_CHAT_URL)
        .send_json(request)
        .map_err(|e| format!("post-process request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("post-process parse failed: {e}"))?;
    let output = response.message.content.trim().to_string();
    if output.is_empty() {
        return Err("post-process response was empty".to_string());
    }
    Ok(output)
}
