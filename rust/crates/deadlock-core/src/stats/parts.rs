//! A player page's load in parts, so the page fills as each answer arrives.
//!
//! One Track press used to read eleven responses one after another and draw nothing until
//! the last one, the 30 day kill map (two seconds or more on the API's side when it is not
//! in their CDN cache), had arrived. Now the first wave is four independent parts read side
//! by side ([`FIRST_WAVE`]); the peer benchmarks follow the rank, as they need its badge; and
//! what only one tab shows ([`DEFERRED`]) waits for that tab.
//!
//! Each part is first read from the disk cache whatever its age ([`StatsClient::cache_only`]),
//! so a player opened before shows at once, then from the API when that copy is past its
//! lifetime. A part that fails reports its own error: the others still fill the page.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::{
    HeroPerformance, MatchSummary, PeerBenchmarks, PlayerProfile, PlayerRank, StatsClient, StatsSnapshot,
    cache::Age, career_totals, filters, now_unix, window_stats,
};
use crate::error::Result;
use crate::util::unix_seconds_to_iso;

/// One part of a player page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartKind {
    /// Name, picture and country (`/v1/players/steam`).
    Profile,
    /// Badge, tier names, share of ranked players and season.
    Rank,
    /// Per-hero career rows and the totals summed from them.
    Heroes,
    /// The match history window.
    Matches,
    /// Per-hero averages at the player's rank tier. Needs the rank's badge.
    Peers,
}

/// What a Track press reads at once, side by side.
pub const FIRST_WAVE: [PartKind; 4] = [PartKind::Profile, PartKind::Rank, PartKind::Heroes, PartKind::Matches];

/// Every part, in the order a load asks for them.
pub const ALL: [PartKind; 5] = [PartKind::Profile, PartKind::Rank, PartKind::Heroes, PartKind::Matches, PartKind::Peers];

/// Reads a Track press does not make, and what starts each: they belong to one tab or one
/// section and wait for it.
pub const DEFERRED: [(&str, &str); 6] = [
    ("/v1/analytics/kill-death-stats", "Map tab"),
    ("/v1/assets/map", "Map tab"),
    ("/v1/analytics/player-stats/metrics", "Overview, after the rank"),
    ("/v1/players/{id}/mate-stats", "Squad tab"),
    ("/v1/players/{id}/enemy-stats", "Squad tab"),
    ("/v1/leaderboard/{region}", "Leaderboard tab"),
];

impl PartKind {
    /// The endpoints one part reads. Assets are cached for a week, so after the first load
    /// only the player's own endpoints reach the network.
    pub fn endpoints(self) -> &'static [&'static str] {
        match self {
            PartKind::Profile => &["/v1/players/steam"],
            PartKind::Rank => &[
                "/v1/players/{id}/rank",
                "/v1/assets/ranks",
                "/v1/players/rank/distribution",
                "/v1/assets/ranked-seasons",
            ],
            PartKind::Heroes => &["/v1/players/hero-stats", "/v1/assets/heroes"],
            PartKind::Matches => &["/v1/players/{id}/match-history", "/v1/assets/heroes"],
            PartKind::Peers => &["/v1/analytics/hero-stats"],
        }
    }

    /// The part this one waits for, if any.
    pub fn after(self) -> Option<PartKind> {
        match self {
            PartKind::Peers => Some(PartKind::Rank),
            _ => None,
        }
    }
}

/// One part's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Profile(PlayerProfile),
    Rank(PlayerRank, BTreeMap<u32, String>),
    Heroes(Vec<HeroPerformance>),
    Matches(Vec<MatchSummary>),
    Peers(Option<PeerBenchmarks>),
}

impl Part {
    pub fn kind(&self) -> PartKind {
        match self {
            Part::Profile(_) => PartKind::Profile,
            Part::Rank(..) => PartKind::Rank,
            Part::Heroes(_) => PartKind::Heroes,
            Part::Matches(_) => PartKind::Matches,
            Part::Peers(_) => PartKind::Peers,
        }
    }
}

