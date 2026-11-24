//! "For you": the landing row picked for this player, worked out on this PC from what Sigil
//! already holds (the heroes they play, the mods they installed, their favorites). Nothing is
//! sent anywhere. A player with no history gets "Rising": new mods ranked by likes per day.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::app::State;
use crate::model::BrowseCard;

/// How many picks are kept: a few pages of the row.
pub const KEEP: usize = 24;
const DAY: f64 = 24.0 * 60.0 * 60.0;

/// What the player's own history says they like. Names are lower case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signals {
    /// Hero name to weight, 0 to 1: played time and installed mods for that hero.
    pub heroes: HashMap<String, f32>,
    /// Creator name to weight: 1 for a favorite, 0.6 for an installed mod.
    pub authors: HashMap<String, f32>,
    /// Root category ("skins") to its share of the player's mods and favorites, 0 to 1.
    pub roots: HashMap<String, f32>,
    /// Mods the player already has or saved: never recommended.
    pub owned: HashSet<u64>,
}

impl Signals {
    pub fn is_empty(&self) -> bool {
        self.heroes.is_empty() && self.authors.is_empty() && self.roots.is_empty()
    }
}

/// Why a pick is in the row, as the tile says it.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    Hero(String),
    Author(String),
    Category(String),
    Rising,
}

#[derive(Clone, Default)]
pub struct ForYou {
    pub picks: Vec<BrowseCard>,
    pub reasons: HashMap<u64, Reason>,
    /// False when there was no history to go on and the row is "Rising".
    pub personal: bool,
    /// What the picks were worked out from, so they are redone only when that changes.
    pub key: u64,
}

fn leaf(category: &str) -> String {
    category.rsplit('/').next().unwrap_or("").trim().to_lowercase()
}

fn root(category: &str) -> String {
    category.split('/').next().unwrap_or("").trim().to_lowercase()
}

fn scale(map: &mut HashMap<String, f32>) {
    let max = map.values().copied().fold(0.0f32, f32::max);
    if max > 0.0 {
        for v in map.values_mut() {
            *v /= max;
        }
    }
}

/// Read the player's history out of the app state.
pub fn signals(s: &State) -> Signals {
    let mut sig = Signals::default();
    // Heroes by time played.
    let mut played: HashMap<String, f32> = HashMap::new();
    if let Some(snapshot) = &s.stats.snapshot {
        for hero in &snapshot.heroes {
            let Some(name) = hero.hero_name.as_deref().map(str::to_lowercase).filter(|n| !n.is_empty()) else { continue };
            let weight = hero.time_played_seconds.max(hero.matches_played * 1800) as f32;
            if weight > 0.0 {
                *played.entry(name).or_default() += weight;
            }
        }
    }
    scale(&mut played);
    // Heroes, creators and categories of the mods they installed: a mod that is on counts double.
    let mut modded: HashMap<String, f32> = HashMap::new();
    let mut roots: HashMap<String, f32> = HashMap::new();
    for addon in &s.addons.addons {
        let weight = if addon.enabled { 2.0 } else { 1.0 };
        let extra = crate::screens::installed::notes::extra(s, addon);
        if let Some(hero) = extra.hero.as_deref().map(str::to_lowercase).filter(|h| !h.is_empty()) {
            *modded.entry(hero).or_default() += weight;
        }
        if let Some(root) = extra.root_category.as_deref().map(str::to_lowercase).filter(|r| !r.is_empty()) {
            *roots.entry(root).or_default() += weight;
        }
    }
    for meta in &s.installed.library.mods {
        if let Some(id) = meta.gamebanana_id {
            sig.owned.insert(id);
        }
        if let Some(author) = meta.author.as_deref().map(str::to_lowercase).filter(|a| !a.is_empty()) {
            let w = sig.authors.entry(author).or_default();
            *w = w.max(0.6);
        }
    }
    sig.owned.extend(s.browse_view.installs.owned.keys().copied());
    for favorite in s.browse_view.favorites.iter().flatten() {
        sig.owned.insert(favorite.id);
        if !favorite.author.is_empty() {
            sig.authors.insert(favorite.author.to_lowercase(), 1.0);
        }
        let hero = leaf(&favorite.category);
        if !hero.is_empty() && played.contains_key(&hero) {
            *modded.entry(hero).or_default() += 1.5;
        }
        let r = root(&favorite.category);
        if !r.is_empty() {
            *roots.entry(r).or_default() += 1.5;
        }
    }
    scale(&mut modded);
    for (hero, weight) in played {
        sig.heroes.insert(hero, 0.6 * weight);
    }
    for (hero, weight) in modded {
        *sig.heroes.entry(hero).or_default() += 0.4 * weight;
    }
    scale(&mut sig.heroes);
    // Categories as shares, so one stray mod does not count like a habit.
    let total: f32 = roots.values().sum();
    if total > 0.0 {
        sig.roots = roots.into_iter().map(|(k, v)| (k, v / total)).collect();
    }
    sig
}

