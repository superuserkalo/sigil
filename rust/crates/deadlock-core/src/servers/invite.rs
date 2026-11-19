//! Sigil invites (ADR 0015, second amendment): one line a host copies into Discord or a chat,
//! carrying where to join and the GameBanana ids of the server's custom content, so a joining
//! Sigil installs that content first and then connects.
//!
//! ```text
//! sigil-invite:v1:<target>:<p|o>:<mod id>/<file id>,<mod id>/<file id>
//! sigil-invite:v1:[A:1:570253314:51555]:o:715100/1414905
//! sigil-invite:v1:203.0.113.7:27067:p:-
//! ```
//!
//! - `<target>` is a Steam networking code (`[A:1:<n>:<n>]`, relay mode and the in-game host)
//!   or an `ip:port` (`[v6]:port` for IPv6).
//! - `p` means the server asks for a join password, `o` that it is open. The password itself
//!   is never in the invite.
//! - Items are GameBanana mod and file ids only, never URLs; `-` when there are none. The
//!   joiner downloads them through the normal GameBanana install path.
//!
//! [`parse_invite`] refuses anything that is not exactly this shape. Pure: no I/O.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::install::InstallJournalEntry;

/// What every invite starts with.
pub const INVITE_PREFIX: &str = "sigil-invite:v1:";
/// The most content items one invite carries.
pub const MAX_INVITE_ITEMS: usize = 32;
/// The longest invite accepted.
pub const MAX_INVITE_LEN: usize = 1024;
/// GameBanana ids are well under this many digits.
const MAX_ID_DIGITS: usize = 12;

/// Where the invite joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum InviteTarget {
    /// A Steam networking code, `[A:1:<n>:<n>]`: relay mode or the in-game host.
    Steam(String),
    /// An address and port.
    Address(SocketAddr),
}

impl InviteTarget {
    /// The text after `connect` in the game console.
    pub fn connect_text(&self) -> String {
        match self {
            Self::Steam(code) => code.clone(),
            Self::Address(addr) => addr.to_string(),
        }
    }

    /// The line friends paste in the console (F7). With a password it is set first, the way the
    /// Deadworks launcher writes its connect command.
    pub fn console_line(&self, password: Option<&str>) -> Result<String> {
        let connect = format!("connect {}", self.connect_text());
        match password.map(str::trim).filter(|p| !p.is_empty()) {
            None => Ok(connect),
            Some(password) => {
                let ok = password.chars().count() <= 64
                    && password.chars().all(|c| !c.is_control() && !matches!(c, '"' | ';' | ' '));
                if !ok {
                    return Err(Error::msg(
                        "The password can have up to 64 characters, without spaces, quotes or semicolons.",
                    ));
                }
                Ok(format!("password \"{password}\"; {connect}"))
            }
        }
    }
}

/// One GameBanana file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteItem {
    pub mod_id: u64,
    pub file_id: u64,
}

impl InviteItem {
    /// The download link the GameBanana install path takes for this exact file.
    pub fn download_target(&self) -> String {
        format!("https://gamebanana.com/dl/{}", self.file_id)
    }
}

/// A parsed invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub target: InviteTarget,
    /// The server asks for a join password.
    pub password: bool,
    pub items: Vec<InviteItem>,
}

impl Invite {
    /// An invite, its items sorted and each listed once, at most [`MAX_INVITE_ITEMS`].
    pub fn new(target: InviteTarget, password: bool, items: impl IntoIterator<Item = InviteItem>) -> Result<Self> {
        if let InviteTarget::Steam(code) = &target
            && !valid_steam_code(code)
        {
            return Err(Error::msg(format!("{code} is not a Steam join code.")));
        }
        if let InviteTarget::Address(addr) = &target {
            check_address(*addr)?;
        }
        let mut items: Vec<InviteItem> = items.into_iter().filter(|i| i.mod_id > 0 && i.file_id > 0).collect();
        items.sort();
        items.dedup();
        if items.len() > MAX_INVITE_ITEMS {
            return Err(Error::msg(format!(
                "An invite carries at most {MAX_INVITE_ITEMS} GameBanana files. Remove some content from the server."
            )));
        }
        Ok(Self { target, password, items })
    }
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let items = if self.items.is_empty() {
            "-".to_string()
        } else {
            self.items
                .iter()
                .map(|i| format!("{}/{}", i.mod_id, i.file_id))
                .collect::<Vec<_>>()
                .join(",")
        };
        write!(
            f,
            "{INVITE_PREFIX}{}:{}:{items}",
            self.target.connect_text(),
            if self.password { 'p' } else { 'o' }
        )
    }
}

