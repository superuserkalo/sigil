//! Launch options and autoexec commands the game no longer reads.
//!
//! Valve renames and removes console variables between Deadlock builds. A player who
//! copied an autoexec or a launch options string from an old guide ends up with lines
//! the game silently ignores. The checks here find those lines and name the replacement
//! when the game renamed the command.
//!
//! The list in `data/obsolete-commands.json` only holds entries proven against Valve's
//! convar dumps: the command was in an earlier dump and is absent from a later one, or
//! the dump ships the new name after dropping the old. Every entry records its source.
//! A launch flag belongs in the same list with its leading dash; none are included yet
//! because no flag removal could be proven from Valve's dumps or patch notes.
//!
//! Both checks are pure: they read text and return findings. Nothing is written, no
//! network call is made and no game folder is touched. `line` is 1 based for autoexec
//! text and 0 for a launch options string, which Steam reads as one line.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::autoexec::{CfgLineKind, classify_line, parse_cfg};

const CATALOG: &str = include_str!("../data/obsolete-commands.json");
const CATALOG_SCHEMA: u32 = 1;

/// What happened to a command the game used to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    /// The game no longer registers the command. `replacement` is None.
    Removed,
    /// The game renamed the command. `replacement` is the name to use now.
    Renamed,
}

/// One obsolete command found in launch options or autoexec text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandFinding {
    /// 1 based line in autoexec text. 0 for a launch options string.
    pub line: usize,
    /// The command as the user spelled it, lowercased. A launch flag keeps its
    /// leading dash; a `+` launch option drops the `+`.
    pub command: String,
    pub kind: FindingKind,
    /// The command that replaced it, when the game renamed it.
    pub replacement: Option<String>,
    /// One sentence on why the command no longer works.
    pub note: String,
}

/// Find obsolete commands in a Steam launch options string.
///
/// The string is split the way Steam and Source 2 read it: `+command value` pairs,
/// `-flag` tokens, quoted values and a trailing `//` comment. A quoted token is a value,
/// never a command. Every finding has `line` 0.
pub fn check_launch_options(text: &str) -> Vec<CommandFinding> {
    let tokens = tokenize(text);
    let mut findings = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let (token, quoted) = &tokens[index];
        index += 1;

        if *quoted || token.is_empty() {
            continue;
        }

        if let Some(command) = token.strip_prefix('+') {
            if command.is_empty() {
                continue;
            }
            if let Some((value, value_quoted)) = tokens.get(index) {
                // A plain token after `+command` is its value. A token starting with
                // a sign belongs to the next option, as Source 2 parses it.
                if !value_quoted && !value.starts_with(['+', '-']) {
                    index += 1;
                }
            }
            findings.extend(finding_for(command, 0));
        } else if token.len() > 1 && token.starts_with('-') {
            findings.extend(finding_for(token, 0));
        }
    }

    findings
}

/// Find obsolete commands in autoexec.cfg text.
///
/// The text goes through [`crate::autoexec::parse_cfg`], so a byte order mark and CRLF
/// lines are handled the same way as the editor. Comment lines, `//` tails and quoted
/// values are ignored, and a line with several `;` statements is checked statement by
/// statement. Every finding carries its 1 based line.
pub fn check_autoexec(text: &str) -> Vec<CommandFinding> {
    let document = parse_cfg(text);
    let mut findings = Vec::new();

    for (index, line) in document.lines.iter().enumerate() {
        if classify_line(&line.text) != CfgLineKind::Command {
            continue;
        }

        for statement in split_statements(&line.text) {
            let tokens = tokenize(statement);
            let Some((command, _)) = tokens.first() else {
                continue;
            };

            if command.is_empty() {
                continue;
            }

            findings.extend(finding_for(command, index + 1));
        }
    }

    findings
}

// --- catalog -------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CatalogFile {
    schema: u32,
    #[serde(default)]
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct CatalogEntry {
    name: String,
    kind: FindingKind,
    #[serde(default)]
    replacement: Option<String>,
    note: String,
    #[serde(rename = "source")]
    _source: String,
}

struct Rule {
    kind: FindingKind,
    replacement: Option<String>,
    note: String,
}

