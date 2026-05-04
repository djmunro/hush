use crate::config::CleanupConfig;

pub fn apply_cleanup(text: &str, cleanup_cfg: &CleanupConfig) -> String {
    let mut result = text.to_string();

    if cleanup_cfg.capitalize {
        result = remove_beginning_caps(&result);
    }
    if cleanup_cfg.end_period {
        result = remove_end_period(&result);
    }
    if cleanup_cfg.end_question {
        result = remove_end_question(&result);
    }

    result
}

fn remove_beginning_caps(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            if !first.is_uppercase() {
                return text.to_string();
            }

            let rest = chars.as_str();
            if rest.is_empty() {
                return first.to_lowercase().collect();
            }

            let second_char = rest.chars().next().unwrap();
            if second_char == ' ' || second_char.is_lowercase() {
                let lowercase = first.to_lowercase().collect::<String>();
                lowercase + rest
            } else {
                text.to_string()
            }
        }
    }
}

fn remove_end_period(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return text.to_string();
    }

    if trimmed.ends_with('.') && trimmed.len() > 1 {
        trimmed[..trimmed.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

fn remove_end_question(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return text.to_string();
    }

    if trimmed.ends_with('?') && trimmed.len() > 1 {
        trimmed[..trimmed.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_beginning_caps_single_char_with_space() {
        assert_eq!(remove_beginning_caps("H ello"), "h ello");
    }

    #[test]
    fn remove_beginning_caps_single_char_with_lowercase() {
        assert_eq!(remove_beginning_caps("Hello"), "hello");
    }

    #[test]
    fn remove_beginning_caps_no_change_uppercase_followed_by_uppercase() {
        assert_eq!(remove_beginning_caps("HELLO"), "HELLO");
    }

    #[test]
    fn remove_beginning_caps_lowercase() {
        assert_eq!(remove_beginning_caps("hello"), "hello");
    }

    #[test]
    fn remove_beginning_caps_single_char() {
        assert_eq!(remove_beginning_caps("H"), "h");
    }

    #[test]
    fn remove_beginning_caps_empty() {
        assert_eq!(remove_beginning_caps(""), "");
    }

    #[test]
    fn remove_end_period_with_period() {
        assert_eq!(remove_end_period("hello."), "hello");
    }

    #[test]
    fn remove_end_period_without_period() {
        assert_eq!(remove_end_period("hello"), "hello");
    }

    #[test]
    fn remove_end_period_only_period() {
        assert_eq!(remove_end_period("."), ".");
    }

    #[test]
    fn remove_end_period_with_other_punctuation() {
        assert_eq!(remove_end_period("hello?"), "hello?");
    }

    #[test]
    fn remove_end_period_empty() {
        assert_eq!(remove_end_period(""), "");
    }

    #[test]
    fn apply_cleanup_both_disabled() {
        let cfg = CleanupConfig {
            capitalize: false,
            end_period: false,
        };
        assert_eq!(apply_cleanup("Hello world.", &cfg), "Hello world.");
    }

    #[test]
    fn apply_cleanup_both_enabled() {
        let cfg = CleanupConfig {
            capitalize: true,
            end_period: true,
        };
        assert_eq!(apply_cleanup("Hello world.", &cfg), "hello world");
    }

    #[test]
    fn apply_cleanup_capitalize_only() {
        let cfg = CleanupConfig {
            capitalize: true,
            end_period: false,
        };
        assert_eq!(apply_cleanup("Hello world.", &cfg), "hello world.");
    }

    #[test]
    fn apply_cleanup_period_only() {
        let cfg = CleanupConfig {
            capitalize: false,
            end_period: true,
        };
        assert_eq!(apply_cleanup("Hello world.", &cfg), "Hello world");
    }
}
