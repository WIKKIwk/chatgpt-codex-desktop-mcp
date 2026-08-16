use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

const REDACTION_LABEL: &str = "[REDACTED]";

pub fn redact_text(text: &str) -> String {
    let key_value = key_value_regex()
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let prefix = captures.get(1).map_or("", |value| value.as_str());
            if captures.get(2).is_some() {
                format!("{prefix}\"{REDACTION_LABEL}\"")
            } else if captures.get(3).is_some() {
                format!("{prefix}'{REDACTION_LABEL}'")
            } else {
                format!("{prefix}{REDACTION_LABEL}")
            }
        })
        .into_owned();
    let mut redacted = key_value;
    for pattern in standalone_secret_patterns() {
        redacted = pattern
            .replace_all(&redacted, |captures: &regex::Captures<'_>| {
                let value = captures.get(0).map_or("", |match_| match_.as_str());
                if value.to_ascii_lowercase().starts_with("bearer ") {
                    format!("Bearer {REDACTION_LABEL}")
                } else {
                    REDACTION_LABEL.to_owned()
                }
            })
            .into_owned();
    }
    redacted
}

pub fn redact_value(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(redact_text(&value)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::Object(values) => Value::Object(redact_object(values)),
        other => other,
    }
}

fn redact_object(values: Map<String, Value>) -> Map<String, Value> {
    values
        .into_iter()
        .map(|(key, value)| {
            let value = if sensitive_key(&key) {
                match value {
                    Value::String(value) if !value.is_empty() => {
                        Value::String(REDACTION_LABEL.to_owned())
                    }
                    other => redact_value(other),
                }
            } else {
                redact_value(value)
            };
            (key, value)
        })
        .collect()
}

fn key_value_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)((?:api[_-]?key|token|access[_-]?token|refresh[_-]?token|secret|client[_-]?secret|password|passwd|pwd|authorization|cookie|set-cookie)\s*[:=]\s*)(?:"([^"]*)"|'([^']*)'|([^"'\s,;}{]+))"#,
        )
        .expect("valid redaction key-value regex")
    })
}

fn standalone_secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{12,}",
            r"\bsk-(?:proj-|admin-)?[A-Za-z0-9_-]{16,}\b",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
            r"\bgh[pousr]_[A-Za-z0-9_]{20,}\b",
            r"\bAIza[0-9A-Za-z_-]{20,}\b",
            r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("valid redaction regex"))
        .collect()
    })
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("passwd")
        || key.contains("pwd")
        || key.contains("authorization")
        || key.contains("cookie")
        || key.contains("api_key")
        || key.contains("api-key")
        || key.contains("apikey")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_text_handles_key_values_and_standalone_credentials() {
        let text = r#"token: "secret-value" Bearer abcdefghijklmnop sk-proj-abcdefghijklmnop"#;
        let redacted = redact_text(text);
        assert!(!redacted.contains("secret-value"));
        assert!(!redacted.contains("abcdefghijklmnop"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_value_recurses_without_replacing_non_secret_values() {
        let value = serde_json::json!({
            "token": "hidden",
            "nested": ["Bearer abcdefghijklmnop", "visible"],
            "count": 2
        });
        let redacted = redact_value(value);
        assert_eq!(redacted["token"], REDACTION_LABEL);
        assert_eq!(redacted["nested"][1], "visible");
        assert_eq!(redacted["count"], 2);
    }
}
