//! Small library and Browse facts the two reference apps show: when each mod was installed, a
//! random hero to browse, one featured mod per category, and a mod author's public contacts.
//!
//! - Install date: Grimoire prints "Installed {date}" on cards. Sigil reads it from the install
//!   journal, keyed the way the library keys a mod (slot plus directory VPK size), so a mod that
//!   moved slots in the load order still finds its install.
//! - Random hero: Deadlock Mod Manager's "Roll a random hero" on the Browse hero rail. The roll
//!   never lands on the hero already shown.
//! - Featured per category: Deadlock Mod Manager picks one mod per category each ISO week from
//!   the category's most popular, safe, downloadable mods, with a seeded shuffle so every player
//!   sees the same pick that week. Sigil does the same over a Browse page it already fetched
//!   (behaviour only; this is Sigil's own code). No extra request.
//! - Author contacts: Deadlock Mod Manager lists the creator's Discord, Steam, X and other links
//!   from the member's GameBanana profile (`_aContactInfo`). Sigil reads the same public
//!   `apiv11/Member/<id>/ProfilePage` it already uses to link an account, and keeps a link only
//!   when it is https on the platform's own host.
//!
//! Nothing here writes to the game install.

use std::collections::BTreeMap;

use chrono::{Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::gamebanana::BrowseMod;
use crate::install::InstallJournalEntry;
use crate::locker::{ROSTER, RosterHero, roster_hero};

// ---------------------------------------------------------------------------
// Install dates
// ---------------------------------------------------------------------------

/// When the mod in `slot` whose directory VPK is `vpk_bytes` was installed (journal
/// `installedAt`, RFC 3339). The newest entry for that slot and size wins; else the newest entry
/// with that size in any slot (the mod was moved in the load order); else None (installed by
/// another manager or by hand). Pass a repaired mod's original size from
/// [`crate::compat_repair::original_of`].
pub fn install_date(journal: &[InstallJournalEntry], slot: u16, vpk_bytes: u64) -> Option<String> {
    journal
        .iter()
        .rev()
        .find(|entry| entry.slot == slot && entry.vpk_bytes == vpk_bytes)
        .or_else(|| journal.iter().rev().find(|entry| entry.vpk_bytes == vpk_bytes))
        .map(|entry| entry.installed_at.clone())
}

/// [`install_date`] for many mods at once, keyed by `(slot, vpk_bytes)`.
pub fn install_dates(
    journal: &[InstallJournalEntry],
    mods: &[(u16, u64)],
) -> BTreeMap<(u16, u64), String> {
    mods.iter()
        .filter_map(|&(slot, bytes)| install_date(journal, slot, bytes).map(|at| ((slot, bytes), at)))
        .collect()
}

/// `2026-11-18` from an RFC 3339 time, in the user's local calendar; the text itself when it
/// does not parse.
pub fn short_date(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

// ---------------------------------------------------------------------------
// Random hero
// ---------------------------------------------------------------------------

/// A seed from the operating system's random source, for [`random_hero`] and [`featured`]'s
/// `salt`. Falls back to the clock.
pub fn random_seed() -> u64 {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return nanos as u64 ^ u64::from(std::process::id());
    }
    u64::from_le_bytes(bytes)
}

/// A hero from the roster, never `current` (a display name or codename) when another exists.
/// `seed` picks; pass [`random_seed`] for a fresh roll.
pub fn random_hero(current: Option<&str>, seed: u64) -> &'static RosterHero {
    let skip = current.and_then(roster_hero).map(|hero| hero.name);
    let pool: Vec<&'static RosterHero> =
        ROSTER.iter().filter(|hero| Some(hero.name) != skip).collect();
    let pool = if pool.is_empty() { ROSTER.iter().collect() } else { pool };
    pool[(mix(seed) % pool.len() as u64) as usize]
}

/// SplitMix64: spreads a small or sequential seed over every bit.
fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

