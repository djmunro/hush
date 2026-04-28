use std::path::PathBuf;

const DEFAULT_POST_PROCESS_MODEL: &str = "qwen2.5:0.5b";
pub const DEFAULT_POST_PROCESS_PROMPT: &str = r#"You are a transcription and code editor. When given text, follow these rules in order:

RULE 1 - FIX GRAMMAR:
Fix all spelling and grammar errors in the text.

RULE 2 - REMOVE END PUNCTUATION:
Remove punctuation at the end of sentences (no periods, exclamation marks, or question marks).

RULE 3 - DETECT AND REWRITE CODE REFERENCES:
If any part of the text sounds like spoken code, rewrite it using correct code syntax.
Spoken code patterns to watch for:

- "dot" between words -> replace with . (console dot log -> console.log)
- "open paren" / "close paren" -> replace with ( ) (open paren x close paren -> (x))
- "equals equals" -> replace with ==
- "arrow" -> replace with =>
- "open bracket"/"close bracket" -> replace with [ ]
- "open curly"/"close curly" -> replace with { }
- "plus plus" -> replace with ++
- "not equals" -> replace with !=
- "greater than"/"less than" -> replace with > / <
- spoken variable names and function names should be kept as-is

RULE 4 - OUTPUT FORMAT:
Return ONLY the corrected text - no explanations, labels, or commentary.

EXAMPLES:

Input: "hello hello this is a test"
Output: "hello hello this is a test"

Input: "we need to call console dot log open paren message close paren to debug it"
Output: "we need to call console.log(message) to debug it"

Input: "i think the for loop needs i plus plus at the end not i equals i plus one"
Output: "i think the for loop needs i++ at the end not i = i + 1"

Input: "she went to the store and buyed some apple's?"
Output: "she went to the store and bought some apples"

Input: "use array dot filter open paren x arrow x greater than zero close paren to remove negatives"
Output: "use array.filter(x => x > 0) to remove negatives"

Input:
${output}"#;

fn prefs_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush")
}

fn backend_path() -> PathBuf {
    prefs_dir().join("backend")
}

fn post_process_enabled_path() -> PathBuf {
    prefs_dir().join("post-process-enabled")
}

fn post_process_model_path() -> PathBuf {
    prefs_dir().join("post-process-model")
}

fn post_process_prompt_path() -> PathBuf {
    prefs_dir().join("post-process-prompt")
}

/// Returns the active backend name. Env var HUSH_BACKEND takes priority.
pub fn get_backend() -> String {
    if let Ok(v) = std::env::var("HUSH_BACKEND") {
        return v;
    }
    std::fs::read_to_string(backend_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "whisper".to_string())
}

pub fn set_backend(backend: &str) {
    let path = backend_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, backend);
}

pub fn get_post_process_enabled() -> bool {
    std::fs::read_to_string(post_process_enabled_path())
        .map(|s| s.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn set_post_process_enabled(enabled: bool) {
    let path = post_process_enabled_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if enabled { "true" } else { "false" });
}

pub fn get_post_process_model() -> String {
    match std::fs::read_to_string(post_process_model_path()) {
        Ok(model) => {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                DEFAULT_POST_PROCESS_MODEL.to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => DEFAULT_POST_PROCESS_MODEL.to_string(),
    }
}

pub fn set_post_process_model(model: &str) {
    let path = post_process_model_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, model);
}

pub fn get_post_process_prompt() -> String {
    match std::fs::read_to_string(post_process_prompt_path()) {
        Ok(prompt) => {
            if prompt.trim().is_empty() {
                DEFAULT_POST_PROCESS_PROMPT.to_string()
            } else {
                prompt
            }
        }
        Err(_) => DEFAULT_POST_PROCESS_PROMPT.to_string(),
    }
}

pub fn set_post_process_prompt(prompt: &str) {
    let path = post_process_prompt_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, prompt);
}

pub fn reset_post_process_prompt() {
    set_post_process_prompt(DEFAULT_POST_PROCESS_PROMPT);
}