/// `[A:1:<digits>:<digits>]`, the shape `connect` takes for a Steam networking address.
pub fn valid_steam_code(code: &str) -> bool {
    let Some(inner) = code.strip_prefix("[A:").and_then(|c| c.strip_suffix(']')) else {
        return false;
    };
    let fields: Vec<&str> = inner.split(':').collect();
    fields.len() == 3 && fields.iter().all(|f| !f.is_empty() && f.len() <= 20 && f.bytes().all(|b| b.is_ascii_digit()))
}

fn check_address(addr: SocketAddr) -> Result<()> {
    let ip = match addr.ip() {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        ip => ip,
    };
    let bad = ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, IpAddr::V4(v4) if v4.is_broadcast())
        || addr.port() == 0;
    if bad {
        return Err(Error::msg(format!("{addr} is not an address a server can have.")));
    }
    Ok(())
}

fn parse_id(text: &str) -> Option<u64> {
    let ok = !text.is_empty() && text.len() <= MAX_ID_DIGITS && text.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| text.parse().ok()).flatten().filter(|id| *id > 0)
}

const MALFORMED: &str = "This is not a Sigil invite. Ask your friend to copy it again from Sigil's Host page.";

/// Read an invite. Surrounding whitespace is ignored; anything else that is not exactly the
/// documented shape is refused, with nothing installed or opened.
pub fn parse_invite(text: &str) -> Result<Invite> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::msg("Paste the invite your friend copied from Sigil."));
    }
    if text.len() > MAX_INVITE_LEN || !text.is_ascii() || text.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        return Err(Error::msg(MALFORMED));
    }
    let Some(rest) = text.strip_prefix(INVITE_PREFIX) else {
        if text.starts_with("sigil-invite:") {
            return Err(Error::msg("This invite is from a newer Sigil. Update Sigil, then paste it again."));
        }
        return Err(Error::msg(MALFORMED));
    };
    let mut parts = rest.rsplitn(3, ':');
    let (Some(items), Some(flag), Some(target)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(Error::msg(MALFORMED));
    };
    let password = match flag {
        "p" => true,
        "o" => false,
        _ => return Err(Error::msg(MALFORMED)),
    };
    let target = if target.starts_with("[A:") {
        if !valid_steam_code(target) {
            return Err(Error::msg(MALFORMED));
        }
        InviteTarget::Steam(target.to_string())
    } else {
        let addr: SocketAddr = target.parse().map_err(|_| Error::msg(MALFORMED))?;
        InviteTarget::Address(addr)
    };
    let mut list = Vec::new();
    if items != "-" {
        for item in items.split(',') {
            let (mod_id, file_id) = item.split_once('/').ok_or_else(|| Error::msg(MALFORMED))?;
            let (Some(mod_id), Some(file_id)) = (parse_id(mod_id), parse_id(file_id)) else {
                return Err(Error::msg(MALFORMED));
            };
            let item = InviteItem { mod_id, file_id };
            if list.contains(&item) {
                return Err(Error::msg(MALFORMED));
            }
            list.push(item);
        }
        if list.len() > MAX_INVITE_ITEMS {
            return Err(Error::msg(MALFORMED));
        }
    }
    Invite::new(target, password, list).map_err(|_| Error::msg(MALFORMED))
}

