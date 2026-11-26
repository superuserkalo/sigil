//! The Steam accounts this computer has signed in with, from `<Steam>/config/loginusers.vdf`.
//!
//! Read only, local only. Settings uses it to link a Steam account to Sigil (Stats opens on it,
//! the profile link opens in the browser) without asking the user to paste an id.

use std::fs;
use std::path::{Path, PathBuf};

/// SteamID64 = account id + this.
pub const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamUser {
    pub steam_id64: u64,
    pub account_id: u32,
    pub persona: String,
    pub most_recent: bool,
}

impl SteamUser {
    pub fn profile_url(&self) -> String {
        profile_url(self.account_id)
    }
}

pub fn account_to_id64(account_id: u32) -> u64 {
    STEAM_ID64_BASE + account_id as u64
}

pub fn id64_to_account(id64: u64) -> Option<u32> {
    id64.checked_sub(STEAM_ID64_BASE)
        .and_then(|a| u32::try_from(a).ok())
}

pub fn profile_url(account_id: u32) -> String {
    format!(
        "https://steamcommunity.com/profiles/{}",
        account_to_id64(account_id)
    )
}

/// Tokens of a text VDF file: quoted strings and braces.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => out.push(c.to_string()),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                s.push(next);
                            }
                        }
                        '"' => break,
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Parse loginusers.vdf text. The most recent account comes first.
pub fn parse(text: &str) -> Vec<SteamUser> {
    let t = tokens(text);
    let mut users = Vec::new();
    // Shape: "users" { "<id64>" { "Key" "Value" ... } ... }
    let Some(start) = t.iter().position(|s| s.eq_ignore_ascii_case("users")) else {
        return users;
    };
    let mut i = start + 1;
    if t.get(i).map(String::as_str) != Some("{") {
        return users;
    }
    i += 1;
    while i < t.len() && t[i] != "}" {
        let id = t[i].clone();
        i += 1;
        if t.get(i).map(String::as_str) != Some("{") {
            continue;
        }
        i += 1;
        let mut persona = String::new();
        let mut recent = false;
        while i + 1 < t.len() && t[i] != "}" {
            let (key, value) = (&t[i], &t[i + 1]);
            if value == "{" {
                // A nested block we do not read: skip to its end.
                let mut depth = 0;
                i += 1;
                while i < t.len() {
                    if t[i] == "{" {
                        depth += 1;
                    } else if t[i] == "}" {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            if key.eq_ignore_ascii_case("PersonaName") {
                persona = value.clone();
            } else if key.eq_ignore_ascii_case("MostRecent") {
                recent = value == "1";
            }
            i += 2;
        }
        i += 1;
        if let Ok(id64) = id.parse::<u64>()
            && let Some(account_id) = id64_to_account(id64)
        {
            users.push(SteamUser {
                steam_id64: id64,
                account_id,
                persona: if persona.is_empty() {
                    account_id.to_string()
                } else {
                    persona
                },
                most_recent: recent,
            });
        }
    }
    users.sort_by_key(|u| !u.most_recent);
    users
}

/// Every account in the first Steam root that has a loginusers.vdf.
pub fn detect(steam_roots: &[PathBuf]) -> Vec<SteamUser> {
    for root in steam_roots {
        if let Some(users) = read(&root.join("config").join("loginusers.vdf"))
            && !users.is_empty()
        {
            return users;
        }
    }
    Vec::new()
}

/// The avatar Steam itself keeps for an account signed in on this PC
/// (`config/avatarcache/<steamid64>.png`), so the Stats start page can show the player's own
/// picture before anything is asked of the network. `None` when Steam has no copy.
pub fn cached_avatar(steam_roots: &[PathBuf], steam_id64: u64) -> Option<PathBuf> {
    steam_roots
        .iter()
        .map(|root| root.join("config").join("avatarcache").join(format!("{steam_id64}.png")))
        .find(|path| path.is_file())
}

fn read(path: &Path) -> Option<Vec<SteamUser>> {
    fs::read_to_string(path).ok().map(|text| parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
"users"
{
	"76561197960287930"
	{
		"AccountName"		"gaben"
		"PersonaName"		"Gabe \"N\""
		"MostRecent"		"0"
	}
	// a comment
	"76561198041486192"
	{
		"AccountName"		"player"
		"PersonaName"		"Player One"
		"RememberPassword"		"1"
		"MostRecent"		"1"
		"Extra" { "Nested" "1" }
	}
	"not-a-number" { "PersonaName" "x" }
}
"#;

    #[test]
    fn reads_accounts_most_recent_first() {
        let users = parse(SAMPLE);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].persona, "Player One");
        assert!(users[0].most_recent);
        assert_eq!(users[0].account_id, 81_220_464);
        assert_eq!(users[1].persona, "Gabe \"N\"");
        assert_eq!(users[1].account_id, 22_202);
    }

    #[test]
    fn id_conversions_round_trip() {
        assert_eq!(id64_to_account(account_to_id64(81_220_464)), Some(81_220_464));
        assert_eq!(id64_to_account(5), None);
        assert!(profile_url(1).ends_with("/76561197960265729"));
    }

    #[test]
    fn garbage_gives_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("\"users\" \"x\"").is_empty());
    }
}
