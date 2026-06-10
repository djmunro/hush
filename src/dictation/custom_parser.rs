use std::env;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use deno_core::JsRuntime;

use crate::config::CustomParserConfig;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

pub const DEFAULT_SCRIPT: &str = r#"let s = input.trim();
if (!s) return s;
s = s.replace(/([.!?])(?=[A-Za-z])/g, "$1 ");
s = s.replace(/([.!?]\s+)([a-z])/g, (m, p, c) => p + c.toUpperCase());
const tail = (prev || "").trimEnd();
if (!tail || /[.!?]$/.test(tail)) {
  s = s[0].toUpperCase() + s.slice(1);
} else if (!/^I\b/.test(s)) {
  s = s[0].toLowerCase() + s.slice(1);
}
if (tail && !/^[,.;:!?)\]]/.test(s)) s = " " + s;
return s;"#;

pub(crate) fn apply_with_timeout(
    cfg: &CustomParserConfig,
    input: &str,
    prev: &str,
    timeout_ms: u64,
) -> Option<String> {
    if !cfg.enabled || cfg.script.trim().is_empty() {
        if is_debug_enabled() {
            eprintln!("[hush] parser disabled");
        }
        return None;
    }

    if is_debug_enabled() {
        eprintln!("[hush] parser enabled");
        eprintln!("[hush] parser input: {input}");
    }

    let script = cfg.script.clone();
    let input = input.to_string();
    let prev = prev.to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = run_parser_once(&script, &input, &prev);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(output)) => {
            if is_debug_enabled() {
                eprintln!("[hush] parser output: {output}");
            }
            Some(output)
        }
        Ok(Err(e)) => {
            if is_debug_enabled() {
                eprintln!("[hush] parser failed: {e}");
            }
            None
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if is_debug_enabled() {
                eprintln!("[hush] parser timeout");
            }
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if is_debug_enabled() {
                eprintln!("[hush] parser worker disconnected");
            }
            None
        }
    }
}

pub fn apply(cfg: &CustomParserConfig, input: &str, prev: &str) -> Option<String> {
    let timeout_ms = parse_timeout_ms();
    apply_with_timeout(cfg, input, prev, timeout_ms)
}

fn run_parser_once(script: &str, input: &str, prev: &str) -> Result<String, String> {
    let mut runtime = JsRuntime::new(Default::default());
    let input = serde_json::to_string(input).map_err(|e| e.to_string())?;
    let prev = serde_json::to_string(prev).map_err(|e| e.to_string())?;
    let script = normalize_script(script);
    let source = format!(
        "(function(input, prev) {{\n{}\n}})({input}, {prev});\n",
        script
    );
    let wrapper = format!(
        r#"(() => {{
  const __result = {source}
  const __coerced = typeof __result === "number" ? String(__result) : __result;
  if (typeof __coerced !== "string") {{
    throw new Error("custom parser must return string or number");
  }}
  return __coerced;
}})()"#
    );
    if is_debug_enabled() {
        eprintln!("[hush] parser source:\n{source}");
        eprintln!("[hush] parser wrapper:\n{wrapper}");
    }

    let value = runtime
        .execute_script("custom_parser", wrapper)
        .map_err(|e| e.to_string())?;
    let mut scope = runtime.handle_scope();
    let value = value.open(&mut scope);
    let string = value
        .to_string(&mut scope)
        .ok_or_else(|| "parser did not return string".to_string())?;
    Ok(string.to_rust_string_lossy(&mut scope))
}

fn normalize_script(script: &str) -> String {
    script
        .replace(['“', '”'], "\"")
        .replace(['‘', '’'], "'")
        .replace('\u{00A0}', " ")
}

fn parse_timeout_ms() -> u64 {
    env::var("HUSH_PARSER_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
}

fn is_debug_enabled() -> bool {
    env::var("HUSH_DEBUG").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_config(script: &str) -> CustomParserConfig {
        CustomParserConfig {
            enabled: true,
            script: script.to_string(),
        }
    }

    #[test]
    fn parser_returns_strings() {
        let cfg = parser_config("return input;");
        let got = apply_with_timeout(&cfg, "Hello", "", 1_000).expect("parser output");
        assert_eq!(got, "Hello");
    }

    #[test]
    fn parser_converts_number() {
        let cfg = parser_config("return 42;");
        let got = apply_with_timeout(&cfg, "ignored", "", 1_000).expect("parser output");
        assert_eq!(got, "42");
    }

    #[test]
    fn parser_rejects_invalid_result() {
        let cfg = parser_config("return {a: 1};");
        let got = apply_with_timeout(&cfg, "input", "", 1_000);
        assert_eq!(got, None);
    }

    #[test]
    fn parser_times_out_on_infinite_loop() {
        let cfg = parser_config("while (true) {}");
        let got = apply_with_timeout(&cfg, "input", "", 5);
        assert_eq!(got, None);
    }

    #[test]
    fn parser_normalizes_smart_quotes() {
        let cfg = parser_config("return “bleh”;");
        let got = apply_with_timeout(&cfg, "Hello", "", 1_000).expect("parser output");
        assert_eq!(got, "bleh");
    }

    #[test]
    fn parser_receives_prev() {
        let cfg = parser_config("return prev + input;");
        let got = apply_with_timeout(&cfg, "world", "hello ", 1_000).expect("parser output");
        assert_eq!(got, "hello world");
    }

    fn run_default(input: &str, prev: &str) -> String {
        let cfg = parser_config(DEFAULT_SCRIPT);
        apply_with_timeout(&cfg, input, prev, 5_000).expect("parser output")
    }

    #[test]
    fn default_capitalizes_first_dictation() {
        assert_eq!(run_default("hello there.", ""), "Hello there.");
    }

    #[test]
    fn default_adds_space_after_period() {
        assert_eq!(run_default("One.Two.three", ""), "One. Two. Three");
    }

    #[test]
    fn default_capitalizes_after_sentence_punctuation() {
        assert_eq!(
            run_default("first. second? third! fourth", ""),
            "First. Second? Third! Fourth"
        );
    }

    #[test]
    fn default_leaves_decimals_alone() {
        assert_eq!(run_default("version 3.5 is out.", ""), "Version 3.5 is out.");
    }

    #[test]
    fn default_continues_unfinished_sentence() {
        assert_eq!(
            run_default("And then kept going", "I started a sentence"),
            " and then kept going"
        );
    }

    #[test]
    fn default_starts_new_sentence_after_period() {
        assert_eq!(
            run_default("a new sentence", "The last one ended."),
            " A new sentence"
        );
    }

    #[test]
    fn default_keeps_capital_i_when_continuing() {
        assert_eq!(
            run_default("I'll finish it", "He said that"),
            " I'll finish it"
        );
    }

    #[test]
    fn default_no_space_before_leading_punctuation() {
        assert_eq!(run_default(", and more", "Some text"), ", and more");
    }

    #[test]
    fn default_empty_input_stays_empty() {
        assert_eq!(run_default("   ", "Whatever came before."), "");
    }
}