/// One part arriving, from the disk cache or the API.
#[derive(Clone, Debug)]
pub struct Arrival {
    pub account_id: u64,
    pub kind: PartKind,
    pub part: std::result::Result<Part, String>,
    /// The stored copy, shown while the API is asked again. A later arrival of the same
    /// part replaces it.
    pub from_cache: bool,
    /// True when no later arrival of this part follows in this load.
    pub last: bool,
    pub age: Age,
}

/// Hero names, read once per load and shared by the hero rows and the match rows.
struct Names {
    cell: OnceLock<HashMap<u32, String>>,
}

impl Names {
    fn get(&self, client: &StatsClient) -> &HashMap<u32, String> {
        self.cell.get_or_init(|| client.fetch_hero_names().unwrap_or_default())
    }
}

/// Read one part with `client`. `badge` is the rank's badge, for the peers.
fn read_part(
    client: &StatsClient,
    account_id: u64,
    kind: PartKind,
    match_limit: usize,
    names: &Names,
    badge: Option<u32>,
) -> Result<Part> {
    Ok(match kind {
        PartKind::Profile => Part::Profile(client.fetch_player(account_id)?),
        PartKind::Rank => {
            let (rank, tiers) = client.fetch_rank_card(account_id)?;
            Part::Rank(rank, tiers)
        }
        PartKind::Heroes => {
            let mut heroes = client.fetch_hero_stats(account_id)?;
            let names = names.get(client);
            for hero in &mut heroes {
                hero.hero_name = names.get(&hero.hero_id).cloned();
            }
            heroes.sort_by(|a, b| b.matches_played.cmp(&a.matches_played).then(a.hero_id.cmp(&b.hero_id)));
            Part::Heroes(heroes)
        }
        PartKind::Matches => {
            let mut matches = client.fetch_matches(account_id, match_limit)?;
            let names = names.get(client);
            for row in &mut matches {
                row.hero_name = names.get(&row.hero_id).cloned();
            }
            Part::Matches(matches)
        }
        PartKind::Peers => {
            let badge = match badge {
                Some(badge) => badge,
                None => client.fetch_rank_card(account_id)?.0.badge,
            };
            let tiers = client.fetch_rank_names().unwrap_or_default();
            Part::Peers(client.fetch_peer_benchmarks(badge, now_unix(), tiers.get(&(badge / 10)).cloned())?)
        }
    })
}

/// Load `kinds` of one player's page, calling `send` as each part arrives. Runs one worker
/// per first-wave part (at most four requests in flight for the player's own endpoints) and
/// returns when every part has arrived or failed.
///
/// With a cache directory, each part is first read from the disk whatever its age and sent
/// with `from_cache`; when that copy is inside its lifetime the API is not asked at all.
pub fn load(
    cache_dir: Option<PathBuf>,
    account_id: u64,
    kinds: &[PartKind],
    match_limit: usize,
    send: &(dyn Fn(Arrival) + Sync),
) {
    let net = match &cache_dir {
        Some(dir) => StatsClient::with_cache(dir.clone()),
        None => StatsClient::new(),
    };
    let net_names = Names { cell: OnceLock::new() };
    let peek_names = Names { cell: OnceLock::new() };
    let want = |kind: PartKind| kinds.contains(&kind);
    // Peers ride with the rank: they need its badge.
    let peers_with_rank = want(PartKind::Peers);
    let run = |kind: PartKind| {
        // The stored copy first, so a player opened before shows at once.
        let mut fresh = false;
        let mut badge = None;
        if let Some(dir) = &cache_dir {
            let peek = StatsClient::with_cache(dir.clone()).cache_only();
            if let Ok(part) = read_part(&peek, account_id, kind, match_limit, &peek_names, None) {
                fresh = !peek.saw_expired();
                if let Part::Rank(rank, _) = &part {
                    badge = Some(rank.badge);
                }
                send(Arrival { account_id, kind, part: Ok(part), from_cache: true, last: fresh, age: peek.age() });
            }
        }
        if !fresh {
            let started = StatsClient::with_cache_dir_of(&net);
            let result = read_part(&started, account_id, kind, match_limit, &net_names, None);
            if let Ok(Part::Rank(rank, _)) = &result {
                badge = Some(rank.badge);
            }
            let age = started.age();
            send(Arrival { account_id, kind, part: result.map_err(|e| e.to_string()), from_cache: false, last: true, age });
        }
        badge
    };
    std::thread::scope(|scope| {
        for kind in FIRST_WAVE {
            if !want(kind) && !(kind == PartKind::Rank && peers_with_rank) {
                continue;
            }
            let run = &run;
            let net = &net;
            let net_names = &net_names;
            scope.spawn(move || {
                if kind != PartKind::Rank {
                    run(kind);
                    return;
                }
                // Rank asked only for the peers' sake is read quietly.
                let badge = if want(PartKind::Rank) {
                    run(PartKind::Rank)
                } else {
                    net.fetch_rank_card(account_id).ok().map(|(rank, _)| rank.badge)
                };
                if peers_with_rank {
                    let part = match badge {
                        Some(badge) => read_part(net, account_id, PartKind::Peers, match_limit, net_names, Some(badge))
                            .map_err(|e| e.to_string()),
                        None => Err("The rank did not load, so there is no tier to compare with.".to_string()),
                    };
                    send(Arrival {
                        account_id,
                        kind: PartKind::Peers,
                        part,
                        from_cache: false,
                        last: true,
                        age: Age::default(),
                    });
                }
            });
        }
    });
}

