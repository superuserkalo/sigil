//! KV3 resources as JSON, and the loose value shapes Source 2 entity lumps use.

use morphic::kv3::Value;
use serde_json::Value as Json;

use crate::error::{Error, Result};

/// Decode a compiled KV3 resource (`.vwrld_c`, `.vwnod_c`, `.vents_c`, `.vmat_c`, ...) to JSON.
pub(crate) fn resource_json(bytes: &[u8], what: &str) -> Result<Json> {
    let value = morphic::decode_kv3_resource(bytes)
        .map_err(|error| Error::msg(format!("decoding {what}: {error:?}")))?;
    Ok(to_json(&value))
}

/// The KV3 tree as JSON. Integers stay integers; a binary blob becomes a byte array.
pub(crate) fn to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Int(number) => Json::from(*number),
        Value::UInt(number) => Json::from(*number),
        Value::Double(number) => {
            serde_json::Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        Value::String(text) => Json::String(text.clone()),
        Value::Binary(bytes) => Json::Array(bytes.iter().map(|byte| Json::from(*byte)).collect()),
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        Value::Object(pairs) => Json::Object(
            pairs
                .iter()
                .map(|(key, child)| (key.clone(), to_json(child)))
                .collect(),
        ),
    }
}

/// A number, or a number written as a string (entity lumps do both).
pub(crate) fn num(value: Option<&Json>) -> Option<f64> {
    match value? {
        Json::Number(number) => number.as_f64(),
        Json::String(text) => text.trim().parse().ok(),
        Json::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
    .filter(|number| number.is_finite())
}

/// A boolean, or `1`/`0`, or `"true"`.
pub(crate) fn flag(value: Option<&Json>) -> Option<bool> {
    match value? {
        Json::Bool(flag) => Some(*flag),
        Json::Number(number) => number.as_f64().map(|number| number != 0.0),
        Json::String(text) => match text.trim() {
            "1" | "true" | "True" => Some(true),
            "0" | "false" | "False" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Three numbers from an array or a space separated string.
pub(crate) fn vec3(value: Option<&Json>) -> Option<[f64; 3]> {
    let parts: Vec<f64> = match value? {
        Json::Array(items) => items
            .iter()
            .map(|item| num(Some(item)))
            .collect::<Option<Vec<_>>>()?,
        Json::String(text) => text
            .split_whitespace()
            .map(|part| part.parse::<f64>().ok())
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    (parts.len() >= 3 && parts.iter().take(3).all(|part| part.is_finite()))
        .then(|| [parts[0], parts[1], parts[2]])
}

pub(crate) fn text(value: Option<&Json>) -> Option<&str> {
    value?.as_str().filter(|text| !text.is_empty())
}

pub(crate) fn array(value: Option<&Json>) -> &[Json] {
    value
        .and_then(Json::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
