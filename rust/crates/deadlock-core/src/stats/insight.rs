//! Derived views over a [`StatsSnapshot`]: rank trajectory, rolling form,
//! streaks, sessions, and the short brief at the top of the Stats screen.
//!
//! Every function here is pure. Inputs are the match window (newest first, as
//! `fetch_matches` returns it), the career hero rows and the optional peer
//! benchmarks. Thresholds are constants so the screen, the docs and the tests
//! agree on them:
//!
//! - A comparison needs at least [`MIN_SAMPLE`] decided matches on each side.
//! - A win rate gap counts when it is at least [`GAP_POINTS`] points.
//! - A sitting (session) ends after a [`SESSION_GAP_SECONDS`] break.
//!
//! Nothing here invents a rating. Each insight quotes the numbers it rests on,
//! and the sample they come from.

use std::collections::BTreeMap;

use chrono::{Datelike, Local, TimeZone};

use super::{HeroPerformance, MatchSummary, StatsSnapshot};

/// A break longer than this starts a new sitting.
pub const SESSION_GAP_SECONDS: i64 = 3 * 3600;
/// "Recent" is the newest 20 matches in the window.
pub const RECENT_MATCHES: usize = 20;
/// Smallest decided sample a comparison may rest on.
pub const MIN_SAMPLE: u64 = 15;
/// Smallest win rate gap, in points, worth a sentence.
pub const GAP_POINTS: f64 = 6.0;
/// Heroes need this many career matches before they can carry an insight.
pub const HERO_SAMPLE: u64 = 20;
/// Subranks per rank tier.
pub const SUBRANKS: u32 = 6;

/// Ladder position of a badge (`tier * 10 + subrank`): Initiate 1 is 1,
/// Eternus 6 is 66. `None` for unranked or a malformed badge.
pub fn badge_step(badge: u32) -> Option<u32> {
    let tier = badge / 10;
    let subrank = badge % 10;
    if tier == 0 || subrank == 0 || subrank > SUBRANKS {
        return None;
    }
    Some((tier - 1) * SUBRANKS + subrank)
}

/// The badge for a ladder position, the inverse of [`badge_step`].
pub fn step_badge(step: u32) -> u32 {
    let step = step.max(1);
    let tier = (step - 1) / SUBRANKS + 1;
    let subrank = (step - 1) % SUBRANKS + 1;
    tier * 10 + subrank
}

/// `Eternus 5`, or `Unranked`.
pub fn badge_label(badge: u32, names: &BTreeMap<u32, String>) -> String {
    let tier = badge / 10;
    let subrank = badge % 10;
    if tier == 0 {
        return "Unranked".to_string();
    }
    let name = names
        .get(&tier)
        .cloned()
        .unwrap_or_else(|| format!("Tier {tier}"));
    if subrank == 0 {
        name
    } else {
        format!("{name} {subrank}")
    }
}

/// One ranked match on the ladder.
#[derive(Debug, Clone, PartialEq)]
pub struct RankPoint {
    pub match_id: u64,
    pub start_time: i64,
    pub badge: u32,
    pub step: u32,
    pub delta: Option<i32>,
    pub win: Option<bool>,
}

/// Ranked matches with a badge, oldest first. The badge is the one the match
/// history reports for that match (`ranked_display_badge`).
pub fn rank_trajectory(matches: &[MatchSummary]) -> Vec<RankPoint> {
    let mut points: Vec<RankPoint> = matches
        .iter()
        .filter_map(|summary| {
            let badge = summary.ranked_badge?;
            let step = badge_step(badge)?;
            Some(RankPoint {
                match_id: summary.match_id,
                start_time: summary.start_time,
                badge,
                step,
                delta: summary.ranked_delta,
                win: summary.win,
            })
        })
        .collect();
    points.sort_by_key(|point| point.start_time);
    points
}

