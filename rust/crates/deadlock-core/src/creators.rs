//! Creator pages: one GameBanana member's profile, every Deadlock mod and sound they posted,
//! and the totals across them.
//!
//! Behaviour after Deadlock Mod Manager's mod author profiles and author search (GPL-3.0,
//! behaviour only). DMM reads its own backend; Sigil reads GameBanana directly, the hosts the
//! Browse page already uses:
//!
//! - `GET gamebanana.com/apiv11/Member/<id>/ProfilePage`: name, avatar, contacts
//!   ([`crate::library_facts::parse_author_profile`]) plus the title, join date, subscriber
//!   count and banner read here. A private or banned member shows no extras.
//! - `GET gamebanana.com/apiv11/Mod/Index` and `Sound/Index` filtered to Deadlock and the
//!   member (`_aFilters[Generic_Submitter]`), newest first, through
//!   [`crate::gamebanana::fetch_browse`]. At most [`MOD_PAGES`] pages of 50 mods and
//!   [`SOUND_PAGES`] of sounds are read; the page says when a creator has more.
//!
//! The request carries only the public member id. Nothing about the player is sent.

use serde_json::Value;

use crate::error::{Error, Result};
use crate::gamebanana::{BrowseMod, BrowseOrder, BrowseQuery, BrowseSection};
use crate::library_facts::AuthorProfile;

/// Pages of 50 mods read for one creator.
pub const MOD_PAGES: usize = 6;
/// Pages of 50 sounds read for one creator.
pub const SOUND_PAGES: usize = 2;
const PER_PAGE: usize = 50;

/// What a member's profile page adds to the name and links.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileExtras {
    /// `_sUserTitle`.
    pub title: Option<String>,
    /// `_tsJoinDate`, unix seconds.
    pub joined: Option<i64>,
    /// `_nSubscriberCount`.
    pub subscribers: u64,
    /// `_sUpicUrl`, a banner image on GameBanana's own hosts.
    pub banner: Option<String>,
    /// `_sHdAvatarUrl`, preferred over the small avatar.
    pub hd_avatar: Option<String>,
}

fn gamebanana_image(url: &str) -> Option<String> {
    (crate::gamebanana::is_gamebanana_url(url) && !url.contains("/static/img/defaults/")).then(|| url.to_string())
}

/// Read the extras from a `ProfilePage` body. A private or banned member yields none.
pub fn parse_extras(body: &str) -> ProfileExtras {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return ProfileExtras::default();
    };
    let hidden = ["_bIsPrivate", "_bIsBanned"]
        .iter()
        .any(|key| value.get(*key).and_then(Value::as_bool) == Some(true));
    if hidden {
        return ProfileExtras::default();
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    ProfileExtras {
        title: text("_sUserTitle"),
        joined: value.get("_tsJoinDate").and_then(Value::as_i64).filter(|ts| *ts > 0),
        subscribers: value.get("_nSubscriberCount").and_then(Value::as_u64).unwrap_or(0),
        banner: text("_sUpicUrl").and_then(|url| gamebanana_image(&url)),
        hd_avatar: text("_sHdAvatarUrl").and_then(|url| gamebanana_image(&url)),
    }
}

/// One creator, read whole.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatorPage {
    pub profile: AuthorProfile,
    pub extras: ProfileExtras,
    /// Mods and sounds, newest change first.
    pub mods: Vec<BrowseMod>,
    /// GameBanana has more than the pages read.
    pub more: bool,
}

/// Totals across a creator's submissions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreatorStats {
    pub submissions: usize,
    pub likes: u64,
    pub views: u64,
    pub comments: u64,
    /// The newest change, unix seconds.
    pub last_update: i64,
    /// The first publish, unix seconds.
    pub first_post: i64,
    /// The most liked submission's id and name.
    pub top: Option<(u64, String, u64)>,
    /// Submissions per root category, most first.
    pub categories: Vec<(String, usize)>,
}

pub fn stats(mods: &[BrowseMod]) -> CreatorStats {
    let mut out = CreatorStats {
        submissions: mods.len(),
        ..CreatorStats::default()
    };
    let mut categories: Vec<(String, usize)> = Vec::new();
    for item in mods {
        out.likes += item.likes;
        out.views += item.views;
        out.comments += item.comments;
        out.last_update = out.last_update.max(item.updated_ts.max(item.added_ts));
        if item.added_ts > 0 && (out.first_post == 0 || item.added_ts < out.first_post) {
            out.first_post = item.added_ts;
        }
        if out.top.as_ref().is_none_or(|(_, _, likes)| item.likes > *likes) {
            out.top = Some((item.id, item.name.clone(), item.likes));
        }
        let category = if item.category.trim().is_empty() { "Other".to_string() } else { item.category.clone() };
        match categories.iter_mut().find(|(name, _)| *name == category) {
            Some((_, count)) => *count += 1,
            None => categories.push((category, 1)),
        }
    }
    categories.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.categories = categories;
    out
}