impl StatsClient {
    /// A network client on the same cache directory as `other`, with its own age.
    fn with_cache_dir_of(other: &StatsClient) -> StatsClient {
        let mut client = match &other.cache {
            Some(cache) => StatsClient::with_cache(cache.dir().to_path_buf()),
            None => StatsClient::new(),
        };
        client.agent = other.agent.clone();
        client.api_key = other.api_key.clone();
        client
    }
}

/// A page with only the player's id and name, drawn the moment Track is pressed while its
/// parts load. Every number reads as not loaded yet.
pub fn placeholder(account_id: u64, name: &str) -> StatsSnapshot {
    let now = now_unix();
    StatsSnapshot {
        account_id,
        source: super::STATS_API_BASE.to_string(),
        generated_at: unix_seconds_to_iso(now),
        profile: PlayerProfile {
            account_id,
            persona_name: if name.trim().is_empty() { account_id.to_string() } else { name.to_string() },
            profile_url: String::new(),
            avatar: String::new(),
            avatar_medium: String::new(),
            avatar_full: String::new(),
            country_code: None,
            real_name: None,
            last_updated: String::new(),
            matches_played_last_30d: 0,
            last_team_avg_badge: None,
            friend_count: 0,
        },
        rank: PlayerRank::unavailable(),
        career: career_totals(&[]),
        heroes: Vec::new(),
        matches: Vec::new(),
        window: window_stats(&[]),
        generated_unix: now,
        tier_names: BTreeMap::new(),
        peers: None,
    }
}

/// Put one arrived part into the page's snapshot, leaving the other parts as they are.
/// Match rows merge with the ones already shown ([`merge_history`]).
pub fn apply(snapshot: &mut StatsSnapshot, part: Part, match_limit: usize) {
    match part {
        Part::Profile(profile) => snapshot.profile = profile,
        Part::Rank(rank, tiers) => {
            snapshot.rank = rank;
            snapshot.tier_names = tiers;
        }
        Part::Heroes(heroes) => {
            snapshot.career = career_totals(&heroes);
            snapshot.heroes = heroes;
        }
        Part::Matches(rows) => {
            let old = std::mem::take(&mut snapshot.matches);
            snapshot.matches = merge_history(&old, rows, match_limit);
            snapshot.window = window_stats(&filters::normal_only(&snapshot.matches));
            let now = now_unix();
            snapshot.generated_unix = now;
            snapshot.generated_at = unix_seconds_to_iso(now);
        }
        Part::Peers(peers) => snapshot.peers = peers,
    }
}