/// Index of the first point of the current season. A season reset shows up as
/// a drop of a full tier (six subranks) or more between two consecutive ranked
/// matches; a single loss never moves a badge that far.
pub fn season_start(points: &[RankPoint]) -> usize {
    points
        .windows(2)
        .rposition(|pair| pair[0].step >= pair[1].step + SUBRANKS)
        .map(|index| index + 1)
        .unwrap_or(0)
}

/// Win rate over the last `window` decided matches at each decided match,
/// oldest first. Starts once the window is full, so no point rests on fewer
/// than `window` games.
pub fn rolling_win_rate(matches: &[MatchSummary], window: usize) -> Vec<(i64, f64)> {
    let decided: Vec<(i64, bool)> = matches
        .iter()
        .rev()
        .filter_map(|summary| summary.win.map(|win| (summary.start_time, win)))
        .collect();
    if window == 0 || decided.len() < window {
        return Vec::new();
    }
    let mut wins = decided[..window].iter().filter(|(_, win)| *win).count();
    let mut out = vec![(decided[window - 1].0, 100.0 * wins as f64 / window as f64)];
    for index in window..decided.len() {
        if decided[index].1 {
            wins += 1;
        }
        if decided[index - window].1 {
            wins -= 1;
        }
        out.push((decided[index].0, 100.0 * wins as f64 / window as f64));
    }
    out
}

/// The run of identical results that ends at the newest decided match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    pub win: bool,
    pub length: usize,
}

pub fn current_streak(matches: &[MatchSummary]) -> Option<Streak> {
    let mut decided = matches.iter().filter_map(|summary| summary.win);
    let first = decided.next()?;
    let length = 1 + decided.take_while(|win| *win == first).count();
    Some(Streak {
        win: first,
        length,
    })
}

/// Longest run of `win` results anywhere in the window.
pub fn longest_run(matches: &[MatchSummary], win: bool) -> usize {
    let mut best = 0;
    let mut run = 0;
    for result in matches.iter().filter_map(|summary| summary.win) {
        if result == win {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// How many runs of at least `length` identical `win` results the window holds.
pub fn runs_at_least(matches: &[MatchSummary], win: bool, length: usize) -> usize {
    let mut count = 0;
    let mut run = 0;
    for result in matches.iter().filter_map(|summary| summary.win) {
        if result == win {
            run += 1;
            if run == length {
                count += 1;
            }
        } else {
            run = 0;
        }
    }
    count
}

/// One sitting: matches with no break longer than [`SESSION_GAP_SECONDS`].
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub start_time: i64,
    pub end_time: i64,
    /// Indexes into the match window, newest first.
    pub indices: Vec<usize>,
    pub wins: usize,
    pub losses: usize,
    /// Sum of `ranked_delta` over the ranked matches.
    pub rank_delta: i64,
    pub ranked: usize,
}

/// Sittings, newest first, from a match window ordered newest first.
pub fn sessions(matches: &[MatchSummary], gap_seconds: i64) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for (index, summary) in matches.iter().enumerate() {
        let end = summary.start_time + summary.duration_seconds as i64;
        let joins = out
            .last()
            .is_some_and(|session| session.start_time - end <= gap_seconds);
        if !joins {
            out.push(Session {
                start_time: summary.start_time,
                end_time: end,
                indices: Vec::new(),
                wins: 0,
                losses: 0,
                rank_delta: 0,
                ranked: 0,
            });
        }
        let session = out.last_mut().expect("a session was just pushed");
        session.start_time = session.start_time.min(summary.start_time);
        session.indices.push(index);
        match summary.win {
            Some(true) => session.wins += 1,
            Some(false) => session.losses += 1,
            None => {}
        }
        if let Some(delta) = summary.ranked_delta {
            session.rank_delta += delta as i64;
            session.ranked += 1;
        }
    }
    out
}

/// Sums over a slice of matches, with the per-match rates the screen shows.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Aggregate {
    pub matches: u64,
    pub wins: u64,
    pub losses: u64,
    pub kills: u64,
    pub deaths: u64,
    pub assists: u64,
    pub seconds: u64,
    pub net_worth: u64,
}