// ---------------------------------------------------------------------------
// Featured mod per category
// ---------------------------------------------------------------------------

/// How many of a category's most liked eligible mods the weekly pick draws from.
pub const FEATURED_POOL: usize = 12;

/// A mod a featured slot may show: downloadable, not mature, not obsolete, and with something to
/// look at or hear (a thumbnail or a sound sample).
pub fn featured_eligible(row: &BrowseMod) -> bool {
    row.has_files
        && !row.nsfw
        && !row.obsolete
        && (row.thumbnail_url.is_some() || row.audio_url.is_some())
}

/// ISO year and week of `now_unix`, the period a featured pick holds for.
pub fn iso_week(now_unix: i64) -> (i32, u32) {
    let time = Utc.timestamp_opt(now_unix, 0).single().unwrap_or_else(Utc::now);
    let week = time.iso_week();
    (week.year(), week.week())
}

/// The category's eligible mods in this week's order; the first is the pick. The pool is the
/// [`FEATURED_POOL`] most liked (views break ties, then id), shuffled with a seed from the ISO
/// week and `salt`, so everyone sees the same pick all week and a new one the next.
pub fn featured_candidates(rows: &[BrowseMod], now_unix: i64, salt: u64) -> Vec<BrowseMod> {
    let mut pool: Vec<BrowseMod> = rows.iter().filter(|row| featured_eligible(row)).cloned().collect();
    pool.sort_by(|a, b| b.likes.cmp(&a.likes).then(b.views.cmp(&a.views)).then(a.id.cmp(&b.id)));
    pool.dedup_by_key(|row| row.id);
    pool.truncate(FEATURED_POOL);
    let (year, week) = iso_week(now_unix);
    let mut state = mix(((year as u64) * 100 + u64::from(week)) ^ mix(salt));
    for index in (1..pool.len()).rev() {
        state = mix(state);
        let pick = (state % (index as u64 + 1)) as usize;
        pool.swap(index, pick);
    }
    pool
}

