//! `dah stats`: read-only player statistics from the public Deadlock API.
//!
//! This module is not wired into `main.rs` yet. Each `run_*` function takes a
//! plain argument struct so the clap surface can be added without changing the
//! fetch or formatting code.
//!
//! Every request goes to `https://api.deadlock-api.com`. No key is required;
//! `DEADLOCK_API_KEY` is read by the client and only raises rate limits.

use serde_json::json;

use deadlock_core::stats::{
    CareerTotals, HeroPerformance, LEADERBOARD_REGIONS, MATCH_WINDOW_MAX, MatchOutcome,
    MatchSummary, StatsClient, StatsSnapshot, career_totals, window_stats,
};

use crate::error::{CliError, Result};
use crate::util::print_json;

pub struct PlayerArgs {
    pub account_id: u64,
    /// Newest matches to keep for the window, recent form and averages.
    pub recent: usize,
    pub json: bool,
}

pub struct MatchesArgs {
    pub account_id: u64,
    pub limit: usize,
    pub json: bool,
}

pub struct HeroesArgs {
    pub account_id: u64,
    pub json: bool,
}

pub struct HeatmapArgs {
    pub account_id: u64,
    pub json: bool,
}

pub struct LeaderboardArgs {
    pub region: String,
    pub limit: usize,
    pub json: bool,
}

/// `stats player <account-id>`: profile, rank, career totals, recent window.
pub fn run_player(args: &PlayerArgs) -> Result<()> {
    ensure_account(args.account_id)?;
    ensure_window(args.recent, "Recent match window")?;

    let client = StatsClient::new();
    let snapshot = client
        .fetch_snapshot(args.account_id, args.recent)
        .map_err(stats_error)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": client.base_url(),
            "apiKeyUsed": client.has_api_key(),
            "snapshot": snapshot,
        }));
        return Ok(());
    }

    println!("{}", format_snapshot(&snapshot));
    println!();
    println!(
        "Source: {} (public, no key needed). Read only.",
        client.base_url()
    );

    Ok(())
}

/// `stats matches <account-id>`: the recent match list with derived window stats.
pub fn run_matches(args: &MatchesArgs) -> Result<()> {
    ensure_account(args.account_id)?;
    ensure_window(args.limit, "Match window")?;

    let client = StatsClient::new();
    let mut matches = client
        .fetch_matches(args.account_id, args.limit)
        .map_err(stats_error)?;

    if let Ok(names) = client.fetch_hero_names() {
        for summary in &mut matches {
            summary.hero_name = names.get(&summary.hero_id).cloned();
        }
    }

    let window = window_stats(&matches);

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": client.base_url(),
            "accountId": args.account_id,
            "window": window,
            "matches": matches,
        }));
        return Ok(());
    }

    println!(
        "Matches for {} (newest {}, newest first)",
        args.account_id, args.limit
    );
    println!("{}", format_window(&window));
    println!();
    for summary in &matches {
        println!("{}", format_match(summary));
    }
    println!();
    println!(
        "The API returns the full stored history in one response; this client keeps the newest {}.",
        args.limit
    );

    Ok(())
}

/// `stats heroes <account-id>`: lifetime per-hero rows sorted by matches played.
pub fn run_heroes(args: &HeroesArgs) -> Result<()> {
    ensure_account(args.account_id)?;

    let client = StatsClient::new();
    let mut heroes = client
        .fetch_hero_stats(args.account_id)
        .map_err(stats_error)?;

    if let Ok(names) = client.fetch_hero_names() {
        for hero in &mut heroes {
            hero.hero_name = names.get(&hero.hero_id).cloned();
        }
    }

    heroes.sort_by(|left, right| {
        right
            .matches_played
            .cmp(&left.matches_played)
            .then(left.hero_id.cmp(&right.hero_id))
    });
    let career = career_totals(&heroes);

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": client.base_url(),
            "accountId": args.account_id,
            "career": career,
            "heroes": heroes,
        }));
        return Ok(());
    }

    println!("Heroes for account {}", args.account_id);
    println!(
        "Career: {} matches, {} wins, {} losses, {:.1}% win rate (95% CI {}), {:.2} KDA",
        thousands(career.matches_played),
        thousands(career.wins),
        thousands(career.losses),
        career.win_rate_percent,
        format_interval(career.win_rate_low_percent, career.win_rate_high_percent),
        career.kda
    );
    println!();
    println!(
        "{:<18} {:>8} {:>7} {:>15} {:>6} {:>14} {:>9} {:>9}  {}",
        "Hero", "Matches", "Win%", "95% CI", "KDA", "K/D/A", "Dmg/min", "Net/min", "Last played"
    );

    for hero in &heroes {
        println!(
            "{:<18} {:>8} {:>6.1}% {:>15} {:>6.2} {:>14} {:>9} {:>9}  {}",
            hero_label(hero),
            thousands(hero.matches_played),
            hero.win_rate_percent,
            format_interval(hero.win_rate_low_percent, hero.win_rate_high_percent),
            hero.kda,
            format_kda(hero.kills, hero.deaths, hero.assists),
            thousands(hero.damage_per_minute.round() as u64),
            thousands(hero.net_worth_per_minute.round() as u64),
            short_date(&hero.last_played)
        );
    }

    if heroes.is_empty() {
        println!("No hero rows returned. The account may have no parsed matches.");
    }

    Ok(())
}

