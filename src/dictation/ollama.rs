use serde::Deserialize;

const OLLAMA_V1_BASE_URL: &str = "http://localhost:11434/v1";
const OLLAMA_MODELS_PATH: &str = "/models";
const OLLAMA_GENERATE_URL: &str = "http://localhost:11434/api/generate";
const SYSTEM_PROMPT: &str = "You are a transcription post-processor. Rewrite the provided transcript text only. Do not answer questions, do not add new information, and do not acknowledge or respond conversationally.";

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(serde::Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    system: &'a str,
    prompt: String,
    raw: bool,
    stream: bool,
    options: GenerateOptions,
}

#[derive(serde::Serialize)]
struct GenerateOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
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

pub fn post_process(model: &str, prompt_template: &str, text: &str) -> Result<String, String> {
    let transformed_prompt = if prompt_template.contains("${output}") {
        prompt_template.replace("${output}", text)
    } else {
        format!("{prompt_template}\n\nInput:\n{text}")
    };
    let prompt = format!(
        "<CONTRACT>\n\
You must transform text only.\n\
Return output text only.\n\
No preamble, no explanation, no labels.\n\
Do not answer the user as a chatbot.\n\
Do not add facts not present in input.\n\
Do not add terminal punctuation at the end of lines.\n\
If input starts lowercase, do not force-capitalize the first letter.\n\
</CONTRACT>\n\
<USER_RULES>\n\
{transformed_prompt}\n\
</USER_RULES>\n\
<INPUT>\n\
{text}\n\
</INPUT>\n\
<OUTPUT>\n"
    );
    let request = GenerateRequest {
        model,
        system: SYSTEM_PROMPT,
        prompt,
        raw: true,
        stream: false,
        options: GenerateOptions { temperature: 0.0 },
    };
    let response: GenerateResponse = ureq::post(OLLAMA_GENERATE_URL)
        .send_json(request)
        .map_err(|e| format!("post-process request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("post-process parse failed: {e}"))?;
    let output = normalize_output(text, &response.response);
    if output.is_empty() {
        return Err("post-process response was empty".to_string());
    }
    Ok(output)
}

fn normalize_output(input: &str, raw_output: &str) -> String {
    let mut out_lines = Vec::new();
    for line in raw_output.lines() {
        let stripped = line
            .trim_end()
            .trim_end_matches(['.', '!', '?'])
            .to_string();
        out_lines.push(stripped);
    }
    let mut output = out_lines.join("\n").trim().to_string();
    let input_starts_lower = input
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false);
    if input_starts_lower {
        let mut chars = output.chars().collect::<Vec<char>>();
        if let Some(idx) = chars.iter().position(|c| c.is_ascii_alphabetic()) {
            chars[idx] = chars[idx].to_ascii_lowercase();
            output = chars.into_iter().collect();
        }
    }
    output
}