/// One featured mod per category, in the order categories first appear in `rows`. A category
/// with no eligible mod is left out. `rows` is any Browse result already fetched (a category
/// page, the top list); nothing is requested here.
pub fn featured(rows: &[BrowseMod], now_unix: i64, salt: u64) -> Vec<(String, BrowseMod)> {
    let mut order: Vec<String> = Vec::new();
    let mut by_category: BTreeMap<String, Vec<BrowseMod>> = BTreeMap::new();
    for row in rows {
        let category = if row.category.is_empty() { "Other".to_string() } else { row.category.clone() };
        if !order.contains(&category) {
            order.push(category.clone());
        }
        by_category.entry(category).or_default().push(row.clone());
    }
    order
        .into_iter()
        .filter_map(|category| {
            let pick = featured_candidates(&by_category[&category], now_unix, salt).into_iter().next()?;
            Some((category, pick))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Author contacts
// ---------------------------------------------------------------------------

/// The platforms an author contact can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContactKind {
    Discord,
    Steam,
    X,
    Bluesky,
    Instagram,
    Youtube,
    Twitch,
    Kofi,
    Patreon,
    BuyMeACoffee,
    Github,
    Reddit,
    Soundcloud,
}

impl ContactKind {
    pub fn label(self) -> &'static str {
        match self {
            ContactKind::Discord => "Discord",
            ContactKind::Steam => "Steam",
            ContactKind::X => "X",
            ContactKind::Bluesky => "Bluesky",
            ContactKind::Instagram => "Instagram",
            ContactKind::Youtube => "YouTube",
            ContactKind::Twitch => "Twitch",
            ContactKind::Kofi => "Ko-fi",
            ContactKind::Patreon => "Patreon",
            ContactKind::BuyMeACoffee => "Buy Me a Coffee",
            ContactKind::Github => "GitHub",
            ContactKind::Reddit => "Reddit",
            ContactKind::Soundcloud => "SoundCloud",
        }
    }

    /// GameBanana's contact titles for the platform, lowercased.
    fn titles(self) -> &'static [&'static str] {
        match self {
            ContactKind::Discord => &["discord"],
            ContactKind::Steam => &["steam"],
            ContactKind::X => &["twitter/x", "twitter", "x"],
            ContactKind::Bluesky => &["bluesky"],
            ContactKind::Instagram => &["instagram"],
            ContactKind::Youtube => &["youtube"],
            ContactKind::Twitch => &["twitch"],
            ContactKind::Kofi => &["ko-fi", "kofi"],
            ContactKind::Patreon => &["patreon"],
            ContactKind::BuyMeACoffee => &["buy me a coffee", "buymeacoffee"],
            ContactKind::Github => &["github"],
            ContactKind::Reddit => &["reddit"],
            ContactKind::Soundcloud => &["soundcloud"],
        }
    }

    /// Hosts a link for the platform may point at (a subdomain counts).
    fn hosts(self) -> &'static [&'static str] {
        match self {
            ContactKind::Discord => &["discord.gg", "discord.com", "discordapp.com"],
            ContactKind::Steam => &["steamcommunity.com"],
            ContactKind::X => &["x.com", "twitter.com"],
            ContactKind::Bluesky => &["bsky.app"],
            ContactKind::Instagram => &["instagram.com"],
            ContactKind::Youtube => &["youtube.com"],
            ContactKind::Twitch => &["twitch.tv"],
            ContactKind::Kofi => &["ko-fi.com"],
            ContactKind::Patreon => &["patreon.com"],
            ContactKind::BuyMeACoffee => &["buymeacoffee.com"],
            ContactKind::Github => &["github.com"],
            ContactKind::Reddit => &["reddit.com"],
            ContactKind::Soundcloud => &["soundcloud.com"],
        }
    }

    /// The profile link for a bare handle. Discord has none: a user name is copied, not opened.
    fn profile_url(self, handle: &str) -> Option<String> {
        Some(match self {
            ContactKind::Discord => return None,
            ContactKind::Steam => format!("https://steamcommunity.com/id/{handle}"),
            ContactKind::X => format!("https://x.com/{handle}"),
            ContactKind::Bluesky => format!("https://bsky.app/profile/{handle}"),
            ContactKind::Instagram => format!("https://www.instagram.com/{handle}"),
            ContactKind::Youtube => format!("https://www.youtube.com/@{handle}"),
            ContactKind::Twitch => format!("https://www.twitch.tv/{handle}"),
            ContactKind::Kofi => format!("https://ko-fi.com/{handle}"),
            ContactKind::Patreon => format!("https://www.patreon.com/{handle}"),
            ContactKind::BuyMeACoffee => format!("https://buymeacoffee.com/{handle}"),
            ContactKind::Github => format!("https://github.com/{handle}"),
            ContactKind::Reddit => format!("https://www.reddit.com/user/{handle}"),
            ContactKind::Soundcloud => format!("https://soundcloud.com/{handle}"),
        })
    }

    const ALL: [ContactKind; 13] = [
        ContactKind::Discord,
        ContactKind::Steam,
        ContactKind::X,
        ContactKind::Bluesky,
        ContactKind::Instagram,
        ContactKind::Youtube,
        ContactKind::Twitch,
        ContactKind::Kofi,
        ContactKind::Patreon,
        ContactKind::BuyMeACoffee,
        ContactKind::Github,
        ContactKind::Reddit,
        ContactKind::Soundcloud,
    ];
}

/// One public contact of a mod author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorContact {
    pub kind: ContactKind,
    /// What to show: the handle, the Discord user name or the invite code.
    pub value: String,
    /// An https link on the platform's own host. None for a Discord user name (copy it).
    pub url: Option<String>,
}

