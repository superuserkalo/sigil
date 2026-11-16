//! `dah demo`: import local Source 2 demos and read the merged summary.
//!
//! The importer never writes to a game install. A demo is copied into
//! `<workspace>/matches/<match-id>/` and the derived files stay there.
//!
//! Text output is deliberately compact. `--json` returns the stored summary
//! and provenance so the desktop app does not have to parse terminal text.

use std::fmt::Display;
use std::path::PathBuf;

use serde_json::json;

use deadlock_core::demo::{
    self, Confidence, CoverageState, FieldValue, ImportResult, MatchSummaryDoc,
};
use deadlock_core::review;

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::{format_bytes, lossy, print_json};

pub struct ImportArgs {
    /// A Deadlock `.dem` recorded by the game.
    pub file: PathBuf,
    /// Match id for the store folder.
    pub id: Option<String>,
    /// Workspace directory. Defaults to `DAH_WORKSPACE_DIR` or `<data>/workspace`.
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct ListArgs {
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct ShowArgs {
    pub id: String,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct HeatmapArgs {
    pub id: String,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub struct BackendsArgs {
    pub json: bool,
}

/// `demo import <file>`: parse, store and summarize one demo.
pub fn run_import(args: &ImportArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let result = demo::import_demo(&workspace, &args.file, args.id.as_deref())
        .map_err(|error| CliError::new("demo_import", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "matchId": result.match_id,
            "directory": lossy(&result.directory),
            "existed": result.existed,
            "reviewPreserved": result.review_preserved,
            "meta": result.meta,
            "summary": result.summary,
            "provenance": result.provenance,
        }));
        return Ok(());
    }

    print_import(&result);

    Ok(())
}

/// `demo list`: one row per imported match.
pub fn run_list(args: &ListArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let list = demo::list_matches(&workspace).map_err(core_error)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "workspace": lossy(&list.workspace),
            "count": list.matches.len(),
            "skipped": list.skipped,
            "matches": list.matches,
        }));
        return Ok(());
    }

    println!(
        "Matches in {} ({})",
        lossy(&list.workspace),
        if list.matches.is_empty() {
            "none".to_string()
        } else {
            format!("{}", list.matches.len())
        }
    );

    if list.matches.is_empty() {
        println!("No imported matches. Run `dah demo import <file.dem>`.");
        if list.skipped > 0 {
            println!(
                "{} folder(s) under matches/ had no readable meta.json or summary.json.",
                list.skipped
            );
        }
        return Ok(());
    }

    for item in &list.matches {
        let map = item.map.as_deref().unwrap_or("-");
        let duration = item
            .duration_seconds
            .map(format_duration)
            .unwrap_or_else(|| "-".to_string());
        let result = item.result.as_deref().unwrap_or("-");
        println!(
            "  {}  {:<20} {:<11} {:<16} {:>8}  {:>2} players  {:>3} events  {:>3} cells  {}",
            short_date(&item.updated_at),
            item.match_id,
            item.source,
            map,
            duration,
            item.player_count,
            item.event_count,
            item.heatmap_cells,
            result
        );
    }

    if list.skipped > 0 {
        println!(
            "{} folder(s) under matches/ had no readable meta.json or summary.json.",
            list.skipped
        );
    }

    Ok(())
}

/// `demo show <id>`: the merged summary and its coverage.
pub fn run_show(args: &ShowArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let summary = demo::read_summary(&workspace, &args.id).map_err(core_error)?;
    let provenance = demo::read_provenance(&workspace, &args.id).ok();

    if args.json {
        print_json(&json!({
            "ok": true,
            "summary": summary,
            "provenance": provenance,
        }));
        return Ok(());
    }

    print_summary(&summary, provenance.as_ref());

    Ok(())
}

/// `demo heatmap <id>`: cells from real positions, or an explicit absence.
pub fn run_heatmap(args: &HeatmapArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let summary = demo::read_summary(&workspace, &args.id).map_err(core_error)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "matchId": summary.match_id,
            "source": summary.source,
            "heatmap": summary.heatmap,
        }));
        return Ok(());
    }

    println!("{}", format_heatmap(&summary));

    Ok(())
}

/// `demo backends`: which extraction lanes exist on this build.
pub fn run_backends(args: &BackendsArgs) -> Result<()> {
    let status = demo::backend_status();

    if args.json {
        print_json(&json!({
            "ok": true,
            "backends": status,
        }));
        return Ok(());
    }

    println!("{}", demo::format_backend_status(&status));

    Ok(())
}

