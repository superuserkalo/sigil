//! Player edits on top of a config, and the history of them.
//!
//! A config writes hundreds of ConVars. Players tune a few and want to know what
//! they changed, so every edit is kept under the app data directory, keyed by
//! config id, next to a history of the values it replaced. An apply merges the
//! edits into the same `gameinfo.gi` block the config writes, so turning the
//! config off still puts every stock value back.
//!
//! The file is small JSON. It is written atomically, and a missing or unreadable
//! file simply means no edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util::{now_iso, write_atomic};

const FILE: &str = "convar-edits.json";
/// The editor keeps the most recent edits; older ones stop being useful.
const HISTORY_LIMIT: usize = 250;

/// Every config the player has edited.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edits {
    #[serde(default)]
    pub configs: BTreeMap<String, ConfigEdits>,
}

/// The edits over one config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEdits {
    /// ConVar key to the value the player set.
    pub values: BTreeMap<String, String>,
    /// Newest last.
    pub history: Vec<HistoryEntry>,
}

/// One change in the edit history: what the key held and what it holds now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// RFC 3339, as `now_iso`.
    pub at: String,
    pub key: String,
    /// The value before. `None` when the key had no value yet.
    pub from: Option<String>,
    /// The value after.
    pub to: String,
}

/// A player edit that is safe to write as one `key "value"` line in gameinfo.gi: the key is
/// a plain ConVar name and the value cannot close the quotes, end the line or open a block.
pub fn is_valid_edit(key: &str, value: &str) -> bool {
    let key_ok = !key.is_empty()
        && key.len() <= 128
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    let value_ok = !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(|c| matches!(c, '"' | '\n' | '\r' | '{' | '}' | '\0') || c.is_control());
    key_ok && value_ok
}

impl Edits {
    pub fn for_config(&self, config: &str) -> &ConfigEdits {
        static EMPTY: std::sync::OnceLock<ConfigEdits> = std::sync::OnceLock::new();
        self.configs
            .get(config)
            .unwrap_or_else(|| EMPTY.get_or_init(ConfigEdits::default))
    }

    /// Record a value, keeping the previous one in the history. Refuses (returns false) a
    /// key or value that could break out of its gameinfo.gi line: see [`is_valid_edit`].
    pub fn set(&mut self, config: &str, key: &str, from: Option<&str>, to: &str) -> bool {
        if !is_valid_edit(key, to) {
            return false;
        }
        let entry = self.configs.entry(config.to_string()).or_default();
        entry.values.insert(key.to_string(), to.to_string());
        entry.push(HistoryEntry {
            at: now_iso(),
            key: key.to_string(),
            from: from.map(str::to_string),
            to: to.to_string(),
        });
        true
    }

    /// Drop the player's value, so the config's own value applies again. The
    /// history keeps the change and the value it went back to.
    pub fn reset(&mut self, config: &str, key: &str, from: Option<&str>, author: &str) {
        let entry = self.configs.entry(config.to_string()).or_default();
        entry.values.remove(key);
        entry.push(HistoryEntry {
            at: now_iso(),
            key: key.to_string(),
            from: from.map(str::to_string),
            to: author.to_string(),
        });
    }

    /// Drop every edit for a config, history included.
    pub fn clear(&mut self, config: &str) {
        self.configs.remove(config);
    }
}

impl ConfigEdits {
    /// The value the player set, if they changed this key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn push(&mut self, entry: HistoryEntry) {
        self.history.push(entry);
        let over = self.history.len().saturating_sub(HISTORY_LIMIT);
        if over > 0 {
            self.history.drain(..over);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.history.is_empty()
    }
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Read the saved edits. A missing or unreadable file means none.
pub fn load(data_dir: &Path) -> Edits {
    std::fs::read_to_string(path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write the edits. Runs on a worker: the editor saves from a user action.
pub fn save(data_dir: &Path, edits: &Edits) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    write_atomic(
        &path(data_dir),
        serde_json::to_string_pretty(edits)?.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sigil-edits-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_saved_edit_survives_and_keeps_its_history() {
        let dir = temp_dir("save");
        let mut edits = Edits::default();
        edits.set("sqooky", "r_shadows", Some("1"), "0");
        save(&dir, &edits).expect("save");
        let read = load(&dir);
        assert_eq!(read.for_config("sqooky").get("r_shadows"), Some("0"));
        let history = &read.for_config("sqooky").history;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].from.as_deref(), Some("1"));
        assert_eq!(history[0].to, "0");
        edits.reset("sqooky", "r_shadows", Some("0"), "1");
        assert_eq!(edits.for_config("sqooky").get("r_shadows"), None);
        assert_eq!(
            edits.for_config("sqooky").history.last().map(|entry| entry.to.as_str()),
            Some("1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_config_keeps_its_own_edits() {
        let mut edits = Edits::default();
        edits.set("sqooky", "r_shadows", Some("1"), "0");
        edits.set("optilock-fps", "fps_max", None, "240");
        assert_eq!(edits.for_config("sqooky").get("r_shadows"), Some("0"));
        assert_eq!(edits.for_config("optilock-fps").get("fps_max"), Some("240"));
        edits.clear("sqooky");
        assert!(edits.for_config("sqooky").is_empty());
        assert_eq!(edits.for_config("optilock-fps").get("fps_max"), Some("240"));
    }

    #[test]
    fn history_is_capped() {
        let mut edits = Edits::default();
        for i in 0..HISTORY_LIMIT + 20 {
            edits.set("sqooky", "fps_max", None, &i.to_string());
        }
        let history = &edits.for_config("sqooky").history;
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history.last().map(|entry| entry.to.as_str()), Some("269"));
    }
}

#[cfg(test)]
mod edit_validation_tests {
    use super::*;

    #[test]
    fn edits_that_could_break_out_of_a_gameinfo_line_are_refused() {
        assert!(is_valid_edit("r_fullscreen_gamma", "2.2"));
        assert!(is_valid_edit("cl.some_value", "-1"));
        for (key, value) in [
            ("ok_key", "1\"\n}\nGame evil"),
            ("ok_key", "1\r\n"),
            ("ok_key", "{"),
            ("ok_key", "1\u{0}"),
            ("ok_key", ""),
            ("bad key", "1"),
            ("bad\"key", "1"),
            ("", "1"),
        ] {
            assert!(!is_valid_edit(key, value), "{key:?} = {value:?} must be refused");
        }
        let mut edits = Edits::default();
        assert!(!edits.set("cfg", "ok_key", None, "1\"\n}"));
        assert!(edits.for_config("cfg").values.is_empty());
        assert!(edits.set("cfg", "ok_key", None, "2"));
    }
}