/// How good a mod is on its own, 0 to 1: likes on a log scale against the best candidate, a
/// little more for a good like rate, and for being recent.
fn quality(card: &BrowseCard, top_likes: f64, now: i64) -> f32 {
    let likes = (1.0 + card.likes as f64).ln() / (1.0 + top_likes).ln().max(1.0);
    // Smoothed like rate: a mod with 3 views and 3 likes is not a hit.
    let rate = if card.views >= 50 { (card.likes as f64 + 2.0) / (card.views as f64 + 400.0) } else { 0.0 };
    let age_days = if card.added_ts > 0 { ((now - card.added_ts).max(0) as f64) / DAY } else { 365.0 };
    let fresh = (-age_days / 45.0).exp();
    (likes * (0.55 + 0.25 * fresh + (rate * 12.0).min(0.20))) as f32
}

/// Rank `candidates` for a player. Returns at most `KEEP` picks, best first, each with why.
pub fn rank(candidates: &[BrowseCard], sig: &Signals, now: i64) -> (Vec<BrowseCard>, HashMap<u64, Reason>, bool) {
    let mut seen = HashSet::new();
    let pool: Vec<&BrowseCard> = candidates
        .iter()
        .filter(|c| c.model.eq_ignore_ascii_case("mod") && !c.obsolete && c.thumbnail_url.is_some())
        .filter(|c| !sig.owned.contains(&c.id) && seen.insert(c.id))
        .collect();
    let top_likes = pool.iter().map(|c| c.likes).max().unwrap_or(0) as f64;
    let mut scored: Vec<(f32, &BrowseCard, Reason, String, String)> = Vec::new();
    if !sig.is_empty() {
        for c in &pool {
            if c.likes < 3 {
                continue;
            }
            let hero_name = leaf(&c.category);
            let hero = sig.heroes.get(&hero_name).copied().unwrap_or(0.0);
            let author_name = c.author.to_lowercase();
            let author = sig.authors.get(&author_name).copied().unwrap_or(0.0);
            let category = sig.roots.get(&root(&c.category)).copied().unwrap_or(0.0);
            if hero <= 0.0 && author <= 0.0 && category < 0.15 {
                continue;
            }
            let score = quality(c, top_likes, now) * (1.0 + 2.0 * hero + 1.0 * author + 0.5 * category);
            let reason = if 2.0 * hero >= author && hero > 0.0 {
                Reason::Hero(c.category.rsplit('/').next().unwrap_or("").trim().to_string())
            } else if author > 0.0 {
                Reason::Author(c.author.clone())
            } else {
                Reason::Category(c.category.split('/').next().unwrap_or("").trim().to_string())
            };
            scored.push((score, c, reason, hero_name, author_name));
        }
    }
    let personal = scored.len() >= 6;
    if !personal {
        // Rising: likes per day over the last two weeks, so a fast riser beats an older mod
        // that merely had longer to collect.
        scored.clear();
        for c in &pool {
            let age_days = if c.added_ts > 0 { ((now - c.added_ts).max(0) as f64) / DAY } else { continue };
            if age_days > 14.0 || c.likes < 3 {
                continue;
            }
            let score = (c.likes as f64 / (age_days + 2.0).powf(1.2)) as f32;
            scored.push((score, c, Reason::Rising, leaf(&c.category), c.author.to_lowercase()));
        }
    }
    // Greedy pick with a falling return per hero and per creator, so the row is not ten skins
    // for one hero by one maker.
    let mut picks = Vec::new();
    let mut reasons = HashMap::new();
    let mut per_hero: HashMap<String, u32> = HashMap::new();
    let mut per_author: HashMap<String, u32> = HashMap::new();
    while picks.len() < KEEP && !scored.is_empty() {
        let mut best = 0;
        let mut best_score = f32::MIN;
        for (i, (score, _, _, hero, author)) in scored.iter().enumerate() {
            let h = per_hero.get(hero).copied().unwrap_or(0);
            let a = per_author.get(author).copied().unwrap_or(0);
            let adjusted = score * 0.65f32.powi(h as i32) * 0.5f32.powi(a as i32);
            if adjusted > best_score {
                best_score = adjusted;
                best = i;
            }
        }
        let (_, card, reason, hero, author) = scored.swap_remove(best);
        if !hero.is_empty() {
            *per_hero.entry(hero).or_default() += 1;
        }
        if !author.is_empty() {
            *per_author.entry(author).or_default() += 1;
        }
        reasons.insert(card.id, reason);
        picks.push(super::store::clone_card(card));
    }
    (picks, reasons, personal)
}

