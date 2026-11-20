//! Reading the KV3-as-JSON shape the bundle stores (Grimoire's `ParticleDefinitionParser`):
//! numbers may arrive as numbers, booleans or strings, enums as names or integers, vectors as
//! arrays or space separated strings. A missing key is the default.

use serde_json::Value;

use crate::math::Vec3;

pub fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object().and_then(|o| o.get(key)).filter(|x| !x.is_null())
}

/// A JSON scalar as a number (true is 1, a numeric string parses).
pub fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// A float field. An object (a float input authored where a literal is read) takes its
/// literal value, as Grimoire's parser does after evaluating it with a default state.
pub fn f(v: &Value, key: &str, default: f32) -> f32 {
    match get(v, key) {
        None => default,
        Some(Value::Object(o)) => o.get("m_flLiteralValue").and_then(number).unwrap_or(0.0) as f32,
        Some(x) => number(x).map_or(0.0, |n| n as f32),
    }
}

pub fn i(v: &Value, key: &str, default: i32) -> i32 {
    match get(v, key) {
        None => default,
        Some(Value::Object(o)) => o.get("m_flLiteralValue").and_then(number).unwrap_or(0.0) as i32,
        Some(x) => number(x).map_or(0, |n| n as i32),
    }
}

pub fn b(v: &Value, key: &str, default: bool) -> bool {
    match get(v, key) {
        None => default,
        Some(Value::Bool(x)) => *x,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            other => other.parse::<f64>().map(|n| n != 0.0).unwrap_or(false),
        },
        Some(_) => default,
    }
}

pub fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    get(v, key).and_then(Value::as_str)
}

pub fn vec3_of(x: &Value) -> Option<Vec3> {
    match x {
        Value::Array(a) => {
            let c = |k: usize| a.get(k).and_then(number).unwrap_or(0.0) as f32;
            Some([c(0), c(1), c(2)])
        }
        Value::String(s) => {
            let mut it = s.split_whitespace().map(|p| p.parse::<f32>().unwrap_or(0.0));
            Some([it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0)])
        }
        _ => None,
    }
}

pub fn v3(v: &Value, key: &str, default: Vec3) -> Vec3 {
    get(v, key).and_then(vec3_of).unwrap_or(default)
}

/// A 0-255 colour as 0-1.
pub fn color(v: &Value, key: &str, default: Vec3) -> Vec3 {
    get(v, key).and_then(vec3_of).map_or(default, |c| [c[0] / 255.0, c[1] / 255.0, c[2] / 255.0])
}

/// A particle field index (`m_nOutputField: 3`).
pub fn field(v: &Value, key: &str, default: usize) -> i32 {
    match get(v, key) {
        Some(x) if !x.is_object() && !x.is_array() => number(x).map_or(default as i32, |n| n as i32),
        _ => default as i32,
    }
}

/// An enum read by name (`"PF_MAP_TYPE_REMAP"`) or by value. `table` maps names to values;
/// an unknown name or value is the default.
pub fn enumv(v: &Value, key: &str, table: &[(&str, i32)], default: i32) -> i32 {
    match get(v, key) {
        None => default,
        Some(Value::String(s)) => {
            let t = s.trim();
            if let Ok(n) = t.parse::<i32>() {
                return if table.iter().any(|(_, x)| *x == n) { n } else { default };
            }
            table.iter().find(|(name, _)| name.eq_ignore_ascii_case(t)).map_or(default, |(_, x)| *x)
        }
        Some(x) => {
            let n = number(x).map_or(default, |n| n as i32);
            if table.iter().any(|(_, x)| *x == n) { n } else { default }
        }
    }
}

pub fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    get(v, key).and_then(Value::as_array).map_or(&[], |a| a.as_slice())
}