/// The GameBanana file id in a download link (`/dl/<id>` or `/mmdl/<id>` on gamebanana.com).
pub fn gamebanana_file_id(url: &str) -> Option<u64> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let host = host.to_ascii_lowercase();
    if host != "gamebanana.com" && !host.ends_with(".gamebanana.com") {
        return None;
    }
    let path = path.split(['?', '#']).next()?.trim_end_matches('/');
    let (kind, id) = path.rsplit_once('/')?;
    let kind = kind.rsplit('/').next()?;
    matches!(kind, "dl" | "mmdl").then(|| parse_id(id)).flatten()
}

/// The GameBanana file an install journal entry installed, when the journal knows both ids.
/// `mod_id_for_slot` names the mod for installs the journal labels by link only (Sigil's
/// library knows it).
pub fn journal_item(entry: &InstallJournalEntry, mod_id_for_slot: &dyn Fn(u16) -> Option<u64>) -> Option<InviteItem> {
    let file_id = entry.download_url.as_deref().and_then(gamebanana_file_id)?;
    let mod_id = crate::mod_updates::journal_mod_id(entry).or_else(|| mod_id_for_slot(entry.slot))?;
    Some(InviteItem { mod_id, file_id })
}

/// The GameBanana file for the VPK with this SHA-256 (the install journal's `vpkSha256`), from
/// the newest journal entry that wrote it.
pub fn item_for_sha256(
    journal: &[InstallJournalEntry],
    sha256: &str,
    mod_id_for_slot: &dyn Fn(u16) -> Option<u64>,
) -> Option<InviteItem> {
    journal
        .iter()
        .rev()
        .filter(|e| e.vpk_sha256.eq_ignore_ascii_case(sha256))
        .find_map(|e| journal_item(e, mod_id_for_slot))
}

/// The GameBanana file in addon slot `slot`, from the newest journal entry for that slot whose
/// VPK has `vpk_bytes` (the file on disk now).
pub fn item_for_slot(
    journal: &[InstallJournalEntry],
    slot: u16,
    vpk_bytes: u64,
    mod_id_for_slot: &dyn Fn(u16) -> Option<u64>,
) -> Option<InviteItem> {
    journal
        .iter()
        .rev()
        .find(|e| e.slot == slot && e.vpk_bytes == vpk_bytes)
        .and_then(|e| journal_item(e, mod_id_for_slot))
}

/// Whether the joiner already has this exact file installed: the newest journal entry of the
/// mod is this file and its VPK (on or off) is still in place.
pub fn item_installed(journal: &[InstallJournalEntry], item: &InviteItem, mod_id_for_slot: &dyn Fn(u16) -> Option<u64>) -> Option<u16> {
    journal
        .iter()
        .rev()
        .filter(|e| journal_item(e, mod_id_for_slot).is_some_and(|i| i.mod_id == item.mod_id))
        .take(1)
        .find(|e| {
            journal_item(e, mod_id_for_slot) == Some(*item)
                && (e.target.is_file() || disabled_name(&e.target).is_some_and(|p| p.is_file()))
        })
        .map(|e| e.slot)
}

