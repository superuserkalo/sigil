//! A KV3 tree as KV3 text, the form Source 2 tools write (`.vdata`, `.vsndevts`, `.vpcf`).
//! The explorer shows decoded data this way and exports it as an editable file.

use morphic::kv3::Value;

/// The header line a KV3 text file starts with. The format GUID is the generic one; the
/// decoded resource does not keep its original format name.
pub const KV3_HEADER: &str = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:generic:version{7412167c-06e9-4698-aff2-e63eb59037e7} -->";

/// Blobs longer than this print as a size note, not as bytes.
const BLOB_PRINT_MAX: usize = 64;

/// The tree as KV3 text with the header line.
pub fn to_text(value: &Value) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(KV3_HEADER);
    out.push('\n');
    write_value(value, 0, &mut out);
    out.push('\n');
    out
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push('\t');
    }
}

fn write_value(value: &Value, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Int(number) => out.push_str(&number.to_string()),
        Value::UInt(number) => out.push_str(&number.to_string()),
        Value::Double(number) => out.push_str(&format_double(*number)),
        Value::String(text) => write_string(text, out),
        Value::Binary(bytes) => {
            if bytes.len() > BLOB_PRINT_MAX {
                out.push_str(&format!("#[ /* {} bytes */ ]", bytes.len()));
            } else {
                out.push_str("#[");
                for byte in bytes {
                    out.push_str(&format!(" {byte:02X}"));
                }
                out.push_str(" ]");
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[  ]");
                return;
            }
            if items.iter().all(is_scalar) && items.len() <= 16 {
                out.push_str("[ ");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_value(item, depth, out);
                }
                out.push_str(" ]");
                return;
            }
            out.push('\n');
            indent(depth, out);
            out.push_str("[\n");
            for item in items {
                indent(depth + 1, out);
                write_value(item, depth + 1, out);
                out.push_str(",\n");
            }
            indent(depth, out);
            out.push(']');
        }
        Value::Object(pairs) => {
            if depth > 0 {
                out.push('\n');
                indent(depth, out);
            }
            out.push_str("{\n");
            for (key, child) in pairs {
                indent(depth + 1, out);
                write_key(key, out);
                out.push_str(" = ");
                write_value(child, depth + 1, out);
                out.push('\n');
            }
            indent(depth, out);
            out.push('}');
        }
    }
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::UInt(_) | Value::Double(_)
    )
}

fn write_key(key: &str, out: &mut String) {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !key.starts_with(|c: char| c.is_ascii_digit());
    if bare {
        out.push_str(key);
    } else {
        write_string(key, out);
    }
}

fn write_string(text: &str, out: &mut String) {
    if text.contains('\n') && !text.contains("\"\"\"") {
        out.push_str("\"\"\"\n");
        out.push_str(text);
        out.push_str("\n\"\"\"");
        return;
    }
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn format_double(number: f64) -> String {
    if !number.is_finite() {
        return "0.0".to_string();
    }
    if number.fract() == 0.0 && number.abs() < 1e15 {
        return format!("{number:.1}");
    }
    let text = format!("{number:.6}");
    let trimmed = text.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}
