//! The console name a custom map's GameBanana description gives: `maps/<name>.vpk`, or a
//! `map <name>` command. Descriptions are HTML, so the text is flattened first: block tags end a
//! line, an opening `<code>` or `<pre>` reads as an opening backtick (an author who writes
//! `<code>map streetball</code>` is quoting the command), and every other tag is a space.
//!
//! A `map <name>` candidate written as a command wins: quoted or inside code, after "type",
//! "typing", "command" or "run", or alone on its line. Without one, a candidate that looks like a
//! map identifier (an underscore or a digit in it) is the answer. Prose such as "this map
//! features" is neither and never matches.

use super::valid_map_name;

/// Find the console name in a map description. Anything that does not look like a map name is
/// ignored.
pub fn console_map_name(description: &str) -> Option<String> {
    let text = description_text(description);
    vpk_path_name(&text).or_else(|| map_command_name(&text))
}

/// `maps/<name>.vpk` anywhere in the text.
fn vpk_path_name(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("maps/") {
        let start = from + at + "maps/".len();
        let name: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if valid_map_name(&name) && lower[start + name.len()..].starts_with(".vpk") {
            return Some(name);
        }
        from = start;
    }
    None
}

/// The best `map <name>` (or `map_<name>`) candidate.
fn map_command_name(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut identifier = None;
    let mut from = 0;
    while let Some(at) = lower[from..].find("map") {
        let start = from + at;
        from = start + 3;
        // A word of its own: "bitmap" and "maps" are not the command.
        if start > 0 && is_word(bytes[start - 1]) {
            continue;
        }
        let mut cursor = start + 3;
        let underscore = bytes.get(cursor) == Some(&b'_');
        if underscore {
            cursor += 1;
        } else {
            let gap = lower[cursor..]
                .bytes()
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
            if gap == 0 {
                continue;
            }
            cursor += gap;
        }
        let name: String = lower[cursor..]
            .bytes()
            .take_while(|b| is_word(*b))
            .map(char::from)
            .collect();
        let starts_with_letter = name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic());
        if !starts_with_letter || name.len() < 3 || !valid_map_name(&name) {
            continue;
        }
        // "map_dl_bearpit" stands for "map dl_bearpit"; "map_editor" is just a word.
        if underscore && !name.contains('_') {
            continue;
        }
        let end = cursor + name.len();
        let line_start = lower[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = lower[end..].find('\n').map_or(lower.len(), |i| end + i);
        let before = &lower[line_start..start];
        let after = &lower[end..line_end];
        if quoted(before) || instructed(before) || alone(before, after) {
            return Some(name);
        }
        if identifier.is_none() && name.bytes().any(|b| b == b'_' || b.is_ascii_digit()) {
            identifier = Some(name);
        }
    }
    identifier
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The text before the command ends in an opening quote or backtick.
fn quoted(before: &str) -> bool {
    before
        .trim_end()
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '"' | '\'' | '`' | '\u{2018}' | '\u{2019}' | '\u{201c}' | '\u{201d}'))
}

/// "type map x", "run the command map x", "typing: map x": an instruction word, at most one
/// other word, then the command.
fn instructed(before: &str) -> bool {
    let words: Vec<&str> = before
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let verb = |word: &&str| matches!(*word, "type" | "typing" | "command" | "run");
    words.last().is_some_and(verb) || (words.len() >= 2 && verb(&words[words.len() - 2]))
}

/// The command stands on its own line, give or take bullets, numbers and punctuation.
fn alone(before: &str, after: &str) -> bool {
    before.chars().all(|c| !c.is_alphabetic())
        && after.chars().all(|c| !c.is_alphanumeric())
}

/// The description as plain text with its line breaks.
pub fn description_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let block = matches!(
            name.as_str(),
            "br" | "p" | "li" | "ol" | "ul" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" | "blockquote"
        );
        out.push_str(if block {
            "\n"
        } else if !closing && matches!(name.as_str(), "code" | "pre") {
            "`"
        } else {
            " "
        });
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> Option<String> {
        console_map_name(text)
    }

    #[test]
    fn a_command_inside_code_tags_is_read() {
        assert_eq!(name("Then enter <code>map harbor_night</code> in the console."), Some("harbor_night".into()));
        assert_eq!(name("Then enter <code>map lighthouse</code> to begin."), Some("lighthouse".into()));
        assert_eq!(
            name("Load it with the console command:</b><br><code data-x=\"1\">map old_quarry</code>"),
            Some("old_quarry".into())
        );
        assert_eq!(name("<pre>map dockyard</pre>"), Some("dockyard".into()));
    }

    #[test]
    fn the_same_words_without_code_are_prose() {
        assert_eq!(name("Then enter map lighthouse to begin."), None);
        assert_eq!(name("Switch on <code>sv_cheats</code> map borders are open."), None);
        assert_eq!(name("This map features three lanes and a vault."), None);
        assert_eq!(name("<br>Map has long jumps<br>"), None);
    }

    #[test]
    fn quotes_instructions_and_own_lines_mark_a_command() {
        assert_eq!(name("Type \"<b>map tower_run</b>\" twice."), Some("tower_run".into()));
        assert_eq!(name("press F7 and type map ridge into the console"), Some("ridge".into()));
        assert_eq!(name("notes<br><br>map arena<br><br>have fun"), Some("arena".into()));
        assert_eq!(name("Open the console and type:\n\nmap harbor_night\n"), Some("harbor_night".into()));
    }

    #[test]
    fn an_identifier_is_the_fallback_and_map_underscore_counts() {
        assert_eq!(name("Use <span>map dl_1v1_c4</span> while it updates"), Some("dl_1v1_c4".into()));
        assert_eq!(name("then reload by typing map_dl_pit."), Some("dl_pit".into()));
        assert_eq!(name("a map_editor build"), None);
        assert_eq!(name("a bitmap pack"), None);
    }

    #[test]
    fn the_vpk_path_wins() {
        assert_eq!(
            name("Install into citadel/maps/my_arena.vpk and then run <code>map other</code>."),
            Some("my_arena".into())
        );
        assert_eq!(name("maps/not a name.vpk"), None);
        assert_eq!(name("a skin pack with no map"), None);
    }

    #[test]
    fn entities_and_tags_flatten() {
        assert_eq!(description_text("a&nbsp;<b>b</b><br>c &amp; d"), "a  b \nc & d");
    }
}