/// The match window after a refresh: the new rows, plus any shown row the new answer lacks,
/// newest first, one row per match, at most `limit`.
///
/// The API has no "newer than" parameter: the history is one response, so the refresh
/// downloads it whole. The merge keeps the page steady when an answer misses rows an earlier
/// one had (the API joins Steam's list and its own store, and the two can lag each other):
/// a row the new answer carries replaces the shown one, and nothing is listed twice.
pub fn merge_history(old: &[MatchSummary], new: Vec<MatchSummary>, limit: usize) -> Vec<MatchSummary> {
    let mut seen: std::collections::HashSet<u64> = new.iter().map(|row| row.match_id).collect();
    let mut out = new;
    out.extend(old.iter().filter(|row| seen.insert(row.match_id)).cloned());
    // Stable: rows with the same start keep the order the API gave them.
    out.sort_by_key(|row| std::cmp::Reverse(row.start_time));
    out.dedup_by_key(|row| row.match_id);
    out.truncate(limit.max(1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::MatchOutcome;

    fn row(match_id: u64, start_time: i64, kills: u32) -> MatchSummary {
        MatchSummary {
            match_id,
            hero_id: 1,
            hero_name: None,
            start_time,
            started_at: String::new(),
            duration_seconds: 1800,
            game_mode: 1,
            match_mode: 4,
            player_team: 0,
            kills,
            deaths: 1,
            assists: 1,
            kda: 1.0,
            net_worth: 1,
            last_hits: 0,
            denies: 0,
            outcome: MatchOutcome::Win,
            win: Some(true),
            abandoned_seconds: None,
            ranked_badge: None,
            ranked_delta: None,
        }
    }

    fn ids(rows: &[MatchSummary]) -> Vec<u64> {
        rows.iter().map(|row| row.match_id).collect()
    }

    #[test]
    fn a_track_press_reads_the_first_wave_together_and_defers_tab_reads() {
        assert_eq!(FIRST_WAVE, [PartKind::Profile, PartKind::Rank, PartKind::Heroes, PartKind::Matches]);
        assert!(FIRST_WAVE.iter().all(|kind| kind.after().is_none()), "the first wave waits on nothing");
        assert_eq!(PartKind::Peers.after(), Some(PartKind::Rank), "peers need the badge");
        let first: Vec<&str> = FIRST_WAVE.iter().flat_map(|kind| kind.endpoints().iter().copied()).collect();
        for endpoint in ["/v1/players/steam", "/v1/players/{id}/rank", "/v1/players/hero-stats", "/v1/players/{id}/match-history"] {
            assert!(first.contains(&endpoint), "{endpoint} is in the first wave");
        }
        // The slow ones are not: the kill map took 2 to 2.5 s cold and the rank metrics 3.5 s.
        for (endpoint, _) in DEFERRED {
            assert!(!first.contains(&endpoint), "{endpoint} waits for its tab");
            assert!(!PartKind::Peers.endpoints().contains(&endpoint));
        }
        assert!(DEFERRED.iter().any(|(endpoint, when)| *endpoint == "/v1/analytics/kill-death-stats" && *when == "Map tab"));
    }

    #[test]
    fn new_matches_go_on_top_without_duplicates() {
        let old = vec![row(30, 300, 1), row(20, 200, 1), row(10, 100, 1)];
        let new = vec![row(50, 500, 1), row(40, 400, 1), row(30, 300, 9), row(20, 200, 1), row(10, 100, 1)];
        let merged = merge_history(&old, new, 200);
        assert_eq!(ids(&merged), vec![50, 40, 30, 20, 10]);
        assert_eq!(merged[2].kills, 9, "the newer answer's copy of a match wins");
    }

    #[test]
    fn rows_a_later_answer_lacks_are_kept_in_order() {
        // The API's own store can lag Steam's list: the newest shown row is missing.
        let old = vec![row(40, 400, 1), row(30, 300, 1), row(20, 200, 1)];
        let new = vec![row(30, 300, 1), row(20, 200, 1), row(10, 100, 1)];
        let merged = merge_history(&old, new, 200);
        assert_eq!(ids(&merged), vec![40, 30, 20, 10]);
        let window = merge_history(&old, merged.clone(), 3);
        assert_eq!(ids(&window), vec![40, 30, 20], "the window keeps the newest");
        assert_eq!(merge_history(&[], vec![row(1, 1, 1), row(1, 1, 1)], 10).len(), 1, "one row per match");
    }

    #[test]
    fn parts_fill_a_placeholder_one_at_a_time() {
        let mut snap = placeholder(42, "Wren");
        assert_eq!(snap.profile.persona_name, "Wren");
        assert!(!snap.rank.reported, "the rank reads -- until it arrives");
        assert!(snap.matches.is_empty() && snap.heroes.is_empty());
        apply(&mut snap, Part::Matches(vec![row(2, 200, 3), row(1, 100, 5)]), 200);
        assert_eq!(snap.window.matches, 2);
        assert_eq!(snap.profile.persona_name, "Wren", "a later part leaves the others alone");
        apply(&mut snap, Part::Matches(vec![row(3, 300, 1)]), 200);
        assert_eq!(ids(&snap.matches), vec![3, 2, 1]);
        assert_eq!(placeholder(7, " ").profile.persona_name, "7", "no name yet reads as the id");
    }

    /// Timing against the real API, by hand only: `DAH_TIMING_ACCOUNT=<id> cargo test -p
    /// deadlock-core timing_against_the_api -- --ignored --nocapture`. Public GETs for that id.
    /// `DAH_TIMING_MODE=before` times the old serial plan (eleven reads in a row, nothing
    /// drawn until the last); otherwise the parts plan: cold, warm, and with every stored
    /// copy past its lifetime.
    #[test]
    #[ignore = "reaches api.deadlock-api.com"]
    fn timing_against_the_api() {
        use std::time::Instant;
        let account: u64 = std::env::var("DAH_TIMING_ACCOUNT").ok().and_then(|v| v.parse().ok()).expect("account");
        let dir = std::env::temp_dir().join(format!("dah-stats-timing-{account}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ms = |t: Instant| t.elapsed().as_millis();
        let age_dir = || {
            // The next day: every player copy past its lifetime (assets still fresh), shown at once, then refreshed.
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let text = std::fs::read_to_string(entry.path()).unwrap();
                let (head, body) = text.split_once('\n').unwrap();
                let (at, key) = head.split_once('\t').unwrap();
                let at: i64 = at.parse().unwrap();
                std::fs::write(entry.path(), format!("{}\t{key}\n{body}", at - 25 * 3_600)).unwrap();
            }
        };
        if std::env::var("DAH_TIMING_MODE").as_deref() == Ok("before") {
            for label in ["cold", "warm", "expired"] {
                if label == "expired" {
                    age_dir();
                }
                let c = StatsClient::with_cache(dir.clone());
                let t = Instant::now();
                let _ = c.fetch_player(account);
                let _ = c.fetch_rank(account);
                let _ = c.fetch_rank_distribution();
                let _ = c.fetch_seasons();
                let _ = c.fetch_hero_stats(account);
                let _ = c.fetch_matches(account, 200);
                let _ = c.fetch_hero_names();
                println!("{label}: snapshot (first paint) {} ms", ms(t));
                let _ = c.fetch_peer_benchmarks(116, now_unix(), None);
                println!("{label}: peers {} ms", ms(t));
                let _ = c.fetch_kill_heatmap(account);
                let _ = c.fetch_map_objectives();
                println!("{label}: kill map + map asset, load done {} ms", ms(t));
            }
            return;
        }
        let run = |label: &str| {
            let t = Instant::now();
            let send = |a: Arrival| {
                println!(
                    "{label}: {:>5} ms {:?}{} {}",
                    ms(t),
                    a.kind,
                    if a.from_cache { " (stored copy)" } else { "" },
                    if a.part.is_ok() { "ok" } else { "failed" }
                )
            };
            load(Some(dir.clone()), account, &ALL, 200, &send);
            println!("{label}: done {} ms", ms(t));
        };
        run("cold");
        run("warm");
        age_dir();
        run("expired");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn peers_keep_one_cache_key_for_an_hour() {
        let at = 1_795_797_625;
        assert_eq!(crate::stats::peer_window_end(at), crate::stats::peer_window_end(at + 60));
        assert_eq!(crate::stats::peer_window_end(at) % 3_600, 0);
    }
}