/// `stats heatmap <account-id>`: aggregate kill and death positions.
///
/// This is the only heatmap the public API exposes synchronously. Per-match
/// position timelines would need an asynchronous demo query and are not part of
/// this client.
pub fn run_heatmap(args: &HeatmapArgs) -> Result<()> {
    ensure_account(args.account_id)?;

    let client = StatsClient::new();
    let heatmap = client
        .fetch_kill_heatmap(args.account_id)
        .map_err(stats_error)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": client.base_url(),
            "heatmap": heatmap,
        }));
        return Ok(());
    }

    println!(
        "Kill and death heatmap for {} ({})",
        args.account_id, heatmap.window
    );
    println!(
        "Cells: {}  Kills: {}  Deaths: {}",
        thousands(heatmap.cells as u64),
        thousands(heatmap.kills),
        thousands(heatmap.deaths)
    );

    let mut points = heatmap.points.clone();
    points.sort_by(|left, right| {
        let left_events = left.kills + left.deaths;
        let right_events = right.kills + right.deaths;
        right_events
            .cmp(&left_events)
            .then(right.kills.cmp(&left.kills))
            .then(left.position_x.cmp(&right.position_x))
            .then(left.position_y.cmp(&right.position_y))
    });

    println!();
    println!("Busiest cells:");
    for point in points.iter().take(10) {
        println!(
            "  ({:>6}, {:>6})  team {}  {} kills  {} deaths",
            point.position_x, point.position_y, point.killer_team, point.kills, point.deaths
        );
    }

    println!();
    println!(
        "Positions are map world units. The map asset reports a radius of 10752. \
         This aggregate covers the last 30 days, not one selected match."
    );

    Ok(())
}

/// `stats leaderboard <region>`: top entries for one region.
pub fn run_leaderboard(args: &LeaderboardArgs) -> Result<()> {
    if !LEADERBOARD_REGIONS.contains(&args.region.as_str()) {
        return Err(CliError::new(
            "invalid_region",
            format!(
                "Unknown leaderboard region {:?}. Use one of {}.",
                args.region,
                LEADERBOARD_REGIONS.join(", ")
            ),
        ));
    }
    if args.limit == 0 {
        return Err(CliError::new(
            "invalid_limit",
            "Leaderboard limit must be 1 or greater.",
        ));
    }

    let client = StatsClient::new();
    let board = client
        .fetch_leaderboard(&args.region, args.limit)
        .map_err(stats_error)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": client.base_url(),
            "leaderboard": board,
        }));
        return Ok(());
    }

    println!(
        "{} leaderboard (top {} of the current season)",
        board.region, args.limit
    );
    for entry in &board.entries {
        let place = entry
            .rank
            .map(|rank| format!("#{rank}"))
            .unwrap_or_else(|| "#?".to_string());
        let name = entry.account_name.as_deref().unwrap_or("unknown");
        let ids = entry
            .possible_account_ids
            .iter()
            .take(3)
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let ids = if entry.possible_account_ids.len() > 3 {
            format!("{ids}, +{} more", entry.possible_account_ids.len() - 3)
        } else {
            ids
        };
        let heroes = entry
            .top_hero_ids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        println!("{place:<5} {name}  accounts: {ids}  heroes: {heroes}");
    }
    println!();
    println!(
        "Steam names are not unique, so an entry can list several possible account ids. \
         Pass the id you want to `Sigil cli stats player`."
    );

    Ok(())
}