fn disabled_name(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!("{name}.disabled")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(mod_id: u64, file_id: u64) -> InviteItem {
        InviteItem { mod_id, file_id }
    }

    #[test]
    fn an_invite_round_trips() {
        let relay = Invite::new(
            InviteTarget::Steam("[A:1:570253314:51555]".into()),
            false,
            [item(715100, 1414905), item(600009, 77), item(715100, 1414905)],
        )
        .unwrap();
        let text = relay.to_string();
        assert_eq!(text, "sigil-invite:v1:[A:1:570253314:51555]:o:600009/77,715100/1414905");
        assert_eq!(parse_invite(&format!("  {text}\n")).unwrap(), relay);

        let direct = Invite::new(InviteTarget::Address("203.0.113.7:27067".parse().unwrap()), true, []).unwrap();
        assert_eq!(direct.to_string(), "sigil-invite:v1:203.0.113.7:27067:p:-");
        assert_eq!(parse_invite(&direct.to_string()).unwrap(), direct);

        let v6 = Invite::new(InviteTarget::Address("[2001:db8::7]:27067".parse().unwrap()), false, [item(1, 2)]).unwrap();
        assert_eq!(v6.to_string(), "sigil-invite:v1:[2001:db8::7]:27067:o:1/2");
        assert_eq!(parse_invite(&v6.to_string()).unwrap(), v6);
    }

    #[test]
    fn malformed_invites_are_refused() {
        for bad in [
            "",
            "hello",
            "connect [A:1:5:5]",
            "sigil-invite:v1:",
            "sigil-invite:v1:[A:1:5:5]",
            "sigil-invite:v1:[A:1:5:5]:o",
            "sigil-invite:v1:[A:1:5:5]:x:-",
            "sigil-invite:v1:[A:1:5:5]:o:",
            "sigil-invite:v1:[A:1:5:5]:O:-",
            "SIGIL-INVITE:v1:[A:1:5:5]:o:-",
            "sigil-invite:v1:[A:1:5]:o:-",
            "sigil-invite:v1:[A:1:5:x]:o:-",
            "sigil-invite:v1:[U:1:5:5]:o:-",
            "sigil-invite:v1:[A:1:5:5:5]:o:-",
            "sigil-invite:v1:example.com:27067:o:-",
            "sigil-invite:v1:203.0.113.7:o:-",
            "sigil-invite:v1:203.0.113.7:0:o:-",
            "sigil-invite:v1:0.0.0.0:27067:o:-",
            "sigil-invite:v1:255.255.255.255:27067:o:-",
            "sigil-invite:v1:224.0.0.1:27067:o:-",
            "sigil-invite:v1:203.0.113.7:27067:o:1",
            "sigil-invite:v1:203.0.113.7:27067:o:1/",
            "sigil-invite:v1:203.0.113.7:27067:o:/2",
            "sigil-invite:v1:203.0.113.7:27067:o:0/2",
            "sigil-invite:v1:203.0.113.7:27067:o:1/0",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2,",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2,,3/4",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2,1/2",
            "sigil-invite:v1:203.0.113.7:27067:o:-1/2",
            "sigil-invite:v1:203.0.113.7:27067:o:+1/2",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2/3",
            "sigil-invite:v1:203.0.113.7:27067:o:1234567890123/2",
            "sigil-invite:v1:203.0.113.7:27067:o:https://gamebanana.com/dl/1",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2 3/4",
            "sigil-invite:v1:203.0.113.7:27067:o:1/2\u{200b}",
            "sigil-invite:v1:203.0.113.7:27067 :o:-",
            "sigil-invite:v1:[A:1:5:5]:o:-;quit",
        ] {
            assert!(parse_invite(bad).is_err(), "{bad:?}");
        }
        let many = (1..=MAX_INVITE_ITEMS as u64 + 1).map(|i| format!("{i}/{i}")).collect::<Vec<_>>().join(",");
        assert!(parse_invite(&format!("sigil-invite:v1:[A:1:5:5]:o:{many}")).is_err());
        let long = format!("sigil-invite:v1:[A:1:5:5]:o:{}", "1/1,".repeat(300));
        assert!(parse_invite(&long).is_err());
        let newer = parse_invite("sigil-invite:v2:[A:1:5:5]:o:-").unwrap_err().to_string();
        assert!(newer.contains("newer Sigil"), "{newer}");
    }

    #[test]
    fn the_console_line_sets_the_password_first() {
        let steam = InviteTarget::Steam("[A:1:5:6]".into());
        assert_eq!(steam.console_line(None).unwrap(), "connect [A:1:5:6]");
        assert_eq!(steam.console_line(Some("  ")).unwrap(), "connect [A:1:5:6]");
        assert_eq!(steam.console_line(Some("hunter2")).unwrap(), "password \"hunter2\"; connect [A:1:5:6]");
        for bad in ["a\"b", "a;quit", "a b", "a\nb"] {
            assert!(steam.console_line(Some(bad)).is_err(), "{bad:?}");
        }
        let addr = InviteTarget::Address("[2001:db8::1]:27067".parse().unwrap());
        assert_eq!(addr.console_line(None).unwrap(), "connect [2001:db8::1]:27067");
    }

    #[test]
    fn file_ids_come_from_gamebanana_download_links_only() {
        assert_eq!(gamebanana_file_id("https://gamebanana.com/dl/1414905"), Some(1_414_905));
        assert_eq!(gamebanana_file_id("https://gamebanana.com/mmdl/77?x=1"), Some(77));
        assert_eq!(gamebanana_file_id("https://files.gamebanana.com/dl/77/"), Some(77));
        assert_eq!(gamebanana_file_id("https://gamebanana.com/mods/600009"), None);
        assert_eq!(gamebanana_file_id("https://evil.example/dl/5"), None);
        assert_eq!(gamebanana_file_id("https://gamebanana.com.evil.example/dl/5"), None);
        assert_eq!(gamebanana_file_id("https://gamebanana.com/dl/x"), None);
        assert_eq!(item(5, 9).download_target(), "https://gamebanana.com/dl/9");
    }

    fn entry(slot: u16, label: &str, url: Option<&str>, sha: &str, bytes: u64, target: &std::path::Path) -> InstallJournalEntry {
        InstallJournalEntry {
            schema: 1,
            plan_id: format!("p{slot}{sha}"),
            installed_at: "2026-11-19T00:00:00Z".into(),
            source: label.into(),
            source_label: label.into(),
            download_url: url.map(str::to_string),
            archive: "a.zip".into(),
            archive_sha256: String::new(),
            slot,
            file: format!("pak{slot:02}_dir.vpk"),
            target: target.to_path_buf(),
            chunk_files: Vec::new(),
            entries: 1,
            bytes,
            vpk_bytes: bytes,
            vpk_sha256: sha.into(),
            backups: Vec::new(),
            removed_chunks: Vec::new(),
            removed_files: Vec::new(),
            gameinfo_repaired: false,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn the_journal_names_the_gamebanana_file() {
        let root = std::env::temp_dir().join(format!("sigil-invite-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let on = root.join("pak03_dir.vpk");
        std::fs::write(&on, b"x").unwrap();
        let off = root.join("pak04_dir.vpk");
        std::fs::write(root.join("pak04_dir.vpk.disabled"), b"x").unwrap();
        let gone = root.join("pak05_dir.vpk");
        let library = |slot: u16| (slot == 4).then_some(900);
        let journal = vec![
            entry(3, "gamebanana:715100", Some("https://gamebanana.com/dl/11"), "aa", 10, &on),
            // Installed by link: the mod id comes from Sigil's library.
            entry(4, "https://gamebanana.com/dl/22", Some("https://gamebanana.com/dl/22"), "bb", 20, &off),
            entry(5, "gamebanana:600009", Some("https://gamebanana.com/dl/33"), "cc", 30, &gone),
            entry(6, "/tmp/local.zip", None, "dd", 40, &gone),
        ];
        assert_eq!(item_for_sha256(&journal, "AA", &library), Some(item(715100, 11)));
        assert_eq!(item_for_sha256(&journal, "bb", &library), Some(item(900, 22)));
        assert_eq!(item_for_sha256(&journal, "dd", &library), None, "a local archive has no GameBanana file");
        assert_eq!(item_for_slot(&journal, 3, 10, &library), Some(item(715100, 11)));
        assert_eq!(item_for_slot(&journal, 3, 11, &library), None, "the file changed since");
        assert_eq!(item_installed(&journal, &item(715100, 11), &library), Some(3));
        assert_eq!(item_installed(&journal, &item(900, 22), &library), Some(4), "switched off still counts");
        assert_eq!(item_installed(&journal, &item(715100, 12), &library), None, "another file of the mod");
        assert_eq!(item_installed(&journal, &item(600009, 33), &library), None, "removed since");
        let _ = std::fs::remove_dir_all(&root);
    }
}