fn rules() -> &'static HashMap<String, Rule> {
    static RULES: OnceLock<HashMap<String, Rule>> = OnceLock::new();

    RULES.get_or_init(|| {
        let file: CatalogFile =
            serde_json::from_str(CATALOG).expect("bundled obsolete command list parses");
        assert_eq!(
            file.schema, CATALOG_SCHEMA,
            "bundled obsolete command list schema"
        );

        file.entries
            .into_iter()
            .map(|entry| {
                (
                    entry.name.to_ascii_lowercase(),
                    Rule {
                        kind: entry.kind,
                        replacement: entry.replacement,
                        note: entry.note,
                    },
                )
            })
            .collect()
    })
}

fn finding_for(command: &str, line: usize) -> Option<CommandFinding> {
    let name = command.to_ascii_lowercase();
    let rule = rules().get(&name)?;

    Some(CommandFinding {
        line,
        command: name,
        kind: rule.kind,
        replacement: rule.replacement.clone(),
        note: rule.note.clone(),
    })
}

// --- syntax --------------------------------------------------------------------------------

/// One whitespace separated token. `quoted` is true when any part of it was inside
/// double quotes, which makes it a value rather than a command.
fn tokenize(text: &str) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '"' => {
                quoted = true;
                in_quotes = !in_quotes;
            }
            '/' if !in_quotes && chars.peek() == Some(&'/') => break,
            value if value.is_whitespace() && !in_quotes => {
                if !current.is_empty() || quoted {
                    tokens.push((std::mem::take(&mut current), quoted));
                    quoted = false;
                }
            }
            value => current.push(value),
        }
    }

    if !current.is_empty() || quoted {
        tokens.push((current, quoted));
    }

    tokens
}