fn print_import(result: &ImportResult) {
    let summary = &result.summary;
    let provenance = &result.provenance;

    println!(
        "Imported {} into {}",
        result.match_id,
        lossy(&result.directory)
    );
    println!(
        "  Source:     {} ({}, crc32 {})",
        provenance.source.path,
        format_bytes(provenance.source.byte_len),
        provenance.crc32
    );
    println!(
        "  Extractor:  {} ({})",
        provenance.extractor,
        if provenance.source.kind == "fixture" {
            "fixture data, not a real capture"
        } else {
            "container only"
        }
    );
    println!("  {}", field_line("Map", &summary.match_info.map));
    println!(
        "  {}",
        duration_line("Duration", &summary.match_info.duration_seconds)
    );
    if let Some(ticks) = summary.match_info.duration_ticks {
        println!("  Playback ticks: {ticks}");
    }
    println!("  {}", field_line("Result", &summary.match_info.result));
    println!(
        "  Players: {}  Events: {}  Heatmap cells: {}",
        summary.players.len(),
        summary.timeline.len(),
        summary.heatmap.cells.len()
    );

    let (present, partial, absent) = coverage_counts(summary);
    println!("  Coverage: {present} present, {partial} partial, {absent} absent");
    let absent_fields = coverage_fields(summary, CoverageState::Absent);
    if !absent_fields.is_empty() {
        println!("  Absent:   {}", join_limited(&absent_fields, 8));
    }
    if !provenance.missing_fields.is_empty() {
        println!(
            "  Not extracted: {}",
            join_limited(&provenance.missing_fields, 10)
        );
    }
    if !summary.disagreements.is_empty() {
        println!("  Disagreements: {}", summary.disagreements.len());
    }
    if result.existed {
        println!(
            "  Re-import: revision {}; prior review preserved: {}",
            provenance.revision,
            if result.review_preserved { "yes" } else { "no" }
        );
    }

    if summary.players.is_empty() && summary.timeline.is_empty() {
        println!(
            "  No player, event or position rows were produced. The container extractor reads metadata only."
        );
    }
    if !summary.heatmap.available {
        println!(
            "  Heatmap: absent. A heatmap is only drawn from real position samples. Run `dah demo backends`."
        );
    }
    if !summary.disagreements.is_empty() {
        for disagreement in summary.disagreements.iter().take(5) {
            println!(
                "  Conflict {}: demo {} vs api {} (kept {})",
                disagreement.field,
                value_label(disagreement.demo.as_ref()),
                value_label(disagreement.api.as_ref()),
                disagreement.resolution
            );
        }
    }
    if !summary.notes.is_empty() {
        println!();
        for note in summary.notes.iter().take(6) {
            println!("  Note: {note}");
        }
    }
}