/// The author's public profile as the details panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorProfile {
    pub member_id: u64,
    pub name: String,
    pub profile_url: String,
    pub avatar_url: Option<String>,
    pub contacts: Vec<AuthorContact>,
}

/// At most this many contacts are kept.
pub const MAX_CONTACTS: usize = 12;

/// `https://` plus host and path from what an author typed, with or without the scheme.
/// Returns `(host, path)` lowercased host; None when it is not a web link.
fn split_link(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &trimmed[8..]
    } else if lower.starts_with("http://") {
        &trimmed[7..]
    } else if trimmed.contains('/') || trimmed.contains('.') {
        trimmed
    } else {
        return None;
    };
    let (host, path) = match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    let host = host.to_ascii_lowercase();
    // Only plain DNS names: letters, digits, dots and dashes, no empty labels and no trailing
    // dot, so nothing (`\\`, `@`, `%`, `:`) can make the suffix check below match a host that is
    // not the trusted one.
    let host = host.trim_end_matches('.').to_string();
    if !host.contains('.')
        || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || host.split('.').any(str::is_empty)
    {
        return None;
    }
    Some((host, path.to_string()))
}

fn host_matches(host: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|trusted| host == *trusted || host.ends_with(&format!(".{trusted}")))
}

/// True for a handle: letters, digits, `_`, `.`, `-`, 1 to 64 long, with an optional `@`.
fn handle_of(value: &str) -> Option<&str> {
    let handle = value.trim().trim_start_matches('@');
    (!handle.is_empty()
        && handle.len() <= 64
        && handle.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
    .then_some(handle)
}

/// Turn one `_aContactInfo` row into a contact, or None when its title is not a known platform
/// or its value is neither a handle nor a link on that platform's host.
pub fn parse_contact(title: &str, value: &str) -> Option<AuthorContact> {
    let title = title.trim().to_ascii_lowercase();
    let kind = ContactKind::ALL.into_iter().find(|kind| kind.titles().contains(&title.as_str()))?;
    let value = value.trim();
    if value.is_empty() || value.len() > 200 {
        return None;
    }
    if let Some((host, path)) = split_link(value) {
        if !host_matches(&host, kind.hosts()) {
            return None;
        }
        let shown = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or(&host)
            .trim_start_matches('@')
            .to_string();
        return Some(AuthorContact { kind, value: shown, url: Some(format!("https://{host}{path}")) });
    }
    if kind == ContactKind::Discord {
        // A user name (2 to 32 of letters, digits, `_` and `.`, or the old `name#1234`).
        let name = value.trim_start_matches('@');
        let (base, tag) = name.split_once('#').unwrap_or((name, ""));
        let base_ok = (2..=32).contains(&base.len())
            && base.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'));
        let tag_ok = tag.is_empty() || (tag.len() == 4 && tag.chars().all(|c| c.is_ascii_digit()));
        return (base_ok && tag_ok).then(|| AuthorContact {
            kind,
            value: name.to_string(),
            url: None,
        });
    }
    let handle = handle_of(value)?;
    Some(AuthorContact { kind, value: handle.to_string(), url: kind.profile_url(handle) })
}

/// Read a `Member/<id>/ProfilePage` body. A private or banned member yields no contacts.
pub fn parse_author_profile(body: &str) -> Result<AuthorProfile> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| Error::Network(format!("GameBanana member profile was not JSON ({error}).")))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let name = text("_sName");
    if name.is_empty() {
        return Err(Error::Network("GameBanana sent a member profile with no name.".to_string()));
    }
    let hidden = ["_bIsPrivate", "_bIsBanned"]
        .iter()
        .any(|key| value.get(*key).and_then(Value::as_bool) == Some(true));
    let mut contacts: Vec<AuthorContact> = Vec::new();
    if !hidden {
        for row in value.get("_aContactInfo").and_then(Value::as_array).into_iter().flatten() {
            let field = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or("");
            if let Some(contact) = parse_contact(field("_sTitle"), field("_sValue"))
                && !contacts.contains(&contact)
            {
                contacts.push(contact);
            }
            if contacts.len() >= MAX_CONTACTS {
                break;
            }
        }
    }
    Ok(AuthorProfile {
        member_id: value.get("_idRow").and_then(Value::as_u64).unwrap_or(0),
        name,
        profile_url: text("_sProfileUrl"),
        avatar_url: Some(text("_sAvatarUrl")).filter(|url| url.starts_with("https://")),
        contacts,
    })
}