/// Split a cfg line at `;` outside quotes and before a `//` comment. Each statement is
/// returned without its separator; empty statements are harmless because they tokenise
/// to nothing.
fn split_statements(line: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let bytes = line.as_bytes();

    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b'/' if !in_quotes && bytes.get(index + 1) == Some(&b'/') => {
                statements.push(&line[start..index]);
                return statements;
            }
            b';' if !in_quotes => {
                statements.push(&line[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    statements.push(&line[start..]);
    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(findings: &[CommandFinding]) -> Vec<(&str, usize)> {
        findings
            .iter()
            .map(|finding| (finding.command.as_str(), finding.line))
            .collect()
    }

    #[test]
    fn autoexec_reports_lines_and_skips_comments_and_quoted_values() {
        let text = "\
fps_max 144\r\n\
citadel_unit_status_use_new 1\r\n\
CITADEL_UNIT_STATUS_USE_NEW 0\r\n\
citadel_damage_text_lifetime 80\r\n\
// citadel_unit_status_use_new 1\r\n\
echo \"citadel_unit_status_use_new\"\r\n\
bind f \"citadel_unit_status_use_new 1\"\r\n\
citadel_unit_status_use_new 1 // old health bar switch\r\n";

        let findings = check_autoexec(text);

        assert_eq!(
            names(&findings),
            vec![
                ("citadel_unit_status_use_new", 2),
                ("citadel_unit_status_use_new", 3),
                ("citadel_damage_text_lifetime", 4),
                ("citadel_unit_status_use_new", 8),
            ]
        );
        assert_eq!(findings[0].kind, FindingKind::Removed);
        assert_eq!(findings[0].replacement, None);
        assert!(findings[0].note.contains("6711"), "{:?}", findings[0]);
        assert_eq!(findings[2].kind, FindingKind::Renamed);
        assert_eq!(
            findings[2].replacement.as_deref(),
            Some("citadel_damage_text_lifetime_new")
        );
    }

    #[test]
    fn autoexec_checks_each_statement_and_handles_a_byte_order_mark() {
        let text = "\u{feff}fps_max 144; citadel_damage_text_lifetime 100\r\n\
r_shadows 1;citadel_unit_status_use_v2 1\r\n\
alias old \"citadel_unit_status_use_new 1; fps_max 60\"\r\n";

        let findings = check_autoexec(text);

        assert_eq!(
            names(&findings),
            vec![
                ("citadel_damage_text_lifetime", 1),
                ("citadel_unit_status_use_v2", 2),
            ]
        );
        assert_eq!(
            findings[0].replacement.as_deref(),
            Some("citadel_damage_text_lifetime_new")
        );
        assert_eq!(
            findings[0].note,
            "The accumulated damage text lifetime convar replaced it after build 6711."
        );
    }

    #[test]
    fn launch_options_parse_plus_pairs_and_flags() {
        let text = "+citadel_unit_status_use_new 1 -novid -dx11 +fps_max 240 \
+setinfo name \"citadel_unit_status_use_new\" +CITADEL_UNIT_STATUS_USE_NEW 0";

        let findings = check_launch_options(text);

        assert_eq!(
            names(&findings),
            vec![
                ("citadel_unit_status_use_new", 0),
                ("citadel_unit_status_use_new", 0),
            ]
        );
        assert!(findings.iter().all(|finding| finding.line == 0));
        assert!(
            findings
                .iter()
                .all(|finding| finding.kind == FindingKind::Removed)
        );
    }

    #[test]
    fn launch_options_ignore_flags_bare_names_and_comments() {
        let text = "-citadel_unit_status_use_new citadel_unit_status_use_new \
+panorama_max_fps 144 // citadel_unit_status_use_new 1";

        let findings = check_launch_options(text);

        assert_eq!(names(&findings), vec![("panorama_max_fps", 0)]);
    }

    #[test]
    fn a_renamed_launch_option_names_its_replacement() {
        let findings = check_launch_options("+citadel_damage_text_lifetime 80");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 0);
        assert_eq!(findings[0].kind, FindingKind::Renamed);
        assert_eq!(
            findings[0].replacement.as_deref(),
            Some("citadel_damage_text_lifetime_new")
        );
    }

    #[test]
    fn a_comment_marker_inside_quotes_does_not_start_a_comment() {
        let launch = "+setinfo name \"a//b\" +citadel_unit_status_use_new 1";
        assert_eq!(
            names(&check_launch_options(launch)),
            vec![("citadel_unit_status_use_new", 0)]
        );

        let autoexec = "echo \"a//b\"; citadel_unit_status_use_new 1\n";
        assert_eq!(
            names(&check_autoexec(autoexec)),
            vec![("citadel_unit_status_use_new", 1)]
        );
    }

    #[test]
    fn known_good_commands_are_not_flagged() {
        let autoexec = "\
fps_max 144
r_shadows 1
r_low_latency 2
bind \"f\" \"toggleconsole\"
exec autoexec
citadel_crosshair_color_r 255
";
        assert!(check_autoexec(autoexec).is_empty());

        let launch_options = "-novid -dx11 -vulkan -condebug -console \
+exec autoexec +connect 203.0.113.10:27015 +fps_max 240";
        assert!(check_launch_options(launch_options).is_empty());
    }

    #[test]
    fn empty_text_has_no_findings() {
        assert!(check_autoexec("").is_empty());
        assert!(check_launch_options("   ").is_empty());
    }

    #[test]
    fn the_bundled_list_is_well_formed() {
        let rules = rules();

        assert!(rules.len() > 50, "the list lost entries");
        assert!(rules.contains_key("citadel_unit_status_use_new"));
        assert!(rules.contains_key("citadel_damage_text_lifetime"));

        for (name, rule) in rules {
            assert_eq!(name, &name.to_ascii_lowercase());
            assert!(!name.is_empty());
            assert!(!name.contains(char::is_whitespace));
            assert!(!rule.note.trim().is_empty(), "{name} has no note");
            assert!(!rule.note.contains('\u{2014}'), "{name} has an em dash");

            match rule.kind {
                FindingKind::Renamed => {
                    let replacement = rule
                        .replacement
                        .as_deref()
                        .unwrap_or_else(|| panic!("{name} is renamed without a replacement"));
                    assert!(
                        !rules.contains_key(replacement),
                        "{name} points at removed command {replacement}"
                    );
                }
                FindingKind::Removed => {
                    assert!(
                        rule.replacement.is_none(),
                        "{name} is removed but has a replacement"
                    );
                }
            }
        }
    }
}