/// A cheap fingerprint of everything the picks depend on.
fn key_of(s: &State) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let v = &s.browse_view;
    v.shelves.feeds.as_ref().map(|f| (f.recent.len(), f.liked.len())).hash(&mut h);
    v.mirror.as_ref().map(|m| m.entries.len()).hash(&mut h);
    v.favorites.as_ref().map(Vec::len).hash(&mut h);
    v.installs.owned.len().hash(&mut h);
    s.installed.library.mods.len().hash(&mut h);
    for addon in &s.addons.addons {
        (addon.slot, addon.enabled).hash(&mut h);
    }
    s.stats.snapshot.as_ref().map(|snap| snap.heroes.iter().map(|x| x.matches_played).sum::<u64>()).hash(&mut h);
    h.finish()
}

/// Work the picks out again when what they depend on changed. True when they changed.
pub fn refresh(st: &mut State) -> bool {
    if st.browse_view.shelves.feeds.is_none() && st.browse_view.mirror.is_none() {
        return false;
    }
    let key = key_of(st);
    if st.browse_view.shelves.for_you.key == key {
        return false;
    }
    let sig = signals(st);
    let mut candidates: Vec<BrowseCard> = Vec::new();
    if let Some(feeds) = &st.browse_view.shelves.feeds {
        candidates.extend(feeds.recent.iter().chain(feeds.liked.iter()).map(super::store::clone_card));
    }
    // The saved catalog is the wide pool: every mod, so a hero's best skin is found even when
    // it is neither new nor among the all-time top.
    if let Some(mirror) = &st.browse_view.mirror {
        let wanted = |category: &str| {
            let hero = leaf(category);
            sig.heroes.contains_key(&hero) || sig.roots.contains_key(&root(category))
        };
        candidates.extend(
            mirror
                .entries
                .iter()
                .filter(|e| e.likes >= 3)
                .map(super::store::mirror_card)
                .filter(|c| wanted(&c.category) || sig.authors.contains_key(&c.author.to_lowercase())),
        );
    }
    let (picks, reasons, personal) = rank(&candidates, &sig, deadlock_core::gamebanana::now_unix());
    st.browse_view.shelves.for_you = ForYou { picks, reasons, personal, key };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn card(id: u64, category: &str, author: &str, likes: u64, days_ago: i64) -> BrowseCard {
        let mut c = super::super::top::card_of(&deadlock_core::gamebanana::TopSub {
            id,
            name: format!("Mod {id}"),
            author: author.into(),
            image_url: Some(format!("https://images.example/{id}.jpg")),
            ..Default::default()
        });
        c.category = category.into();
        c.likes = likes;
        c.views = likes * 40;
        c.added_ts = NOW - days_ago * 24 * 60 * 60;
        c
    }

    fn pool() -> Vec<BrowseCard> {
        let mut list = Vec::new();
        for i in 0..12 {
            list.push(card(100 + i, "Skins / Haze", &format!("maker{}", i % 6), 40 + i * 5, 20 + i as i64));
            list.push(card(200 + i, "Skins / Paradox", &format!("maker{}", i % 6), 60 + i * 5, 30 + i as i64));
            list.push(card(300 + i, "Skins / Abrams", &format!("other{}", i % 6), 500 + i * 5, 40 + i as i64));
            list.push(card(400 + i, "HUD / Crosshairs", &format!("hud{}", i % 6), 80 + i, 10 + i as i64));
        }
        list
    }

    /// A Haze player gets Haze mods first, even though Abrams mods have far more likes.
    #[test]
    fn played_heroes_lead_the_row() {
        let mut sig = Signals::default();
        sig.heroes.insert("haze".into(), 1.0);
        sig.heroes.insert("paradox".into(), 0.4);
        let (picks, reasons, personal) = rank(&pool(), &sig, NOW);
        assert!(personal);
        assert_eq!(leaf(&picks[0].category), "haze");
        assert!(picks.iter().take(8).all(|c| ["haze", "paradox"].contains(&leaf(&c.category).as_str())), "top picks are for played heroes");
        assert!(picks.iter().all(|c| leaf(&c.category) != "abrams"), "no mods for a hero they do not play");
        assert_eq!(reasons[&picks[0].id], Reason::Hero("Haze".into()));
    }

    /// The row is varied: not one hero or one creator over and over.
    #[test]
    fn the_row_is_varied() {
        let mut sig = Signals::default();
        sig.heroes.insert("haze".into(), 1.0);
        sig.heroes.insert("paradox".into(), 0.9);
        let (picks, _, _) = rank(&pool(), &sig, NOW);
        let first: Vec<String> = picks.iter().take(6).map(|c| leaf(&c.category)).collect();
        assert!(first.contains(&"haze".to_string()) && first.contains(&"paradox".to_string()), "{first:?}");
        let mut by_author: HashMap<&str, usize> = HashMap::new();
        for c in picks.iter().take(6) {
            *by_author.entry(c.author.as_str()).or_default() += 1;
        }
        assert!(by_author.values().all(|&n| n <= 2), "{by_author:?}");
    }

    /// What they already have or saved is never offered again.
    #[test]
    fn owned_mods_are_left_out() {
        let mut sig = Signals::default();
        sig.heroes.insert("haze".into(), 1.0);
        sig.owned.extend([111u64, 110, 109]);
        let (picks, _, _) = rank(&pool(), &sig, NOW);
        assert!(picks.iter().all(|c| !sig.owned.contains(&c.id)));
    }

    /// A creator they favorited brings that creator's other mods in, with the reason.
    #[test]
    fn a_favorite_creator_counts() {
        let mut sig = Signals::default();
        sig.authors.insert("hud3".into(), 1.0);
        let mut list = pool();
        for i in 0..8 {
            list.push(card(900 + i, "HUD / Crosshairs", "hud3", 30 + i, 5));
        }
        let (picks, reasons, personal) = rank(&list, &sig, NOW);
        assert!(personal);
        assert_eq!(picks[0].author, "hud3");
        assert_eq!(reasons[&picks[0].id], Reason::Author("hud3".into()));
    }

    /// No history: the row is Rising, the fastest climbers of the last two weeks.
    #[test]
    fn no_history_gives_rising() {
        let mut list = pool();
        list.push(card(1, "Skins / Wraith", "new", 60, 1));
        list.push(card(2, "Skins / Wraith", "new2", 90, 12));
        let (picks, reasons, personal) = rank(&list, &Signals::default(), NOW);
        assert!(!personal);
        assert_eq!(picks[0].id, 1, "60 likes in a day beats 90 in twelve");
        assert!(picks.iter().all(|c| NOW - c.added_ts <= 14 * 24 * 60 * 60));
        assert_eq!(reasons[&1], Reason::Rising);
    }

    /// A barely liked mod is never a recommendation, however well it matches.
    #[test]
    fn weak_mods_are_not_recommended() {
        let mut sig = Signals::default();
        sig.heroes.insert("haze".into(), 1.0);
        let mut list = pool();
        list.push(card(7, "Skins / Haze", "x", 1, 2));
        let (picks, _, _) = rank(&list, &sig, NOW);
        assert!(picks.iter().all(|c| c.id != 7));
    }
}