fn print_summary(summary: &MatchSummaryDoc, provenance: Option<&demo::Provenance>) {
    println!("Match {}  [{}]", summary.match_id, summary.source);

    if summary.source.ends_with("+api") {
        println!(
            "  Join: {} ({})",
            summary.join.key,
            if summary.join.inferred {
                "inferred"
            } else {
                "exact match id"
            }
        );
        if let Some(note) = &summary.join.note {
            println!("  Join note: {note}");
        }
    } else {
        println!("  Join: demo only (no cached api.json merged)");
    }

    println!("  {}", field_line("Map", &summary.match_info.map));
    println!(
        "  {}",
        duration_line("Duration", &summary.match_info.duration_seconds)
    );
    if let Some(ticks) = summary.match_info.duration_ticks {
        println!("  Playback ticks: {ticks}");
    }
    println!("  {}", field_line("Result", &summary.match_info.result));
    println!(
        "  {}",
        field_line("Start time", &summary.match_info.start_time_unix)
    );
    println!(
        "  {}",
        field_line("Game mode", &summary.match_info.game_mode)
    );
    println!("  {}", field_line("Build", &summary.match_info.build));
    println!("  {}", field_line("Rank", &summary.match_info.rank));
    println!("  {}", field_line("MMR", &summary.match_info.mmr));
    println!(
        "  {}",
        field_line(
            "Population percentile",
            &summary.match_info.population_percentile
        )
    );

    println!();
    println!("Players: {}", summary.players.len());
    for player in summary.players.iter().take(24) {
        print_player(player);
    }
    if summary.players.len() > 24 {
        println!("  ... {} more", summary.players.len() - 24);
    }

    println!();
    println!("Timeline: {} event(s)", summary.timeline.len());
    for event in summary.timeline.iter().take(12) {
        let time = event
            .time_seconds
            .map(|value| format!("{:>7.1}s", value))
            .unwrap_or_else(|| "     ?s".to_string());
        let actor = event.actor.as_deref().unwrap_or("-");
        let target = event.target.as_deref().unwrap_or("-");
        println!("  {time}  {:<12} {actor} -> {target}", event.kind);
    }
    if summary.timeline.len() > 12 {
        println!("  ... {} more", summary.timeline.len() - 12);
    }

    println!();
    println!("{}", format_heatmap(summary));

    let (present, partial, absent) = coverage_counts(summary);
    println!();
    println!("Coverage: {present} present, {partial} partial, {absent} absent");
    let absent_fields = coverage_fields(summary, CoverageState::Absent);
    if !absent_fields.is_empty() {
        println!("  Absent: {}", join_limited(&absent_fields, 12));
    }
    let partial_fields = coverage_fields(summary, CoverageState::Partial);
    if !partial_fields.is_empty() {
        println!("  Partial: {}", join_limited(&partial_fields, 8));
    }

    if !summary.disagreements.is_empty() {
        println!();
        println!("Disagreements: {}", summary.disagreements.len());
        for disagreement in summary.disagreements.iter().take(12) {
            println!(
                "  {}: demo {} vs api {} (kept {})",
                disagreement.field,
                value_label(disagreement.demo.as_ref()),
                value_label(disagreement.api.as_ref()),
                disagreement.resolution
            );
            if let Some(note) = &disagreement.note {
                println!("    {note}");
            }
        }
    }

    if let Some(provenance) = provenance {
        println!();
        println!(
            "Provenance: extractor {}, revision {}, crc32 {}",
            provenance.extractor, provenance.revision, provenance.crc32
        );
        if !provenance.unsupported.is_empty() {
            println!("  Not extracted:");
            for item in &provenance.unsupported {
                println!("    {item}");
            }
        }
    }

    if !summary.notes.is_empty() {
        println!();
        for note in summary.notes.iter().take(10) {
            println!("Note: {note}");
        }
    }
}