/// Fetch a mod author's public profile and contacts: one anonymous GET to
/// `gamebanana.com/apiv11/Member/<id>/ProfilePage`, the allowlisted path Settings already reads.
pub fn fetch_author_profile(member_id: u64) -> Result<AuthorProfile> {
    if member_id == 0 {
        return Err(Error::msg("This mod has no GameBanana author id."));
    }
    let url = format!("{}/Member/{member_id}/ProfilePage", crate::gamebanana::GAMEBANANA_API_BASE);
    let body = crate::gamebanana::get_body(&url, &format!("member {member_id}"))?;
    parse_author_profile(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, category: &str, likes: u64) -> BrowseMod {
        BrowseMod {
            id,
            name: format!("Mod {id}"),
            profile_url: String::new(),
            model: "Mod".into(),
            author: String::new(),
            category: category.into(),
            subcategory: String::new(),
            sub_category_id: 0,
            version: None,
            updated_at: String::new(),
            likes,
            views: 0,
            has_files: true,
            thumbnail_url: Some("https://images.gamebanana.com/x.jpg".into()),
            added_ts: 0,
            updated_ts: 0,
            nsfw: false,
            audio_url: None,
            featured: false,
            comments: 0,
            author_id: 0,
            obsolete: false,
        }
    }

    fn journal(slot: u16, bytes: u64, at: &str) -> InstallJournalEntry {
        serde_json::from_value(serde_json::json!({
            "schema": 1, "planId": "p", "installedAt": at, "source": "x",
            "sourceLabel": "x", "downloadUrl": null, "archive": "/a",
            "archiveSha256": "", "slot": slot, "file": "pak03_dir.vpk", "target": "/t",
            "chunkFiles": [], "entries": 1, "bytes": 1, "vpkBytes": bytes, "vpkSha256": "",
            "backups": [], "removedChunks": [], "removedFiles": [], "gameinfoRepaired": false,
            "warnings": []
        }))
        .unwrap()
    }

    #[test]
    fn install_dates_follow_slot_and_size() {
        let entries = vec![
            journal(3, 100, "2026-01-01T00:00:00Z"),
            journal(3, 100, "2026-02-01T00:00:00Z"),
            journal(5, 200, "2026-03-01T00:00:00Z"),
        ];
        assert_eq!(install_date(&entries, 3, 100).as_deref(), Some("2026-02-01T00:00:00Z"));
        assert_eq!(install_date(&entries, 9, 200).as_deref(), Some("2026-03-01T00:00:00Z"), "moved slot");
        assert_eq!(install_date(&entries, 3, 999), None);
        let all = install_dates(&entries, &[(3, 100), (4, 1)]);
        assert_eq!(all.len(), 1);
        assert_eq!(short_date("nope"), "nope");
        assert_eq!(short_date("2026-02-01T12:00:00Z").len(), 10);
    }

    #[test]
    fn a_random_hero_is_never_the_current_one() {
        for seed in 0..500 {
            assert_ne!(random_hero(Some("Haze"), seed).name, "Haze");
            assert_ne!(random_hero(Some("inferno"), seed).name, "Infernus");
        }
        let names: std::collections::BTreeSet<&str> =
            (0..2000).map(|seed| random_hero(None, seed).name).collect();
        assert!(names.len() > 30, "the roll covers the roster");
    }

    #[test]
    fn featured_picks_are_stable_for_a_week_and_safe() {
        let mut rows: Vec<BrowseMod> = (1..=20).map(|id| row(id, "Skins", id * 10)).collect();
        rows.push(row(100, "Sounds", 5));
        let mut nsfw = row(101, "Maps", 1000);
        nsfw.nsfw = true;
        rows.push(nsfw);
        let mut bare = row(102, "Maps", 1000);
        bare.thumbnail_url = None;
        rows.push(bare);

        let monday = 1_763_337_600; // 2025-11-17, a Monday
        let picks = featured(&rows, monday, 0);
        assert_eq!(picks.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["Skins", "Sounds"]);
        let skin = &picks[0].1;
        assert!(skin.id > 20 - FEATURED_POOL as u64, "drawn from the most liked");
        let sunday = monday + 6 * 86_400;
        assert_eq!(featured(&rows, sunday, 0)[0].1.id, skin.id, "same pick all week");
        let weeks: std::collections::BTreeSet<u64> =
            (0..10).map(|w| featured(&rows, monday + w * 7 * 86_400, 0)[0].1.id).collect();
        assert!(weeks.len() > 1, "the pick changes between weeks");
        assert_eq!(featured_candidates(&rows[..20], monday, 0).len(), FEATURED_POOL);
        assert_eq!(iso_week(monday), (2025, 47));
    }

    #[test]
    fn contacts_keep_only_safe_links() {
        let body = r#"{"_idRow":42,"_sName":"Maker","_sProfileUrl":"https://gamebanana.com/members/42",
            "_sAvatarUrl":"https://images.gamebanana.com/a.jpg","_aContactInfo":[
            {"_sTitle":"Discord","_sValue":"maker_01"},
            {"_sTitle":"Discord","_sValue":"https://discord.gg/abcd"},
            {"_sTitle":"Twitter/X","_sValue":"@maker"},
            {"_sTitle":"YouTube","_sValue":"youtube.com/@maker"},
            {"_sTitle":"Ko-fi","_sValue":"https://evil.example/kofi"},
            {"_sTitle":"Steam","_sValue":"javascript:alert(1)"},
            {"_sTitle":"Fax","_sValue":"123"},
            {"_sTitle":"Github","_sValue":"https://github.com/maker/"}]}"#;
        let profile = parse_author_profile(body).unwrap();
        assert_eq!(profile.member_id, 42);
        let shown: Vec<(ContactKind, &str, Option<&str>)> = profile
            .contacts
            .iter()
            .map(|c| (c.kind, c.value.as_str(), c.url.as_deref()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (ContactKind::Discord, "maker_01", None),
                (ContactKind::Discord, "abcd", Some("https://discord.gg/abcd")),
                (ContactKind::X, "maker", Some("https://x.com/maker")),
                (ContactKind::Youtube, "maker", Some("https://youtube.com/@maker")),
                (ContactKind::Github, "maker", Some("https://github.com/maker/")),
            ]
        );
        let private = r#"{"_sName":"Hidden","_bIsPrivate":true,"_aContactInfo":[{"_sTitle":"Discord","_sValue":"me_me"}]}"#;
        assert!(parse_author_profile(private).unwrap().contacts.is_empty());
        assert!(parse_author_profile("{}").is_err());
        assert!(fetch_author_profile(0).is_err());
    }

    #[test]
    fn a_link_cannot_borrow_a_trusted_host_name() {
        assert!(split_link("https://twitter.com/sigil").is_some());
        for bad in [
            "https://evil.com\\.twitter.com/x",
            "https://evil.com%2etwitter.com/x",
            "https://evil.com@twitter.com/x",
            "https://twitter.com:8443/x",
            "https://twitter..com/x",
            "https://tw itter.com/x",
        ] {
            assert!(split_link(bad).is_none_or(|(host, _)| !host_matches(&host, &["twitter.com"])), "{bad}");
        }
    }

}
