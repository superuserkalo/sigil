//! Pictures on the Stats page: rank badges, item icons and Steam avatars.
//!
//! Views never ask for a picture: `poll` reads what the page shows (the subject's rank, the
//! open match's players, search results) and fetches what is missing on one worker, a batch
//! at a time, through `deadlock_core::stats::art`, which refuses any host off its allowlist.
//! Each picture is scaled down once and kept in `cache/stats/art` for 30 days, so a badge is
//! fetched once a month at most. Nothing is fetched while Player stats is off.

use std::collections::BTreeSet;
use std::path::PathBuf;

use deadlock_core::stats::art;

use crate::app::State;
use crate::jobs::Job;

/// Pictures asked for in one worker run.
const BATCH: usize = 24;
/// How long a kept picture is used before it is fetched again.
const KEEP_SECONDS: u64 = 30 * 86_400;
/// Rank badges and avatars are kept at this many pixels on the long side.
pub const BADGE_PX: u32 = 64;
const AVATAR_PX: u32 = 64;
/// Item icons draw at 20 to 24 points: 48 pixels covers 2x.
const ITEM_PX: u32 = 48;
/// Squad rows whose avatars are fetched: the ones the tab shows first.
const SQUAD_AVATARS: usize = 24;

#[derive(Default)]
pub struct Pictures {
    /// Keys already asked for this session, found or not, so a missing picture is not
    /// asked for again every poll.
    asked: BTreeSet<String>,
    job: Job<Vec<(String, Vec<u8>)>>,
}

/// The image cache key of a rank badge, `None` for an unranked badge.
pub fn rank_key(badge: u32) -> Option<String> {
    art::badge_parts(badge).map(|(rank, sub)| format!("stats-art:rank:{rank}-{sub}"))
}

/// The image cache key of an item icon.
pub fn item_key(item_id: u64) -> String {
    format!("stats-art:item:{item_id}")
}

/// The image cache key of a player's avatar.
pub fn avatar_key(account_id: u64) -> String {
    format!("stats-art:avatar:{account_id}")
}

fn profile_avatar(profile: &deadlock_core::stats::PlayerProfile) -> &str {
    if profile.avatar_medium.is_empty() { &profile.avatar } else { &profile.avatar_medium }
}

fn cache_dir() -> PathBuf {
    crate::data::stats_cache_dir().join("art")
}

/// Every picture the page shows now, as (key, URL, pixels).
fn wanted(st: &State) -> Vec<(String, String, u32)> {
    let v = &st.stats;
    let mut out = Vec::new();
    let mut badge = |badge: u32| {
        if let (Some(key), Some((rank, sub))) = (rank_key(badge), art::badge_parts(badge)) {
            out.push((key, art::rank_badge_url(rank, sub), BADGE_PX));
        }
    };
    if let Some(snap) = &v.snapshot {
        badge(snap.rank.badge);
    }
    let mut avatars: Vec<(u64, &str)> = Vec::new();
    let mut items: Vec<(u64, &str)> = Vec::new();
    if let Some(snap) = &v.snapshot {
        avatars.push((snap.account_id, profile_avatar(&snap.profile)));
    }
    if let super::Fetched::Ready(board) = &v.board {
        for player in board.teams.iter().flatten() {
            if let Some(b) = player.badge {
                badge(b);
            }
            if let Some(url) = &player.avatar {
                avatars.push((player.account_id, url));
            }
            for item in &player.build {
                if let Some(url) = &item.icon {
                    items.push((item.item_id, url));
                }
            }
        }
        for b in board.average_badge.iter().flatten() {
            badge(*b);
        }
    }
    for found in &v.lookup.results {
        if let Some(rank) = found.rank {
            badge(rank.badge);
        }
        if let Some(url) = &found.avatar {
            avatars.push((found.account_id, url));
        }
    }
    if let super::Fetched::Ready(social) = &v.social {
        for record in social.mates.iter().take(SQUAD_AVATARS).chain(social.enemies.iter().take(SQUAD_AVATARS / 2)) {
            if let Some(url) = &record.avatar {
                avatars.push((record.account_id, url));
            }
        }
    }
    if let super::Fetched::Ready(load) = &v.more.items {
        items.extend(load.icons.iter().map(|(id, url)| (*id, url.as_str())));
    }
    for (account_id, url) in avatars {
        if !url.is_empty() {
            out.push((avatar_key(account_id), url.to_string(), AVATAR_PX));
        }
    }
    for (item_id, url) in items {
        out.push((item_key(item_id), url.to_string(), ITEM_PX));
    }
    out
}

/// Take finished pictures and start the next batch. Called from the Stats poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.stats.pictures.job.try_take() {
        if let Ok(found) = result {
            changed |= !found.is_empty();
            st.pending_thumbs.extend(found);
        }
    }
    if !st.prefs.allow_stats || st.demo || rune::is_headless() || st.stats.pictures.job.is_running() {
        return changed;
    }
    let batch: Vec<(String, String, u32)> = wanted(st)
        .into_iter()
        .filter(|(key, _, _)| !st.thumbnails.contains_key(key) && !st.stats.pictures.asked.contains(key))
        .take(BATCH)
        .collect();
    if batch.is_empty() {
        return changed;
    }
    for (key, _, _) in &batch {
        st.stats.pictures.asked.insert(key.clone());
    }
    let wake = st.wake.clone();
    let dir = cache_dir();
    st.stats.pictures.job.start(wake, move || {
        Ok(batch.into_iter().filter_map(|(key, url, px)| load(&dir, &url, px).map(|bytes| (key, bytes))).collect())
    });
    changed
}

/// One picture: the kept copy while it is fresh, else fetched, scaled to `px` and kept.
fn load(dir: &std::path::Path, url: &str, px: u32) -> Option<Vec<u8>> {
    let path = dir.join(format!("{}.png", art::cache_name(url)));
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age.as_secs() < KEEP_SECONDS);
    if fresh && let Ok(bytes) = std::fs::read(&path) {
        return Some(bytes);
    }
    let bytes = art::fetch_image(url).ok()?;
    let scaled = scale(&bytes, px)?;
    if std::fs::create_dir_all(dir).is_ok() {
        let partial = path.with_extension("png.part");
        if std::fs::write(&partial, &scaled).is_ok() {
            let _ = std::fs::rename(&partial, &path);
        }
    }
    Some(scaled)
}

/// Decode, shrink to `px` on the long side (never enlarge), and encode as PNG.
pub fn scale(bytes: &[u8], px: u32) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width().max(image.height()) > px { image.thumbnail(px, px) } else { image };
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_keys_name_tier_and_subrank() {
        assert_eq!(rank_key(113).as_deref(), Some("stats-art:rank:11-3"));
        assert_eq!(rank_key(0), None);
    }

    #[test]
    fn scaling_shrinks_and_never_enlarges() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(200, 100).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let small = scale(png.get_ref(), 64).expect("scaled");
        let back = image::load_from_memory(&small).unwrap();
        assert_eq!((back.width(), back.height()), (64, 32));
        let same = scale(&small, 128).expect("kept");
        assert_eq!(image::load_from_memory(&same).unwrap().width(), 64);
        assert!(scale(b"not an image", 64).is_none());
    }
}