fn ensure_account(account_id: u64) -> Result<()> {
    if account_id == 0 {
        return Err(CliError::new(
            "invalid_account",
            "Account id must be a SteamID3 greater than zero. A leaderboard entry shows candidate \
             ids, and `Sigil cli stats leaderboard` prints them.",
        ));
    }
    Ok(())
}

fn ensure_window(limit: usize, label: &str) -> Result<()> {
    if limit == 0 {
        return Err(CliError::new(
            "invalid_limit",
            format!("{label} must be 1 or greater."),
        ));
    }
    if limit > MATCH_WINDOW_MAX {
        return Err(CliError::new(
            "invalid_limit",
            format!(
                "{label} must be {MATCH_WINDOW_MAX} or fewer. The API returns the full history in \
                 one response, so a larger window adds no data and only slows the screen."
            ),
        ));
    }
    Ok(())
}

fn stats_error(error: deadlock_core::Error) -> CliError {
    CliError::new("stats", error.to_string())
}

fn format_snapshot(snapshot: &StatsSnapshot) -> String {
    let mut lines = Vec::new();
    let profile = &snapshot.profile;

    let mut header = format!("{} (account {})", profile.persona_name, profile.account_id);
    if !profile.real_name.as_deref().unwrap_or("").is_empty() {
        header.push_str(&format!(
            " / {}",
            profile.real_name.as_deref().unwrap_or("")
        ));
    }
    lines.push(header);

    if !profile.profile_url.is_empty() {
        lines.push(format!("Steam: {}", profile.profile_url));
    }
    if !profile.last_updated.is_empty() {
        lines.push(format!(
            "Steam profile refreshed: {}  Friends: {}  Matches last 30 days: {}",
            short_date(&profile.last_updated),
            profile.friend_count,
            thousands(profile.matches_played_last_30d)
        ));
    }

    lines.push(format!("Rank: {}", format_rank(snapshot)));
    lines.push(format!("Career: {}", format_career(&snapshot.career)));
    lines.push(format!(
        "Heroes: {} played, led by {}",
        snapshot.career.heroes_played,
        snapshot
            .heroes
            .first()
            .map(hero_label)
            .unwrap_or_else(|| "no parsed hero rows".to_string())
    ));

    lines.push(String::new());
    lines.push(format!(
        "Recent window: newest {} match(es)",
        snapshot.matches.len()
    ));
    lines.push(format_window(&snapshot.window));

    lines.push(String::new());
    lines.push("Top heroes by matches:".to_string());
    for hero in snapshot.heroes.iter().take(5) {
        lines.push(format!(
            "  {:<18} {:>5} matches  {:>6.1}% win  {:>15}  {:>6.2} KDA  {}",
            hero_label(hero),
            thousands(hero.matches_played),
            hero.win_rate_percent,
            format_interval(hero.win_rate_low_percent, hero.win_rate_high_percent),
            hero.kda,
            short_date(&hero.last_played)
        ));
    }
    if snapshot.heroes.is_empty() {
        lines.push("  No hero rows returned.".to_string());
    }

    lines.push(String::new());
    lines.push("Latest matches:".to_string());
    for summary in snapshot.matches.iter().take(10) {
        lines.push(format!("  {}", format_match(summary)));
    }
    if snapshot.matches.is_empty() {
        lines.push("  No matches in the window.".to_string());
    }

    lines.join("\n")
}

fn format_rank(snapshot: &StatsSnapshot) -> String {
    let rank = &snapshot.rank;
    let mut text = if rank.badge == 0 {
        let tier = rank.tier_name.as_deref().unwrap_or("unknown tier");
        format!("unranked ({tier}, badge 0)")
    } else {
        let tier = rank.tier_name.as_deref().unwrap_or("unknown tier");
        format!("{tier} {}, badge {}", rank.subrank, rank.badge)
    };

    if let Some(last) = &rank.last_match {
        let change = last
            .desired_progress_change
            .map(format_signed)
            .unwrap_or_else(|| "unknown".to_string());
        text.push_str(&format!(
            "  |  last ranked {}  progress {}  match {}",
            short_date(&last.started_at),
            change,
            last.match_id
        ));
    }

    if let Some(benchmark) = &rank.benchmark {
        if benchmark.percentile >= 99.0 {
            text.push_str(&format!(
                "  |  top {:.2}% of {} ranked players",
                100.0 - benchmark.percentile,
                thousands(benchmark.ranked_players)
            ));
        } else {
            text.push_str(&format!(
                "  |  ahead of {:.1}% of {} ranked players",
                benchmark.percentile,
                thousands(benchmark.ranked_players)
            ));
        }
    }

    text
}