impl Aggregate {
    pub fn of<'a>(matches: impl IntoIterator<Item = &'a MatchSummary>) -> Aggregate {
        let mut total = Aggregate::default();
        for summary in matches {
            total.matches += 1;
            match summary.win {
                Some(true) => total.wins += 1,
                Some(false) => total.losses += 1,
                None => {}
            }
            total.kills += summary.kills as u64;
            total.deaths += summary.deaths as u64;
            total.assists += summary.assists as u64;
            if summary.duration_seconds > 0 {
                total.seconds += summary.duration_seconds as u64;
                total.net_worth += summary.net_worth;
            }
        }
        total
    }

    pub fn decided(&self) -> u64 {
        self.wins + self.losses
    }

    pub fn win_rate(&self) -> Option<f64> {
        (self.decided() > 0).then(|| 100.0 * self.wins as f64 / self.decided() as f64)
    }

    pub fn kda(&self) -> f64 {
        (self.kills + self.assists) as f64 / self.deaths.max(1) as f64
    }

    pub fn deaths_per_match(&self) -> f64 {
        if self.matches == 0 {
            0.0
        } else {
            self.deaths as f64 / self.matches as f64
        }
    }

    /// Final net worth over minutes played: souls per minute.
    pub fn souls_per_minute(&self) -> f64 {
        if self.seconds == 0 {
            0.0
        } else {
            self.net_worth as f64 / (self.seconds as f64 / 60.0)
        }
    }
}

/// The newest [`RECENT_MATCHES`] against the rest of the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Form {
    pub recent: Aggregate,
    pub before: Aggregate,
}

pub fn form(matches: &[MatchSummary]) -> Form {
    let split = matches.len().min(RECENT_MATCHES);
    Form {
        recent: Aggregate::of(&matches[..split]),
        before: Aggregate::of(&matches[split..]),
    }
}

impl Form {
    /// Both sides are large enough to compare.
    pub fn comparable(&self) -> bool {
        self.recent.decided() >= MIN_SAMPLE && self.before.decided() >= MIN_SAMPLE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Recognition: something went well.
    Good,
    /// Something to work on.
    Focus,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    RankMove,
    WinStreak,
    LossStreak,
    FormUp,
    FormDown,
    ComfortHero,
    TrapHero,
    LongSessions,
    DeathsUp,
    DeathsDown,
    PeerGap,
    LastSession,
}

/// One sentence and the evidence behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct Insight {
    pub kind: Kind,
    pub tone: Tone,
    pub title: String,
    pub detail: String,
    pub hero_id: Option<u32>,
    /// Size of the effect times the square root of its sample. Orders focus
    /// items; not shown.
    pub weight: f64,
}

/// What changed, what to work on next, and up to three more notes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Brief {
    pub changed: Option<Insight>,
    pub next: Option<Insight>,
    pub also: Vec<Insight>,
}

