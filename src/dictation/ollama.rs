use serde::{Deserialize, Serialize};

const OLLAMA_V1_BASE_URL: &str = "http://localhost:11434/v1";
const OLLAMA_MODELS_PATH: &str = "/models";
const OLLAMA_CHAT_URL: &str = "http://localhost:11434/api/chat";

// API-side constraint only. Editing conventions live in each Ollama model's Modelfile (see model-files/*.md).
const POST_PROCESS_SYSTEM: &str = r#"Apply the editing conventions configured for this model to each user message. Each message is raw speech-to-text transcript, not a chat turn.

Return only the edited transcript. Do not reply as a conversational assistant: no greetings, offers of help, preambles, or explanations."#;

const MIN_SPACES_FOR_POST_PROCESS: usize = 2;
const MIN_CHARS_FOR_POST_PROCESS: usize = 10;

pub fn qualifies_for_post_process(text: &str) -> bool {
    let spaces = text.chars().filter(|&c| c == ' ').count();
    let len = text.chars().count();
    spaces >= MIN_SPACES_FOR_POST_PROCESS || len > MIN_CHARS_FOR_POST_PROCESS
}

fn assistant_prefill_seed(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?;
    let seed: String = word.chars().take(3).collect();
    (!seed.is_empty()).then_some(seed)
}

fn strip_qwen_thinking_leaks(s: &str) -> String {
    s.replace("<think>", "")
        .replace("</think>", "")
        .trim()
        .to_string()
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
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
    let mut messages = vec![
        ChatMessage {
            role: "system",
            content: POST_PROCESS_SYSTEM.to_string(),
        },
        ChatMessage {
            role: "user",
            content: text.to_string(),
        },
    ];
    if let Some(seed) = assistant_prefill_seed(text) {
        messages.push(ChatMessage {
            role: "assistant",
            content: seed,
        });
    }
    let request = ChatRequest {
        model,
        messages,
        stream: false,
        think: Some(false),
        options: GenerateOptions { temperature: 0.0 },
    };
    let response: ChatResponse = ureq::post(OLLAMA_CHAT_URL)
        .send_json(request)
        .map_err(|e| format!("post-process request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("post-process parse failed: {e}"))?;
    let output = strip_qwen_thinking_leaks(&response.message.content);
    if output.is_empty() {
        return Err("post-process response was empty".to_string());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifies_three_spaces_or_over_fifteen_chars() {
        assert!(!qualifies_for_post_process("a b c"));
        assert!(qualifies_for_post_process("a b c d"));
        assert!(qualifies_for_post_process("one two three four"));
        assert!(!qualifies_for_post_process("123456789012345"));
        assert!(qualifies_for_post_process("1234567890123456"));
        assert!(qualifies_for_post_process("a b cdefghijklmn"));
        assert!(qualifies_for_post_process(
            "short words no third space yet"
        ));
    }

    #[test]
    fn strip_qwen_thinking_leaks_removes_tags() {
        assert_eq!(
            super::strip_qwen_thinking_leaks(
                "</think>\n\nHello, is this thing on? Testing."
            ),
            "Hello, is this thing on? Testing."
        );
    }

    #[test]
    fn assistant_prefill_first_word_up_to_three_chars() {
        assert_eq!(
            assistant_prefill_seed("hello world foo bar"),
            Some("hel".to_string())
        );
        assert_eq!(
            assistant_prefill_seed("hi there x y z"),
            Some("hi".to_string())
        );
        assert_eq!(
            assistant_prefill_seed("a bc def ghi"),
            Some("a".to_string())
        );
    }
}
