//! Export the settings a config would write, and read a file back.
//!
//! The exported file is plain ConVars, one `key "value"` per line, with a short
//! header naming the config, its author and its license. Another manager, a
//! friend or the player can read it. Import accepts that file, a block copied
//! out of any `gameinfo.gi`, or a `ConVars { ... }` section, and ignores what it
//! cannot read.
//!
//! Nothing here touches the network or the install. The player reviews what was
//! read before it becomes an edit.

use super::{Config, Options};

/// What one import found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exchange {
    /// `key -> value`, the last line for a key wins.
    pub entries: Vec<(String, String)>,
    /// Lines that looked like settings but could not be read.
    pub skipped: usize,
}

impl Exchange {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// The settings an apply writes, in file order, as a shareable text file.
pub fn export_text(config: &Config, options: &Options) -> String {
    let mut lines = vec![
        "// Sigil performance settings".to_string(),
        format!(
            "// {} by {}{}",
            config.name(),
            config.def.author,
            if config.version.is_empty() {
                String::new()
            } else {
                format!(", version {}", config.version)
            }
        ),
        format!(
            "// {}, from {}",
            config.source.license,
            config.link()
        ),
        format!("// {} settings. Paste into a ConVars section.", count(config, options)),
        String::new(),
    ];
    for (key, value) in effective(config, options) {
        lines.push(format!("{key} \"{value}\""));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// How many ConVars an export writes.
pub fn count(config: &Config, options: &Options) -> usize {
    effective(config, options).len()
}

/// Every ConVar the apply writes with `options`, edits included.
pub fn effective(config: &Config, options: &Options) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut push = |key: &str, value: &str| {
        if !seen.insert(key.to_string()) {
            return;
        }
        let value = options.edits.get(key).map(String::as_str).unwrap_or(value);
        out.push((key.to_string(), value.to_string()));
    };
    for (key, value) in config.body_convars() {
        push(key, value);
    }
    for opt in config.switchable() {
        if options.opt_ins.contains(&opt.key) {
            push(&opt.key, &opt.value);
        }
    }
    for (key, value) in &options.edits {
        if config.convar_state(key) == super::ConvarState::Ignored {
            continue;
        }
        push(key, value);
    }
    out
}

/// Read a settings file. `text` may be the export, a bare list of ConVar lines
/// or a whole `gameinfo.gi`.
pub fn parse(text: &str) -> Exchange {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let block = convar_block(text);
    let mut out = Exchange::default();
    for line in block.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with('{') || line.starts_with('}') || line.contains('{') || line.contains('}') {
            continue;
        }
        match entry(line) {
            Some((key, value)) => {
                if let Some(found) = out.entries.iter_mut().find(|(k, _)| *k == key) {
                    found.1 = value;
                } else {
                    out.entries.push((key, value));
                }
            }
            None => out.skipped += 1,
        }
    }
    out
}

/// The inside of the first `ConVars { ... }` block, or the whole text when
/// there is none.
fn convar_block(text: &str) -> &str {
    let Some(at) = find_convar_section(text) else {
        return text;
    };
    let bytes = text.as_bytes();
    let start = at + 1;
    let mut depth = 1usize;
    for (index, byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..index];
                }
            }
            _ => {}
        }
    }
    &text[start..]
}

/// The offset of the opening brace of the first uncommented `ConVars` section.
fn find_convar_section(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = text[from..].find("ConVars") {
        let at = from + found;
        let line_start = text[..at].rfind('\n').map_or(0, |index| index + 1);
        if !text[line_start..at].contains("//")
            && let Some(brace) = text[at..].find('{')
            && text[at + "ConVars".len()..at + brace].trim().is_empty()
        {
            return Some(at + brace);
        }
        from = at + "ConVars".len();
    }
    None
}

/// `key value` or `"key" "value"`, with an optional trailing comment.
fn entry(line: &str) -> Option<(String, String)> {
    let (key, rest) = split_token(line)?;
    let key = key.trim_matches('"');
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
    {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let value = if let Some(quoted) = rest.strip_prefix('"') {
        let close = quoted.find('"')?;
        &quoted[..close]
    } else {
        let end = rest.find("//").unwrap_or(rest.len());
        rest[..end].trim()
    };
    Some((key.to_string(), value.to_string()))
}

/// Split one token off the front of a line, keeping the rest.
fn split_token(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix('"') {
        let close = rest.find('"')?;
        return Some((&rest[..close], &rest[close + 1..]));
    }
    let end = line
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(line.len());
    Some((&line[..end], &line[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::{Body, catalog};

    fn demo() -> Config {
        let def = catalog::config("sqooky").expect("known config");
        Config {
            def,
            source: catalog::source(def.source),
            version: "3.9".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            date: "2026-11-04".into(),
            path: "Sqooky's .gi/gameinfo.gi".into(),
            body: Body {
                convars: vec![
                    ("r_shadows".into(), "1".into()),
                    ("fps_max".into(), "144".into()),
                ],
                ..Body::default()
            },
            videos: Vec::new(),
            ignored: Vec::new(),
            unknown: Vec::new(),
        }
    }

    #[test]
    fn a_round_trip_keeps_every_value() {
        let config = demo();
        let options = config.default_options();
        let text = export_text(&config, &options);
        assert!(text.starts_with("// Sigil performance settings"));
        let read = parse(&text);
        assert!(read.skipped == 0);
        let written = effective(&config, &options);
        assert_eq!(read.entries.len(), written.len());
        for (key, value) in written {
            assert_eq!(read.get(&key), Some(value.as_str()), "{key}");
        }
    }

    #[test]
    fn edits_win_over_the_author_value() {
        let config = demo();
        let mut options = config.default_options();
        let (key, author) = config
            .body_convars()
            .next()
            .expect("demo config has convars")
            .clone();
        options.edits.insert(key.clone(), "999".into());
        let text = export_text(&config, &options);
        assert!(text.contains(&format!("{key} \"999\"")));
        assert!(!text.contains(&format!("{key} \"{author}\"")));
    }

    #[test]
    fn only_the_convars_section_of_a_gameinfo_is_read() {
        let text = concat!(
            "GameInfo\n{\n\tConVars\n\t{\n",
            "\t\tr_shadows \"0\"\n",
            "\t\tfps_max \"240\" // capped\n",
            "\t}\n",
            "\tRenderSystem\n\t{\n\t\tIndexBufferPoolSizeMB \"32\"\n\t}\n",
            "}\n",
        );
        let read = parse(text);
        assert_eq!(read.entries.len(), 2);
        assert_eq!(read.get("r_shadows"), Some("0"));
        assert_eq!(read.get("fps_max"), Some("240"));
        assert_eq!(read.get("IndexBufferPoolSizeMB"), None);
    }

    #[test]
    fn odd_lines_are_skipped_not_guessed() {
        let read = parse("{\n// note\nkey-with-hyphen 1\nlonely\nok 1\n");
        assert_eq!(read.entries, vec![("ok".to_string(), "1".to_string())]);
        assert_eq!(read.skipped, 2);
    }
}