/// Every insight the window supports, unordered.
pub fn insights(snapshot: &StatsSnapshot) -> Vec<Insight> {
    let matches = &snapshot.matches;
    let mut out = Vec::new();
    let tier = snapshot
        .peers
        .as_ref()
        .and_then(|peers| peers.tier_name.clone());

    // Rank movement over the week that ends at the newest ranked match.
    let all_points = rank_trajectory(matches);
    let trajectory = &all_points[season_start(&all_points)..];
    if let Some(latest) = trajectory.last() {
        let since = latest.start_time - 7 * 86_400;
        if let Some(first) = trajectory.iter().find(|point| point.start_time >= since)
            && first.step != latest.step
            && trajectory.len() >= 2
        {
            let games = trajectory
                .iter()
                .filter(|point| point.start_time >= since)
                .count();
            let now = badge_label(latest.badge, &snapshot.tier_names);
            let was = badge_label(first.badge, &snapshot.tier_names);
            let up = latest.step > first.step;
            out.push(Insight {
                kind: Kind::RankMove,
                tone: if up { Tone::Good } else { Tone::Focus },
                title: if up {
                    format!("Up to {now} this week.")
                } else {
                    format!("Down to {now} this week.")
                },
                detail: format!("From {was}, over {games} ranked matches."),
                hero_id: None,
                weight: (latest.step as f64 - first.step as f64).abs() * 10.0,
            });
        }
    }

    // Streaks. A loss streak is normalised with the player's own history.
    if let Some(streak) = current_streak(matches)
        && streak.length >= 3
    {
        let best = longest_run(matches, streak.win);
        if streak.win {
            let mut detail = if streak.length >= best {
                format!("Your longest run in the last {} matches.", matches.len())
            } else {
                format!("Your best run in the last {} matches is {best}.", matches.len())
            };
            if let Some(delta) = matches
                .iter()
                .find(|summary| summary.win.is_some())
                .and_then(|summary| summary.ranked_delta)
                .filter(|delta| *delta > 0)
            {
                detail.push_str(&format!(" The last win was worth +{delta} rank points."));
            }
            out.push(Insight {
                kind: Kind::WinStreak,
                tone: Tone::Good,
                title: format!("{} wins in a row.", streak.length),
                detail,
                hero_id: None,
                weight: streak.length as f64,
            });
        } else {
            out.push(Insight {
                kind: Kind::LossStreak,
                tone: Tone::Focus,
                title: format!("{} losses in a row.", streak.length),
                detail: after_losses(matches, streak.length),
                hero_id: None,
                weight: f64::MAX,
            });
        }
    }

    // Form: the newest 20 against the rest of the window.
    let window_form = form(matches);
    if window_form.comparable()
        && let (Some(recent), Some(before)) = (
            window_form.recent.win_rate(),
            window_form.before.win_rate(),
        )
    {
        let gap = recent - before;
        let n = window_form.recent.decided();
        let record = format!(
            "{} W {} L over your last {}.",
            window_form.recent.wins, window_form.recent.losses, window_form.recent.matches
        );
        if gap >= GAP_POINTS {
            out.push(Insight {
                kind: Kind::FormUp,
                tone: Tone::Good,
                title: format!("Winning {recent:.0}% lately, up from {before:.0}%."),
                detail: record,
                hero_id: None,
                weight: gap * (n as f64).sqrt(),
            });
        } else if gap <= -GAP_POINTS {
            out.push(Insight {
                kind: Kind::FormDown,
                tone: Tone::Focus,
                title: format!("Winning {recent:.0}% lately, down from {before:.0}%."),
                detail: record,
                hero_id: None,
                weight: -gap * (n as f64).sqrt(),
            });
        }
    }

    // Deaths per match, recent against before.
    if window_form.recent.matches >= MIN_SAMPLE && window_form.before.matches >= MIN_SAMPLE {
        let recent = window_form.recent.deaths_per_match();
        let before = window_form.before.deaths_per_match();
        if before > 0.0 {
            let ratio = recent / before;
            let peer = snapshot
                .peers
                .as_ref()
                .map(|peers| peers.overall.deaths_per_match)
                .filter(|value| *value > 0.0);
            let peer_text = match (&tier, peer) {
                (Some(tier), Some(value)) => format!(" {tier} players average {value:.1}."),
                _ => String::new(),
            };
            if ratio >= 1.15 {
                out.push(Insight {
                    kind: Kind::DeathsUp,
                    tone: Tone::Focus,
                    title: format!("Dying more: {recent:.1} a match over your last 20."),
                    detail: format!("Before that it was {before:.1}.{peer_text}"),
                    hero_id: None,
                    weight: (ratio - 1.0) * 100.0 * (window_form.recent.matches as f64).sqrt(),
                });
            } else if ratio <= 0.87 {
                out.push(Insight {
                    kind: Kind::DeathsDown,
                    tone: Tone::Good,
                    title: format!("Fewer deaths: {recent:.1} a match over your last 20."),
                    detail: format!("Down from {before:.1}.{peer_text}"),
                    hero_id: None,
                    weight: (1.0 - ratio) * 100.0,
                });
            }
        }
    }

    // Heroes: comfort and trap picks from career rows the player still plays.
    let career_rate = snapshot.career.win_rate_percent;
    let recent_cutoff = snapshot.generated_unix - 60 * 86_400;
    let active = |hero: &HeroPerformance| {
        hero.matches_played >= HERO_SAMPLE
            && (snapshot.generated_unix == 0 || hero.last_played_unix >= recent_cutoff)
    };
    let peer_line = |hero: &HeroPerformance| -> String {
        let Some(peers) = &snapshot.peers else {
            return String::new();
        };
        match (peers.hero(hero.hero_id), &tier) {
            (Some(bench), Some(tier)) if bench.matches >= 100 => format!(
                " {tier} {} players win {:.0}%.",
                hero_name(hero),
                bench.win_rate_percent
            ),
            _ => String::new(),
        }
    };
    let mut best: Option<(f64, &HeroPerformance)> = None;
    let mut worst: Option<(f64, &HeroPerformance)> = None;
    for hero in snapshot.heroes.iter().filter(|hero| active(hero)) {
        let gap = hero.win_rate_percent - career_rate;
        let score = gap * (hero.matches_played as f64).sqrt();
        if gap >= GAP_POINTS && best.is_none_or(|(top, _)| score > top) {
            best = Some((score, hero));
        }
        if gap <= -GAP_POINTS && worst.is_none_or(|(low, _)| score < low) {
            worst = Some((score, hero));
        }
    }
    if let Some((score, hero)) = best {
        out.push(Insight {
            kind: Kind::ComfortHero,
            tone: Tone::Good,
            title: format!("{} carries you.", hero_name(hero)),
            detail: format!(
                "{:.0}% over {} matches, against {career_rate:.0}% overall.{}",
                hero.win_rate_percent,
                hero.matches_played,
                peer_line(hero)
            ),
            hero_id: Some(hero.hero_id),
            weight: score,
        });
    }
    if let Some((score, hero)) = worst {
        out.push(Insight {
            kind: Kind::TrapHero,
            tone: Tone::Focus,
            title: format!("{} is costing you.", hero_name(hero)),
            detail: format!(
                "{:.0}% over {} matches, against {career_rate:.0}% overall.{}",
                hero.win_rate_percent,
                hero.matches_played,
                peer_line(hero)
            ),
            hero_id: Some(hero.hero_id),
            weight: -score,
        });
    }

    // Long sittings: games three and later against the first two.
    let mut early = Aggregate::default();
    let mut late = Aggregate::default();
    for session in sessions(matches, SESSION_GAP_SECONDS) {
        for (position, index) in session.indices.iter().rev().enumerate() {
            let one = Aggregate::of(std::iter::once(&matches[*index]));
            let side = if position < 2 { &mut early } else { &mut late };
            side.matches += one.matches;
            side.wins += one.wins;
            side.losses += one.losses;
        }
    }
    if early.decided() >= MIN_SAMPLE
        && late.decided() >= MIN_SAMPLE
        && let (Some(first), Some(later)) = (early.win_rate(), late.win_rate())
        && first - later >= GAP_POINTS
    {
        out.push(Insight {
            kind: Kind::LongSessions,
            tone: Tone::Focus,
            title: "Long sittings cost you.".to_string(),
            detail: format!(
                "From the third game of a sitting you win {later:.0}% ({} games), against \
                 {first:.0}% in the first two.",
                late.decided()
            ),
            hero_id: None,
            weight: (first - later) * (late.decided() as f64).sqrt(),
        });
    }

    // Peer gap: the most played recent hero that trails the player's tier.
    if let (Some(peers), Some(tier)) = (&snapshot.peers, &tier) {
        let recent = &matches[..matches.len().min(RECENT_MATCHES)];
        let mut trailing: Option<(f64, &HeroPerformance, f64)> = None;
        for hero in snapshot.heroes.iter().filter(|hero| active(hero)) {
            let played = recent
                .iter()
                .filter(|summary| summary.hero_id == hero.hero_id)
                .count();
            let Some(bench) = peers.hero(hero.hero_id).filter(|bench| bench.matches >= 100)
            else {
                continue;
            };
            let gap = hero.win_rate_percent - bench.win_rate_percent;
            if played >= 3 && gap < 0.0 && trailing.is_none_or(|(low, _, _)| gap < low) {
                trailing = Some((gap, hero, bench.win_rate_percent));
            }
        }
        if let Some((gap, hero, peer)) = trailing {
            out.push(Insight {
                kind: Kind::PeerGap,
                tone: Tone::Focus,
                title: format!("{} trails your rank.", hero_name(hero)),
                detail: format!(
                    "{:.0}% over {} matches, against {peer:.0}% for {tier} players.",
                    hero.win_rate_percent, hero.matches_played
                ),
                hero_id: Some(hero.hero_id),
                weight: -gap,
            });
        }
    }

    // The last sitting, always available as a fallback headline.
    if let Some(last) = sessions(matches, SESSION_GAP_SECONDS).first() {
        let mut title = format!("Last sitting: {} W {} L", last.wins, last.losses);
        if last.ranked > 0 && last.rank_delta != 0 {
            title.push_str(&format!(", {:+} rank points", last.rank_delta));
        }
        title.push('.');
        out.push(Insight {
            kind: Kind::LastSession,
            tone: if last.wins > last.losses {
                Tone::Good
            } else {
                Tone::Neutral
            },
            title,
            detail: format!(
                "{} {} on {}.",
                last.indices.len(),
                if last.indices.len() == 1 { "match" } else { "matches" },
                day_label(last.start_time, snapshot.generated_unix)
            ),
            hero_id: None,
            weight: 0.0,
        });
    }

    out
}