/// Read one section's pages for a member, newest first.
fn section_mods(member_id: u64, section: BrowseSection, pages: usize) -> Result<(Vec<BrowseMod>, bool)> {
    let query = BrowseQuery {
        section,
        order: BrowseOrder::Updated,
        submitter: Some(member_id),
        ..BrowseQuery::default()
    };
    let mut out: Vec<BrowseMod> = Vec::new();
    for page in 1..=pages {
        let read = crate::gamebanana::fetch_browse(&query, page, PER_PAGE)?;
        let count = read.mods.len();
        for item in read.mods {
            if item.author_id == 0 || item.author_id == member_id {
                out.push(item);
            }
        }
        if read.complete || count == 0 {
            return Ok((out, false));
        }
    }
    Ok((out, true))
}

/// Fetch a creator's page: the profile, then their mods and sounds.
pub fn fetch_creator(member_id: u64) -> Result<CreatorPage> {
    if member_id == 0 {
        return Err(Error::msg("This mod has no GameBanana creator id."));
    }
    let url = format!("{}/Member/{member_id}/ProfilePage", crate::gamebanana::GAMEBANANA_API_BASE);
    let body = crate::gamebanana::get_body(&url, &format!("member {member_id}"))?;
    let profile = crate::library_facts::parse_author_profile(&body)?;
    let extras = parse_extras(&body);
    let (mut mods, more_mods) = section_mods(member_id, BrowseSection::Mods, MOD_PAGES)?;
    let (sounds, more_sounds) = section_mods(member_id, BrowseSection::Sounds, SOUND_PAGES).unwrap_or_default();
    mods.extend(sounds);
    mods.sort_by_key(|item| std::cmp::Reverse(item.updated_ts.max(item.added_ts)));
    mods.dedup_by_key(|item| (item.model.clone(), item.id));
    Ok(CreatorPage {
        profile,
        extras,
        mods,
        more: more_mods || more_sounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, category: &str, likes: u64, added: i64, updated: i64) -> BrowseMod {
        BrowseMod {
            id,
            name: format!("Mod {id}"),
            profile_url: String::new(),
            model: "Mod".into(),
            author: String::new(),
            category: category.to_string(),
            subcategory: String::new(),
            sub_category_id: 0,
            version: None,
            updated_at: String::new(),
            likes,
            views: likes * 10,
            has_files: true,
            thumbnail_url: None,
            added_ts: added,
            updated_ts: updated,
            nsfw: false,
            audio_url: None,
            featured: false,
            comments: 1,
            author_id: 0,
            obsolete: false,
        }
    }

    #[test]
    fn extras_read_title_join_date_subscribers_and_safe_images() {
        let body = r#"{"_idRow": 7, "_sName": "Ana", "_sUserTitle": " Skinner ", "_tsJoinDate": 1700000000,
            "_nSubscriberCount": 42, "_sUpicUrl": "https://images.gamebanana.com/upic/x.png",
            "_sHdAvatarUrl": "https://evil.example/x.png"}"#;
        let extras = parse_extras(body);
        assert_eq!(extras.title.as_deref(), Some("Skinner"));
        assert_eq!(extras.joined, Some(1_700_000_000));
        assert_eq!(extras.subscribers, 42);
        assert_eq!(extras.banner.as_deref(), Some("https://images.gamebanana.com/upic/x.png"));
        assert_eq!(extras.hd_avatar, None, "only GameBanana's own hosts");
        let hidden = parse_extras(r#"{"_sName": "Ana", "_bIsPrivate": true, "_sUserTitle": "x"}"#);
        assert_eq!(hidden, ProfileExtras::default());
        assert_eq!(gamebanana_image("https://gamebanana.com/static/img/defaults/avatar.gif"), None);
        assert_eq!(gamebanana_image("https://user@gamebanana.com/x.png"), None);
    }

    #[test]
    fn stats_total_likes_views_and_categories() {
        let mods = vec![item(1, "Skins", 10, 100, 500), item(2, "HUD", 30, 50, 0), item(3, "Skins", 5, 300, 300)];
        let totals = stats(&mods);
        assert_eq!(totals.submissions, 3);
        assert_eq!(totals.likes, 45);
        assert_eq!(totals.views, 450);
        assert_eq!(totals.comments, 3);
        assert_eq!(totals.last_update, 500);
        assert_eq!(totals.first_post, 50);
        assert_eq!(totals.top, Some((2, "Mod 2".to_string(), 30)));
        assert_eq!(totals.categories, vec![("Skins".to_string(), 2), ("HUD".to_string(), 1)]);
        assert_eq!(stats(&[]), CreatorStats::default());
    }
}
