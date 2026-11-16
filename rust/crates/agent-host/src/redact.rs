//! Key-shaped string redaction. The host applies this to every outbound
//! envelope, every diagnostic line and every tool result.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;

struct Rule {
    pattern: Regex,
    keep_prefix: bool,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(r"\bsk-[A-Za-z0-9_-]{16,}\b", false),
        rule(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", false),
        rule(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b", false),
        rule(r"\bAKIA[0-9A-Z]{16}\b", false),
        rule(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b", false),
        rule(r"(?i)(\bBearer\s+)[A-Za-z0-9._~+/=-]{16,}", true),
        rule(
            r#"(\b(?:DAH_AGENT_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY)\s*[=:]\s*)[^\s"',]{8,}"#,
            true,
        ),
        rule(
            r#"(?i)(\b(?:api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password)\s*["']?\s*[:=]\s*["']?)[A-Za-z0-9._~+/=-]{16,}"#,
            true,
        ),
    ]
});

fn rule(pattern: &str, keep_prefix: bool) -> Rule {
    Rule {
        pattern: Regex::new(pattern).expect("static redaction pattern compiles"),
        keep_prefix,
    }
}

/// Replace key-shaped strings. Idempotent: a redacted string stays redacted.
pub fn secrets(text: &str) -> String {
    let mut result = text.to_string();
    for rule in RULES.iter() {
        result = if rule.keep_prefix {
            rule.pattern
                .replace_all(&result, |captures: &Captures<'_>| {
                    format!("{}[REDACTED]", &captures[1])
                })
                .into_owned()
        } else {
            rule.pattern.replace_all(&result, "[REDACTED]").into_owned()
        };
    }
    result
}

/// Redact every string in a JSON value, keys included.
pub fn value(value: &mut Value) {
    match value {
        Value::String(text) => *text = secrets(text),
        Value::Array(items) => {
            for item in items {
                self::value(item);
            }
        }
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (key, mut item) in entries {
                self::value(&mut item);
                map.insert(secrets(&key), item);
            }
        }
        _ => {}
    }
}