/// Pick the headline, the next focus and the notes.
pub fn brief(snapshot: &StatsSnapshot) -> Brief {
    let mut all = insights(snapshot);
    let take = |all: &mut Vec<Insight>, kind: Kind| -> Option<Insight> {
        let index = all.iter().position(|insight| insight.kind == kind)?;
        Some(all.remove(index))
    };

    let mut changed = None;
    for kind in [
        Kind::RankMove,
        Kind::WinStreak,
        Kind::FormUp,
        Kind::FormDown,
        Kind::LastSession,
    ] {
        if let Some(found) = take(&mut all, kind) {
            changed = Some(found);
            break;
        }
    }

    let next = {
        let mut best: Option<usize> = None;
        for (index, insight) in all.iter().enumerate() {
            if insight.tone == Tone::Focus
                && best.is_none_or(|top| insight.weight > all[top].weight)
            {
                best = Some(index);
            }
        }
        best.map(|index| all.remove(index))
    };

    let mut also: Vec<Insight> = all
        .into_iter()
        .filter(|insight| insight.kind != Kind::LastSession)
        .collect();
    also.sort_by(|left, right| {
        let rank = |tone: Tone| match tone {
            Tone::Good => 0,
            Tone::Focus => 1,
            Tone::Neutral => 2,
        };
        rank(left.tone)
            .cmp(&rank(right.tone))
            .then(right.weight.total_cmp(&left.weight))
    });
    also.truncate(3);

    Brief {
        changed,
        next,
        also,
    }
}