fn print_player(player: &demo::MergedPlayer) {
    let name = player
        .name
        .as_ref()
        .map(|value| value.value.clone())
        .unwrap_or_else(|| player.key.clone());
    let hero = player
        .hero
        .as_ref()
        .map(|value| value.value.clone())
        .or_else(|| {
            player
                .hero_id
                .as_ref()
                .map(|value| format!("hero {}", value.value))
        })
        .unwrap_or_else(|| "-".to_string());
    let team = player
        .team
        .as_ref()
        .map(|value| value.value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let record = format!(
        "{}/{}/{}",
        optional_number(&player.kills),
        optional_number(&player.deaths),
        optional_number(&player.assists)
    );
    let kda = player
        .kda
        .as_ref()
        .map(|value| format!("{:.2}", value.value))
        .unwrap_or_else(|| "-".to_string());
    let record_source = player
        .kills
        .as_ref()
        .map(|value| value.source.label())
        .unwrap_or("-");
    let kda_source = player
        .kda
        .as_ref()
        .map(|value| value.source.label())
        .unwrap_or("-");
    let net = player
        .net_worth
        .as_ref()
        .map(|value| thousands(value.value))
        .unwrap_or_else(|| "-".to_string());

    println!(
        "  {:<18} {:<14} team {:<2} {:<9} [{}]  KDA {:<6} [{}]  net {:<9} evidence {}e/{}p",
        clip(&name, 18),
        clip(&hero, 14),
        team,
        record,
        record_source,
        kda,
        kda_source,
        net,
        player.evidence.event_samples,
        player.evidence.position_samples
    );
}

fn format_heatmap(summary: &MatchSummaryDoc) -> String {
    let heatmap = &summary.heatmap;

    if !heatmap.available {
        let mut lines = vec![format!("Heatmap for {}: absent", summary.match_id)];
        if let Some(reason) = &heatmap.reason {
            lines.push(format!("  Reason: {reason}"));
        }
        let position_state = summary
            .coverage
            .fields
            .get("positions")
            .copied()
            .unwrap_or(CoverageState::Absent);
        let position_label = match position_state {
            CoverageState::Present => "present",
            CoverageState::Partial => "partial",
            CoverageState::Absent => "absent",
        };
        lines.push(format!("  Position coverage: {position_label}"));
        lines.push("  Run `dah demo backends` to see which extractor is available.".to_string());
        return lines.join("\n");
    }

    let mut lines = vec![format!(
        "Heatmap for {} ({})",
        summary.match_id, summary.source
    )];
    lines.push(format!(
        "  Basis: {}  Cell size: {} world units  Samples: {}  Cells: {}",
        heatmap.basis.as_deref().unwrap_or("positions"),
        heatmap.cell_size as u64,
        heatmap.samples,
        heatmap.cells.len()
    ));

    for cell in heatmap.cells.iter().take(20) {
        lines.push(format!(
            "  ({:>6}, {:>6})  {} sample(s)",
            cell.x, cell.y, cell.samples
        ));
    }
    if heatmap.cells.len() > 20 {
        lines.push(format!("  ... {} more cell(s)", heatmap.cells.len() - 20));
    }

    lines.join("\n")
}

fn field_line<T: Display>(label: &str, value: &Option<FieldValue<T>>) -> String {
    match value {
        Some(field) => format!(
            "{label}: {} ({}, {})",
            field.value,
            field.source.label(),
            confidence_label(field.confidence)
        ),
        None => format!("{label}: absent"),
    }
}

fn duration_line(label: &str, value: &Option<FieldValue<u64>>) -> String {
    match value {
        Some(field) => format!(
            "{label}: {} ({} s, {}, {})",
            format_duration(field.value),
            field.value,
            field.source.label(),
            confidence_label(field.confidence)
        ),
        None => format!("{label}: absent"),
    }
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn coverage_counts(summary: &MatchSummaryDoc) -> (usize, usize, usize) {
    let mut present = 0;
    let mut partial = 0;
    let mut absent = 0;

    for state in summary.coverage.fields.values() {
        match state {
            CoverageState::Present => present += 1,
            CoverageState::Partial => partial += 1,
            CoverageState::Absent => absent += 1,
        }
    }

    (present, partial, absent)
}

fn coverage_fields(summary: &MatchSummaryDoc, state: CoverageState) -> Vec<String> {
    summary
        .coverage
        .fields
        .iter()
        .filter(|(_, value)| **value == state)
        .map(|(field, _)| field.clone())
        .collect()
}

fn join_limited(values: &[String], limit: usize) -> String {
    if values.len() <= limit {
        return values.join(", ");
    }
    format!(
        "{}, +{} more",
        values
            .iter()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        values.len() - limit
    )
}

fn value_label(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(value) => {
            let text = value.to_string();
            clip(&text, 40)
        }
        None => "absent".to_string(),
    }
}

fn optional_number<T: Display>(value: &Option<FieldValue<T>>) -> String {
    value
        .as_ref()
        .map(|field| field.value.to_string())
        .unwrap_or_else(|| "-".to_string())
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
    value.chars().take(16).collect::<String>().replace('T', " ")
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

fn clip(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut text: String = value.chars().take(limit.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

fn core_error(error: deadlock_core::Error) -> CliError {
    CliError::new("demo", error.to_string())
}

pub struct ReviewArgs {
    /// Match id in the local store.
    pub id: String,
    /// OpenAI-compatible model name. Needs `base_url`.
    pub model: Option<String>,
    /// Base URL, for example http://127.0.0.1:8080/v1. Needs `model`.
    pub base_url: Option<String>,
    /// A review text to lint. Without it the prompt and fact set are printed.
    pub file: Option<PathBuf>,
    /// Write review.md after the linter accepts every number.
    pub write: bool,
    /// Workspace directory. Defaults to `DAH_WORKSPACE_DIR` or `<data>/workspace`.
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

/// `dah demo review <id>`: build the verified prompt, or lint a review and
/// store it only when every number is supported by the summary.
pub fn run_review(args: &ReviewArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let summary = demo::read_summary(&workspace, &args.id).map_err(core_error)?;
    let facts = review::facts_for(&summary);

    let model = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let base_url = args
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (model, base_url) {
        (Some(model), Some(base_url)) => {
            if args.file.is_some() {
                return Err(CliError::new(
                    "review_args",
                    "Use --file to lint an existing review, or --model with --base-url to generate one, not both.".to_string(),
                ));
            }
            let config = review::ModelConfig {
                base_url: base_url.to_string(),
                model: model.to_string(),
                api_key: review::key_from_environment(),
                timeout_seconds: 120,
            };
            let body = review::generate_review(&facts, &config)
                .map_err(|error| CliError::new("review_model", error.to_string()))?;
            let report = review::lint_review(&body, &facts);
            if !report.ok() {
                if args.json {
                    print_json(&json!({
                        "ok": false,
                        "matchId": facts.match_id,
                        "model": model,
                        "checkedNumbers": report.checked_numbers,
                        "violations": report.violations,
                        "review": body,
                    }));
                } else {
                    println!(
                        "Model review refused: {} unsupported number(s) out of {} checked.",
                        report.violations.len(),
                        report.checked_numbers
                    );
                    for violation in &report.violations {
                        println!("  line {}: {} is not in the fact set", violation.line, violation.number);
                    }
                    println!("Nothing was written. A review may not invent a statistic.");
                }
                return Err(CliError::new(
                    "review_lint",
                    "The model invented a number the verified summary does not support.".to_string(),
                ));
            }
            if args.write {
                let stored = review::write_review(&workspace, &facts, &body)
                    .map_err(core_error)?;
                if args.json {
                    print_json(&json!({
                        "ok": true,
                        "matchId": stored.match_id,
                        "model": model,
                        "checkedNumbers": stored.checked_numbers,
                        "written": true,
                        "path": lossy(&review::review_path(&workspace, &facts.match_id).map_err(core_error)?),
                    }));
                } else {
                    println!(
                        "Wrote review.md for match {} from {} after checking {} number(s).",
                        stored.match_id, model, stored.checked_numbers
                    );
                }
            } else {
                if args.json {
                    print_json(&json!({
                        "ok": true,
                        "matchId": facts.match_id,
                        "model": model,
                        "checkedNumbers": report.checked_numbers,
                        "written": false,
                        "review": body,
                    }));
                } else {
                    println!("Generated with {model}; {} number(s) checked.", report.checked_numbers);
                    println!();
                    print!("{body}");
                    println!();
                    println!("Nothing was written. Re-run with --write to store review.md.");
                }
            }
            return Ok(());
        }
        (None, None) => {}
        _ => {
            return Err(CliError::new(
                "review_args",
                "--model and --base-url must be set together.".to_string(),
            ));
        }
    }

    let Some(file) = &args.file else {
        if args.json {
            print_json(&json!({
                "ok": true,
                "matchId": facts.match_id,
                "prompt": facts.prompt,
                "facts": facts.facts,
                "allowedNumbers": facts.allowed_numbers,
                "absent": facts.absent,
                "note": "No provider is called. Send the prompt to the model you configured, then lint the reply with --file.",
            }));
            return Ok(());
        }
        println!("Review prompt for match {}", facts.match_id);
        println!(
            "  {} verified fact(s), {} absent field(s)",
            facts.facts.len(),
            facts.absent.len()
        );
        println!();
        print!("{}", facts.prompt);
        println!();
        println!(
            "No provider is called. Send this prompt to the model you configured, save the reply, then run:"
        );
        println!(
            "  dah demo review {} --file reply.md --write",
            facts.match_id
        );
        return Ok(());
    };

    let body = std::fs::read_to_string(file).map_err(|error| {
        CliError::new(
            "review_file",
            format!("Could not read {}: {error}", file.display()),
        )
    })?;
    let report = review::lint_review(&body, &facts);

    if !report.ok() {
        if args.json {
            print_json(&json!({
                "ok": false,
                "matchId": facts.match_id,
                "checkedNumbers": report.checked_numbers,
                "violations": report.violations,
            }));
        } else {
            println!(
                "Review refused: {} unsupported number(s) out of {} checked.",
                report.violations.len(),
                report.checked_numbers
            );
            for violation in &report.violations {
                println!(
                    "  line {}: {} is not in the fact set",
                    violation.line, violation.number
                );
                println!("    {}", violation.excerpt);
            }
            println!("Nothing was written. A review may not invent a statistic.");
        }
        return Err(CliError::new(
            "review_lint",
            "The review contains numbers the verified summary does not support.".to_string(),
        ));
    }

    if !args.write {
        if args.json {
            print_json(&json!({
                "ok": true,
                "matchId": facts.match_id,
                "checkedNumbers": report.checked_numbers,
                "written": false,
            }));
        } else {
            println!(
                "Review accepted: {} number(s) checked against the summary. Nothing was written.",
                report.checked_numbers
            );
            println!("Re-run with --write to store review.md.");
        }
        return Ok(());
    }

    let stored = review::write_review(&workspace, &facts, &body).map_err(core_error)?;
    if args.json {
        print_json(&json!({
            "ok": true,
            "matchId": stored.match_id,
            "checkedNumbers": stored.checked_numbers,
            "generatedAt": stored.generated_at,
            "written": true,
            "path": lossy(&review::review_path(&workspace, &facts.match_id).map_err(core_error)?),
        }));
    } else {
        println!(
            "Wrote review.md for match {} after checking {} number(s).",
            stored.match_id, stored.checked_numbers
        );
        println!(
            "  {}",
            review::review_path(&workspace, &facts.match_id)
                .map_err(core_error)?
                .display()
        );
    }
    Ok(())
}