fn format_career(career: &CareerTotals) -> String {
    format!(
        "{} matches, {} W / {} L ({:.1}%, 95% CI {}) {} K/D/A at {:.2} KDA, {} damage ({} objective), avg {}",
        thousands(career.matches_played),
        thousands(career.wins),
        thousands(career.losses),
        career.win_rate_percent,
        format_interval(career.win_rate_low_percent, career.win_rate_high_percent),
        format_kda(career.kills, career.deaths, career.assists),
        career.kda,
        thousands(career.player_damage),
        thousands(career.objective_damage),
        format_duration(career.average_match_seconds.round() as u64)
    )
}

fn format_window(window: &deadlock_core::stats::WindowStats) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "  Record: {} W / {} L ({:.1}%, 95% CI {})  Unscored: {}  K/D/A: {} at {:.2} KDA",
        window.wins,
        window.losses,
        window.win_rate_percent,
        format_interval(window.win_rate_low_percent, window.win_rate_high_percent),
        window.unscored,
        format_kda(window.kills, window.deaths, window.assists),
        window.kda
    ));
    lines.push(format!(
        "  Average duration: {}  Average net worth: {}",
        format_duration(window.average_duration_seconds.round() as u64),
        thousands(window.average_net_worth.round() as u64)
    ));

    if window.recent_form.is_empty() {
        lines.push("  Recent form: no decisive match in the window.".to_string());
    } else {
        let strip = window
            .recent_form
            .iter()
            .map(|entry| if entry.win { "W" } else { "L" })
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!(
            "  Recent form (last {}): {}  {:.1}%",
            window.recent_form.len(),
            strip,
            window.recent_win_rate_percent
        ));
    }

    lines.join("\n")
}

fn format_match(summary: &MatchSummary) -> String {
    let result = match summary.outcome {
        MatchOutcome::Win => "W",
        MatchOutcome::Loss => "L",
        MatchOutcome::Penalized => "P",
        MatchOutcome::PenalizedParty => "PP",
        MatchOutcome::NotScored => "-",
        MatchOutcome::Invalid => "?",
        MatchOutcome::Unknown => "?",
    };

    let badge = summary
        .ranked_badge
        .map(|badge| format!("badge {badge}"))
        .unwrap_or_else(|| "unranked".to_string());

    format!(
        "{}  {:<16} {:<2}  {:<11} {:>6.2} KDA  {:>8}  {:>6}  {}  {}",
        short_date(&summary.started_at),
        summarize_label(summary),
        result,
        format_kda(
            summary.kills as u64,
            summary.deaths as u64,
            summary.assists as u64,
        ),
        summary.kda,
        thousands(summary.net_worth),
        format_duration(summary.duration_seconds as u64),
        badge,
        summary.match_id
    )
}

fn summarize_label(summary: &MatchSummary) -> String {
    summary
        .hero_name
        .clone()
        .unwrap_or_else(|| format!("hero {}", summary.hero_id))
}

fn hero_label(hero: &HeroPerformance) -> String {
    hero.hero_name
        .clone()
        .unwrap_or_else(|| format!("hero {}", hero.hero_id))
}

fn format_kda(kills: u64, deaths: u64, assists: u64) -> String {
    format!("{kills}/{deaths}/{assists}")
}

fn format_signed(value: i64) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}

fn format_interval(low: f64, high: f64) -> String {
    format!("{low:.1}-{high:.1}%")
}

fn format_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    if minutes >= 60 {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m {remainder:02}s")
    }
}

fn short_date(value: &str) -> String {
    let trimmed = value.trim();
    let short: String = trimmed.chars().take(16).collect();
    short.replace('T', " ")
}

fn thousands(value: u64) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    output
}