/// What the player's history says about the game after a loss streak.
fn after_losses(matches: &[MatchSummary], length: usize) -> String {
    let decided: Vec<bool> = matches.iter().rev().filter_map(|summary| summary.win).collect();
    let threshold = length.min(3);
    let mut games = 0u64;
    let mut wins = 0u64;
    let mut run = 0usize;
    for win in &decided {
        if run >= threshold {
            games += 1;
            if *win {
                wins += 1;
            }
        }
        run = if *win { 0 } else { run + 1 };
    }
    let overall = Aggregate::of(matches).win_rate().unwrap_or(0.0);
    if games >= 10 {
        let rate = 100.0 * wins as f64 / games as f64;
        if overall - rate >= GAP_POINTS {
            return format!(
                "After {threshold} straight losses you win {rate:.0}% of the next game ({games} \
                 games), against {overall:.0}% overall. A break may be worth more than a queue."
            );
        }
        return format!(
            "After {threshold} straight losses you still win {rate:.0}% of the next game \
             ({games} games). Your usual is {overall:.0}%."
        );
    }
    let runs = runs_at_least(matches, false, length);
    format!(
        "Runs like this happen: your last {} matches had {runs} of {length} or more.",
        matches.len()
    )
}

fn hero_name(hero: &HeroPerformance) -> String {
    hero.hero_name
        .clone()
        .unwrap_or_else(|| format!("Hero {}", hero.hero_id))
}

