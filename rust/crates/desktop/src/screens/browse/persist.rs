//! Browse state that outlives a run: filters, view options, hidden mods and creators, ignored
//! updates and the preview volume. Favorites keep their own file (`favorites.json`).
//!
//! `state.json` is small and written on the press that changed it, next to the final name and
//! renamed over, so a crash never leaves half a file.

use std::path::PathBuf;

use serde_json::Value;

/// One hidden mod, kept with the name its management row shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiddenMod {
    pub id: u64,
    pub model: String,
    pub name: String,
}

/// Everything under `browse/state.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Persisted {
    pub section: String,
    pub tab: String,
    pub category: Option<(u64, String)>,
    pub sub: Option<(u64, String)>,
    pub hero: Option<String>,
    pub order: Option<String>,
    pub period: String,
    pub date_field: String,
    pub date_from: i64,
    pub date_to: i64,
    pub featured: bool,
    pub creator: Option<(u64, String)>,
    pub owned: String,
    pub mature: String,
    pub hide_outdated: bool,
    pub exclude: bool,
    pub layout: String,
    pub card_size: String,
    pub paged: bool,
    pub audio_volume: f32,
    /// The level before a mute, so Unmute after a restart brings it back.
    pub audio_unmuted: Option<f32>,
    pub hidden_mods: Vec<HiddenMod>,
    pub hidden_authors: Vec<String>,
    /// Date hidden, note, picture and the mod a creator was hidden from (Grimoire #580).
    pub hidden_meta: std::collections::BTreeMap<String, super::hidden::HiddenMeta>,
    pub ignored_updates: Vec<u64>,
}

pub fn state_path() -> PathBuf {
    crate::data::data_dir().join("browse").join("state.json")
}

impl Default for Persisted {
    fn default() -> Self {
        Persisted {
            section: String::new(),
            tab: String::new(),
            category: None,
            sub: None,
            hero: None,
            order: None,
            period: String::new(),
            date_field: String::new(),
            date_from: 0,
            date_to: 0,
            featured: false,
            creator: None,
            owned: String::new(),
            mature: String::new(),
            hide_outdated: false,
            exclude: false,
            layout: String::new(),
            card_size: String::new(),
            paged: false,
            audio_volume: 0.7,
            audio_unmuted: None,
            hidden_mods: Vec::new(),
            hidden_authors: Vec::new(),
            hidden_meta: Default::default(),
            ignored_updates: Vec::new(),
        }
    }
}

/// Read the saved state. A missing or unreadable file is the default state.
pub fn read() -> Persisted {
    if rune::is_headless() {
        return Persisted::default();
    }
    std::fs::read(state_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|value| from_value(&value))
        .unwrap_or_default()
}