/// `today`, `yesterday`, a weekday within the week, else `12 Nov`. Local time,
/// relative to `now` (unix seconds; 0 means the system clock).
pub fn day_label(unix: i64, now: i64) -> String {
    let now = if now > 0 {
        now
    } else {
        chrono::Utc::now().timestamp()
    };
    let (Some(at), Some(today)) = (
        Local.timestamp_opt(unix, 0).single(),
        Local.timestamp_opt(now, 0).single(),
    ) else {
        return String::new();
    };
    let days = today.date_naive().signed_duration_since(at.date_naive()).num_days();
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => at.format("%A").to_string(),
        _ if at.year() == today.year() => format!("{} {}", at.day(), at.format("%b")),
        _ => format!("{} {} {}", at.day(), at.format("%b"), at.year()),
    }
}

/// `20:52` in local time.
pub fn clock_label(unix: i64) -> String {
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map(|at| at.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// `12 Oct` in local time.
pub fn date_label(unix: i64) -> String {
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map(|at| format!("{} {}", at.day(), at.format("%b")))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{MatchOutcome, MatchSummary};

    fn game(start: i64, win: Option<bool>, badge: Option<u32>, delta: Option<i32>) -> MatchSummary {
        let outcome = match win {
            Some(true) => MatchOutcome::Win,
            Some(false) => MatchOutcome::Loss,
            None => MatchOutcome::NotScored,
        };
        MatchSummary {
            match_id: start as u64,
            hero_id: 1,
            hero_name: Some("Haze".to_string()),
            start_time: start,
            started_at: String::new(),
            duration_seconds: 1800,
            game_mode: 1,
            match_mode: 4,
            player_team: 0,
            kills: 5,
            deaths: 4,
            assists: 7,
            kda: 3.0,
            net_worth: 30_000,
            last_hits: 100,
            denies: 5,
            outcome,
            win,
            abandoned_seconds: None,
            ranked_badge: badge,
            ranked_delta: delta,
        }
    }

    /// Newest first, two hours apart, from a result string oldest to newest.
    fn history(results: &str) -> Vec<MatchSummary> {
        let mut out: Vec<MatchSummary> = results
            .chars()
            .enumerate()
            .map(|(index, c)| {
                let win = match c {
                    'W' => Some(true),
                    'L' => Some(false),
                    _ => None,
                };
                game(1_000_000 + index as i64 * 2000, win, None, None)
            })
            .collect();
        out.reverse();
        out
    }

    #[test]
    fn badge_steps_round_trip() {
        assert_eq!(badge_step(11), Some(1));
        assert_eq!(badge_step(115), Some(65));
        assert_eq!(badge_step(116), Some(66));
        assert_eq!(badge_step(0), None);
        assert_eq!(badge_step(110), None);
        assert_eq!(badge_step(117), None);
        for step in 1..=66 {
            assert_eq!(badge_step(step_badge(step)), Some(step));
        }
    }

    #[test]
    fn badge_label_uses_tier_names() {
        let mut names = BTreeMap::new();
        names.insert(11, "Eternus".to_string());
        assert_eq!(badge_label(115, &names), "Eternus 5");
        assert_eq!(badge_label(0, &names), "Unranked");
        assert_eq!(badge_label(23, &names), "Tier 2 3");
    }

    #[test]
    fn streaks_skip_unscored_rows() {
        let matches = history("LWWW-W");
        assert_eq!(
            current_streak(&matches),
            Some(Streak {
                win: true,
                length: 4
            })
        );
        assert_eq!(longest_run(&matches, true), 4);
        assert_eq!(longest_run(&matches, false), 1);
        assert_eq!(runs_at_least(&history("LLLWLLLL"), false, 3), 2);
        assert_eq!(current_streak(&[]), None);
    }

    #[test]
    fn rolling_rate_needs_a_full_window() {
        let matches = history("WWLL");
        let rolling = rolling_win_rate(&matches, 2);
        let rates: Vec<f64> = rolling.iter().map(|(_, rate)| *rate).collect();
        assert_eq!(rates, vec![100.0, 50.0, 0.0]);
        assert!(rolling_win_rate(&matches, 5).is_empty());
    }

    #[test]
    fn sessions_split_on_long_breaks() {
        let mut matches = vec![
            game(100_000, Some(true), Some(113), Some(350)),
            game(102_000, Some(false), Some(113), Some(-350)),
            game(120_000, Some(true), Some(114), Some(400)),
        ];
        matches.reverse();
        let found = sessions(&matches, SESSION_GAP_SECONDS);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].indices, vec![0]);
        assert_eq!(found[1].indices, vec![1, 2]);
        assert_eq!(found[1].rank_delta, 0);
        assert_eq!((found[1].wins, found[1].losses), (1, 1));
    }

    #[test]
    fn trajectory_is_oldest_first_and_ranked_only() {
        let mut matches = vec![
            game(1, Some(true), Some(112), Some(350)),
            game(2, Some(true), None, None),
            game(3, Some(false), Some(113), Some(-350)),
        ];
        matches.reverse();
        let points = rank_trajectory(&matches);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].badge, 112);
        assert_eq!(points[1].step, 63);
    }

    #[test]
    fn season_start_follows_the_last_reset() {
        let point = |start: i64, badge: u32| RankPoint {
            match_id: start as u64,
            start_time: start,
            badge,
            step: badge_step(badge).unwrap_or(0),
            delta: None,
            win: None,
        };
        let points = vec![point(1, 116), point(2, 116), point(3, 103), point(4, 111)];
        assert_eq!(season_start(&points), 2);
        let steady = vec![point(1, 112), point(2, 111), point(3, 116)];
        assert_eq!(season_start(&steady), 0);
    }

    #[test]
    fn form_compares_recent_with_before() {
        let mut results = String::new();
        results.push_str(&"WL".repeat(15));
        results.push_str(&"WWWWL".repeat(4));
        let matches = history(&results);
        let found = form(&matches);
        assert!(found.comparable());
        assert_eq!(found.recent.win_rate(), Some(80.0));
        assert_eq!(found.before.win_rate(), Some(50.0));
    }

    #[test]
    fn after_losses_needs_ten_games_before_quoting_a_rate() {
        let text = after_losses(&history("WLLL"), 3);
        assert!(text.starts_with("Runs like this happen"), "{text}");
        let long = history(&"LLLW".repeat(12));
        let text = after_losses(&long, 3);
        assert!(text.contains("of the next game (12 games)"), "{text}");
    }
}