pub(crate) fn from_value(value: &Value) -> Persisted {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let pick = |key: &str| {
        value.get(key).and_then(|pick| {
            Some((
                pick.get("id")?.as_u64()?,
                pick.get("name")?.as_str()?.to_string(),
            ))
        })
    };
    let hidden_mods = value
        .get("hiddenMods")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    Some(HiddenMod {
                        id: item.get("id")?.as_u64()?,
                        model: item
                            .get("model")
                            .and_then(Value::as_str)
                            .unwrap_or("Mod")
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let hidden_authors = value
        .get("hiddenAuthors")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let ignored_updates = value
        .get("ignoredUpdates")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    Persisted {
        section: text("section"),
        tab: text("tab"),
        category: pick("category"),
        sub: pick("sub"),
        hero: value
            .get("hero")
            .and_then(Value::as_str)
            .map(str::to_string),
        order: value
            .get("order")
            .and_then(Value::as_str)
            .map(str::to_string),
        period: text("period"),
        date_field: text("dateField"),
        date_from: value.get("dateFrom").and_then(Value::as_i64).unwrap_or(0),
        date_to: value.get("dateTo").and_then(Value::as_i64).unwrap_or(0),
        featured: value
            .get("featured")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        creator: pick("creator"),
        owned: text("owned"),
        mature: text("mature"),
        hide_outdated: value
            .get("hideOutdated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        exclude: value
            .get("exclude")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        layout: text("layout"),
        card_size: text("cardSize"),
        paged: value.get("paged").and_then(Value::as_bool).unwrap_or(false),
        audio_volume: value
            .get("audioVolume")
            .and_then(Value::as_f64)
            .map(|volume| volume as f32)
            .unwrap_or(0.7),
        audio_unmuted: value
            .get("audioUnmuted")
            .and_then(Value::as_f64)
            .map(|volume| volume as f32)
            .filter(|volume| *volume > 0.0),
        hidden_mods,
        hidden_authors,
        hidden_meta: value
            .get("hiddenMeta")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .take(2000)
                    .map(|(key, meta)| (key.clone(), super::hidden::HiddenMeta::from_json(meta)))
                    .collect()
            })
            .unwrap_or_default(),
        ignored_updates,
    }
}

/// Write the state atomically. Never called from a frame: only from the press that changed it.
pub fn write(state: &Persisted) -> Result<(), String> {
    if rune::is_headless() {
        return Ok(());
    }
    let path = state_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let body = to_value(state);
    let bytes = serde_json::to_vec_pretty(&body).map_err(|error| error.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())
}

pub(crate) fn to_value(state: &Persisted) -> Value {
    let pick = |pick: &Option<(u64, String)>| match pick {
        Some((id, name)) => serde_json::json!({ "id": id, "name": name }),
        None => Value::Null,
    };
    serde_json::json!({
        "section": state.section,
        "tab": state.tab,
        "category": pick(&state.category),
        "sub": pick(&state.sub),
        "hero": state.hero,
        "order": state.order,
        "period": state.period,
        "dateField": state.date_field,
        "dateFrom": state.date_from,
        "dateTo": state.date_to,
        "featured": state.featured,
        "creator": pick(&state.creator),
        "owned": state.owned,
        "mature": state.mature,
        "hideOutdated": state.hide_outdated,
        "exclude": state.exclude,
        "layout": state.layout,
        "cardSize": state.card_size,
        "paged": state.paged,
        "audioVolume": state.audio_volume,
        "audioUnmuted": state.audio_unmuted,
        "hiddenMods": state.hidden_mods.iter().map(|hidden| serde_json::json!({
            "id": hidden.id,
            "model": hidden.model,
            "name": hidden.name,
        })).collect::<Vec<_>>(),
        "hiddenAuthors": state.hidden_authors,
        "hiddenMeta": state.hidden_meta.iter().map(|(key, meta)| (key.clone(), meta.to_json())).collect::<serde_json::Map<String, Value>>(),
        "ignoredUpdates": state.ignored_updates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
        #[test]
    fn every_field_round_trips_through_the_value_shape() {
        let state = Persisted {
            section: "Sounds".into(),
            tab: "Sounds".into(),
            category: Some((33295, "Skins".into())),
            sub: Some((33311, "Haze".into())),
            hero: Some("Haze".into()),
            order: Some("MostLiked".into()),
            period: "Week".into(),
            date_field: "Updated".into(),
            date_from: 1,
            date_to: 2,
            featured: true,
            creator: Some((7, "someone".into())),
            owned: "Installed".into(),
            mature: "Hide".into(),
            hide_outdated: true,
            exclude: true,
            layout: "List".into(),
            card_size: "Large".into(),
            paged: true,
            audio_volume: 0.4,
            audio_unmuted: None,
            hidden_mods: vec![HiddenMod {
                id: 5,
                model: "Mod".into(),
                name: "Name".into(),
            }],
            hidden_authors: vec!["a".into()],
            hidden_meta: [(
                "author:a".to_string(),
                super::super::hidden::HiddenMeta {
                    hidden_at: 1_763_700_000,
                    note: "spoilers".into(),
                    from: Some((5, "Name".into())),
                    thumb: Some("https://images.gamebanana.com/x.jpg".into()),
                    author: Some("a".into()),
                },
            )]
            .into_iter()
            .collect(),
            ignored_updates: vec![9],
        };
        let value = to_value(&state);
        assert_eq!(from_value(&value), state);
        assert_eq!(from_value(&serde_json::json!({})), Persisted::default());
    }

    #[test]
    fn a_mute_survives_a_restart_with_its_level() {
        let state = Persisted {
            audio_volume: 0.0,
            audio_unmuted: Some(0.55),
            ..Persisted::default()
        };
        let back = from_value(&to_value(&state));
        assert_eq!(back.audio_volume, 0.0);
        assert_eq!(back.audio_unmuted, Some(0.55));
    }
}
