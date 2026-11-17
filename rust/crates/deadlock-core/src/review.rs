//! The review contract: verified facts in, coaching out, numbers checked.
//!
//! A model may explain, prioritise and suggest. It may not invent a number. This
//! module builds the fact set and the prompt from a merged match summary, and
//! lints any generated review against the same facts before it is written. The
//! linter is deterministic and has no provider, so it runs in tests and in CI.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::demo::{DemoEvent, MatchSummaryDoc, MergedPlayer};
use crate::error::{Error, Result};
use crate::util::now_iso;

pub const REVIEW_FILE_NAME: &str = "review.md";
pub const REVIEW_SCHEMA: u32 = 1;
/// Environment variables read for a review model key, in order. The key is
/// never logged, stored in a review or written to the workspace.
pub const REVIEW_KEY_ENVS: [&str; 2] = ["DAH_REVIEW_API_KEY", "OPENAI_API_KEY"];

const REVIEW_SYSTEM: &str = "You are a Deadlock coach. Use only the verified facts in the user message. Never invent a statistic, a timestamp or a movement pattern. If a field is absent, say it is absent. Return Markdown with the requested sections.";

/// An OpenAI-compatible chat endpoint. `api_key` is optional for a local server.
#[derive(Clone, PartialEq, Eq)]
pub struct ModelConfig {
    /// Base URL, for example `http://127.0.0.1:8080/v1`.
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout_seconds: u64,
}

impl fmt::Debug for ModelConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

/// One factual line the model may cite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    /// Stable key, for example `player.kills` or `event.3.time`.
    pub key: String,
    /// The line exactly as the prompt shows it.
    pub text: String,
}

/// Everything a review may say, derived from the summary only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFacts {
    pub schema: u32,
    pub match_id: String,
    pub facts: Vec<Fact>,
    /// Canonical number tokens that may appear in a review.
    pub allowed_numbers: BTreeSet<String>,
    /// Fields the extractor could not produce. The model must not describe them.
    pub absent: Vec<String>,
    pub prompt: String,
}

/// One number in the review that the facts do not support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LintViolation {
    /// 1-based line in the review.
    pub line: usize,
    pub number: String,
    pub excerpt: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LintReport {
    pub checked_numbers: usize,
    pub violations: Vec<LintViolation>,
}

impl LintReport {
    pub fn ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// A stored review, with the lint result that allowed it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredReview {
    pub schema: u32,
    pub match_id: String,
    pub generated_at: String,
    pub checked_numbers: usize,
    pub body: String,
}

/// Build the fact set and the prompt for one summary.
pub fn facts_for(summary: &MatchSummaryDoc) -> ReviewFacts {
    let mut facts = Vec::new();
    let mut absent = Vec::new();

    if let Some(map) = &summary.match_info.map {
        facts.push(fact("match.map", format!("Map: {}", map.value)));
    } else {
        absent.push("match.map".to_string());
    }
    if let Some(duration) = &summary.match_info.duration_seconds {
        let minutes = duration.value / 60;
        let seconds = duration.value % 60;
        facts.push(fact(
            "match.duration",
            format!(
                "Duration: {}m {}s ({} seconds)",
                minutes, seconds, duration.value
            ),
        ));
    } else {
        absent.push("match.duration".to_string());
    }
    if let Some(result) = &summary.match_info.result {
        facts.push(fact("match.result", format!("Result: {}", result.value)));
    } else {
        absent.push("match.result".to_string());
    }
    if let Some(rank) = &summary.match_info.rank {
        facts.push(fact("match.rank", format!("Rank: {}", rank.value)));
    }
    if let Some(mmr) = &summary.match_info.mmr {
        facts.push(fact("match.mmr", format!("MMR: {}", fmt(mmr.value))));
    }
    if let Some(percentile) = &summary.match_info.population_percentile {
        facts.push(fact(
            "match.percentile",
            format!("Population percentile: {}", fmt(percentile.value)),
        ));
    }

    for player in &summary.players {
        player_facts(player, &mut facts, &mut absent);
    }

    for event in &summary.timeline {
        event_fact(event, &mut facts);
        if let Some(time) = event.time_seconds {
            facts.push(fact(
                &format!("event.{}.time", event.sequence),
                format!(
                    "Event {} at {}s: {}",
                    event.sequence,
                    fmt(time),
                    event_summary(event)
                ),
            ));
        }
    }

    if summary.heatmap.available {
        facts.push(fact(
            "heatmap",
            format!(
                "Heatmap: {} cell(s), {} sample(s), basis {}",
                summary.heatmap.cells.len(),
                summary.heatmap.samples,
                summary.heatmap.basis.clone().unwrap_or_else(|| "unknown".to_string())
            ),
        ));
    } else {
        absent.push("heatmap".to_string());
    }

    for disagreement in &summary.disagreements {
        let demo = disagreement
            .demo
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "absent".to_string());
        let api = disagreement
            .api
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "absent".to_string());
        facts.push(fact(
            &format!("disagreement.{}", disagreement.field),
            format!(
                "Disagreement on {}: demo {} vs api {}, kept {}",
                disagreement.field, demo, api, disagreement.resolution
            ),
        ));
    }

    let mut allowed = BTreeSet::new();
    for fact in &facts {
        for token in numbers_in(&fact.text) {
            allowed.insert(token.canonical);
        }
    }

    let prompt = build_prompt(summary, &facts, &absent);

    ReviewFacts {
        schema: REVIEW_SCHEMA,
        match_id: summary.match_id.clone(),
        facts,
        allowed_numbers: allowed,
        absent,
        prompt,
    }
}

fn player_facts(player: &MergedPlayer, facts: &mut Vec<Fact>, absent: &mut Vec<String>) {
    let name = player
        .name
        .as_ref()
        .map(|value| value.value.clone())
        .or_else(|| player.hero.as_ref().map(|value| value.value.clone()))
        .unwrap_or_else(|| player.key.clone());
    let hero = player
        .hero
        .as_ref()
        .map(|value| value.value.clone())
        .unwrap_or_else(|| "unknown hero".to_string());
    let team = player
        .team
        .as_ref()
        .map(|value| value.value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    facts.push(fact(
        &format!("player.{}.identity", player.key),
        format!("Player {name}: {hero}, team {team}"),
    ));

    let mut stats = Vec::new();
    if let Some(kills) = &player.kills {
        stats.push(format!("{} kills", kills.value));
    }
    if let Some(deaths) = &player.deaths {
        stats.push(format!("{} deaths", deaths.value));
    }
    if let Some(assists) = &player.assists {
        stats.push(format!("{} assists", assists.value));
    }
    if let Some(kda) = &player.kda {
        stats.push(format!("{} KDA", fmt(kda.value)));
    }
    if let Some(net) = &player.net_worth {
        stats.push(format!("{} net worth", net.value));
    }
    if stats.is_empty() {
        absent.push(format!("player.{}.stats", player.key));
    } else {
        facts.push(fact(
            &format!("player.{}.stats", player.key),
            format!("Player {name} stats: {}", stats.join(", ")),
        ));
    }
}

fn event_fact(event: &DemoEvent, facts: &mut Vec<Fact>) {
    if let Some(time) = event.time_seconds {
        facts.push(fact(
            &format!("event.{}.time", event.sequence),
            format!(
                "Event {} at {}s: {}",
                event.sequence,
                fmt(time),
                event_summary(event)
            ),
        ));
    }
}

fn event_summary(event: &DemoEvent) -> String {
    let actor = event.actor.clone().unwrap_or_else(|| "unknown".to_string());
    let target = event.target.clone().unwrap_or_else(|| "unknown".to_string());
    match event.kind.as_str() {
        "kill" => format!("{actor} killed {target}"),
        "death" => format!("{actor} died to {target}"),
        other => format!("{other} {actor} -> {target}"),
    }
}

fn fact(key: &str, text: String) -> Fact {
    Fact {
        key: key.to_string(),
        text,
    }
}

fn build_prompt(summary: &MatchSummaryDoc, facts: &[Fact], absent: &[String]) -> String {
    let mut text = String::new();
    text.push_str("You are reviewing one Deadlock match. Use only the verified facts below.\n\n");
    text.push_str("Rules:\n");
    text.push_str("1. Every number you write must appear in the facts exactly, or be a simple sum of facts you name.\n");
    text.push_str("2. Cite the timestamp or the metric key that supports each recommendation.\n");
    text.push_str("3. If a field is absent, say it is absent. Do not describe movement, aim or positioning when no positions were produced.\n");
    text.push_str("4. Do not judge the player's character or intent from a single match.\n");
    text.push_str("5. If the data cannot support a claim, omit the claim.\n\n");
    text.push_str("Verified facts:\n");
    for fact in facts {
        text.push_str("- ");
        text.push_str(&fact.text);
        text.push('\n');
    }
    if !absent.is_empty() {
        text.push_str("\nAbsent fields, do not describe them:\n");
        for field in absent {
            text.push_str("- ");
            text.push_str(field);
            text.push('\n');
        }
    }
    text.push_str("\nCoverage notes:\n");
    for note in &summary.coverage.notes {
        text.push_str("- ");
        text.push_str(note);
        text.push('\n');
    }
    text.push_str("\nWrite the review as Markdown with these sections: Summary, What went well, What cost the match, Three actions, Data gaps.\n");
    text
}

/// Check every number in a review against the facts.
///
/// Numbers inside words (`dl_midtown`, `v2`) are ignored. A token with `k` or
/// `m` is normalised, so `30k` matches `30000`. Percent signs and thousands
/// separators are ignored.
pub fn lint_review(text: &str, facts: &ReviewFacts) -> LintReport {
    let mut violations = Vec::new();
    let mut checked = 0;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let mut list_prefix = 0;
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            list_prefix = line.len() - rest.len();
        } else {
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            let rest = &trimmed[digits..];
            if digits > 0 && digits <= 3 && (rest.starts_with(". ") || rest.starts_with(") ")) {
                list_prefix = indent + digits;
            }
        }
        for token in numbers_in(line) {
            if token.start < list_prefix {
                continue;
            }
            checked += 1;
            if !facts.allowed_numbers.contains(&token.canonical) {
                violations.push(LintViolation {
                    line: index + 1,
                    number: token.raw.clone(),
                    excerpt: line.trim().chars().take(120).collect(),
                    reason: format!(
                        "{} is not in the verified fact set for match {}",
                        token.raw, facts.match_id
                    ),
                });
            }
        }
    }

    LintReport {
        checked_numbers: checked,
        violations,
    }
}

struct NumberToken {
    raw: String,
    canonical: String,
    start: usize,
}

fn numbers_in(text: &str) -> Vec<NumberToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if !byte.is_ascii_digit() {
            index += 1;
            continue;
        }
        let previous = if index == 0 { b' ' } else { bytes[index - 1] };
        if previous.is_ascii_alphanumeric() || previous == b'_' {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        let mut seen_dot = false;
        while end < bytes.len() {
            let current = bytes[end];
            if current.is_ascii_digit() || (current == b',' && end + 1 < bytes.len()) {
                end += 1;
            } else if current == b'.' && !seen_dot && end + 1 < bytes.len() {
                seen_dot = true;
                end += 1;
            } else {
                break;
            }
        }
        let raw = text[start..end].to_string();
        let mut multiplier = 1.0_f64;
        if end < bytes.len() {
            match bytes[end] {
                b'k' | b'K' => multiplier = 1_000.0,
                b'm' | b'M' => multiplier = 1_000_000.0,
                _ => {}
            }
        }
        let cleaned = raw.trim_end_matches('.').replace(',', "");
        if cleaned.is_empty() {
            index = end + 1;
            continue;
        }
        if let Ok(value) = cleaned.parse::<f64>() {
            tokens.push(NumberToken {
                raw,
                canonical: fmt(value * multiplier),
                start,
            });
        }
        index = end + 1;
    }

    tokens
}

/// `6.0` becomes `6`, `58.90` becomes `58.9`, `30000` stays `30000`.
fn fmt(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}", rounded as i64)
    } else {
        let text = format!("{rounded}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Ask an OpenAI-compatible chat model for a review from the verified facts.
///
/// The prompt is the same one printed by `dah demo review`; the reply is not
/// linted here. Call [`review_and_write`] to lint and store in one step.
pub fn generate_review(facts: &ReviewFacts, config: &ModelConfig) -> Result<String> {
    if config.base_url.trim().is_empty() {
        return Err(Error::msg(
            "A review needs a base URL, for example http://127.0.0.1:8080/v1.",
        ));
    }
    if config.model.trim().is_empty() {
        return Err(Error::msg("A review needs a model name."));
    }
    let url = format!(
        "{}/chat/completions",
        config.base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "model": config.model,
        "temperature": 0.2,
        "max_tokens": 1200,
        "messages": [
            {"role": "system", "content": REVIEW_SYSTEM},
            {"role": "user", "content": facts.prompt},
        ],
    });

    let agent = ureq::Agent::config_builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(config.timeout_seconds.max(5))))
        .build()
        .new_agent();
    let mut request = agent
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");
    if let Some(key) = &config.api_key
        && !key.is_empty()
    {
        request = request.header("Authorization", format!("Bearer {key}"));
    }

    let body = serde_json::to_string(&body)
        .map_err(|error| Error::msg(format!("Could not encode the review request: {error}")))?;
    let mut response = request
        .header("Content-Length", body.len().to_string())
        .send(body.as_bytes())
        .map_err(|error| Error::msg(format!("The review request failed: {error}")))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::msg(format!("The review response was unreadable: {error}")))?;

    if !(200..300).contains(&status) {
        return Err(Error::msg(format!(
            "The review endpoint returned HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    if text.len() > 64 * 1024 {
        return Err(Error::msg(
            "The review endpoint returned more than 64 KiB. Refusing to parse it.",
        ));
    }

    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        Error::msg(format!(
            "The review endpoint returned invalid JSON: {error}. Body: {}",
            text.chars().take(300).collect::<String>()
        ))
    })?;
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| {
            Error::msg(
                "The review endpoint returned no choices[0].message.content. Check that the model is a chat model.",
            )
        })?;

    let content = content.trim();
    if content.is_empty() {
        return Err(Error::msg("The review endpoint returned an empty reply."));
    }
    Ok(content.to_string())
}

/// Generate, lint and store a review. A number the facts do not support aborts
/// the write, exactly like a hand-written review.
pub fn review_and_write(
    workspace: &Path,
    facts: &ReviewFacts,
    config: &ModelConfig,
) -> Result<StoredReview> {
    let body = generate_review(facts, config)?;
    write_review(workspace, facts, &body)
}

/// Read the first non-empty key from [`REVIEW_KEY_ENVS`].
pub fn key_from_environment() -> Option<String> {
    for name in REVIEW_KEY_ENVS {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub fn review_path(workspace: &Path, match_id: &str) -> Result<PathBuf> {
    Ok(crate::demo::match_dir(workspace, match_id)?.join(REVIEW_FILE_NAME))
}

/// Write a review only when the linter accepts every number.
pub fn write_review(
    workspace: &Path,
    facts: &ReviewFacts,
    body: &str,
) -> Result<StoredReview> {
    let report = lint_review(body, facts);
    if !report.ok() {
        let first = &report.violations[0];
        return Err(Error::msg(format!(
            "Review refused: {} unsupported number(s). First is {} on line {}: {}. A review may not invent a statistic.",
            report.violations.len(),
            first.number,
            first.line,
            first.reason
        )));
    }
    let stored = StoredReview {
        schema: REVIEW_SCHEMA,
        match_id: facts.match_id.clone(),
        generated_at: now_iso(),
        checked_numbers: report.checked_numbers,
        body: body.to_string(),
    };
    let path = review_path(workspace, &facts.match_id)?;
    fs::write(&path, body)?;
    Ok(stored)
}

pub fn read_review(workspace: &Path, match_id: &str) -> Result<Option<String>> {
    let path = review_path(workspace, match_id)?;
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> ReviewFacts {
        let mut allowed = BTreeSet::new();
        for value in ["1234", "30000", "58.9", "7", "4", "9", "2.4", "842"] {
            allowed.insert(value.to_string());
        }
        ReviewFacts {
            schema: REVIEW_SCHEMA,
            match_id: "42".to_string(),
            facts: Vec::new(),
            allowed_numbers: allowed,
            absent: vec!["heatmap".to_string()],
            prompt: String::new(),
        }
    }

    #[test]
    fn accepts_numbers_from_the_facts() {
        let report = lint_review(
            "You had 7 kills and 4 deaths at 842s. Net worth 30,000 and 58.9% win rate.",
            &facts(),
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(report.checked_numbers, 5);
    }

    #[test]
    fn rejects_an_invented_metric() {
        let report = lint_review("Your aim accuracy was 87% and impact 92.", &facts());
        assert!(!report.ok());
        assert_eq!(report.violations.len(), 2);
        assert_eq!(report.violations[0].number, "87");
    }

    #[test]
    fn normalises_k_and_m_suffixes() {
        let report = lint_review("Net worth 30k, damage 2.4m.", &facts());
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].number, "2.4");
    }

    #[test]
    fn ignores_numbers_inside_words() {
        let report = lint_review("Map dl_midtown and build v2 are names.", &facts());
        assert!(report.ok(), "{report:?}");
    }

    #[test]
    fn keeps_list_markers_out_of_the_count() {
        let report = lint_review("1. Buy boots\n2. Ward mid", &facts());
        assert_eq!(report.checked_numbers, 0);
    }

    fn serve_once(content: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let payload = format!(
            "{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":{}}}}}]}}",
            serde_json::to_string(content).expect("content json")
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header_end = buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .expect("headers")
                + 4;
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buffer.len() < header_end + length {
                let read = stream.read(&mut chunk).expect("body read");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&buffer).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            stream.write_all(response.as_bytes()).expect("write");
            request
        });
        (format!("http://{address}/v1"), handle)
    }

    fn model_facts() -> ReviewFacts {
        let mut facts = facts();
        facts.prompt = "Verified facts for match 42".to_string();
        facts
    }

    #[test]
    fn a_model_reply_is_linted_before_it_is_stored() {
        let (base_url, server) = serve_once("Your aim accuracy was 87%.");
        let facts = model_facts();
        let config = ModelConfig {
            base_url,
            model: "test-model".to_string(),
            api_key: Some("secret".to_string()),
            timeout_seconds: 5,
        };
        let text = generate_review(&facts, &config).expect("reply");
        assert!(text.contains("87%"));
        let report = lint_review(&text, &facts);
        assert!(!report.ok(), "an invented number must be refused");

        let request = server.join().expect("server");
        assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
        assert!(request.contains("Bearer secret"));
        assert!(request.contains("\"model\":\"test-model\""));
        assert!(request.contains("Verified facts for match 42"));
    }

    #[test]
    fn a_grounded_model_reply_is_written() {
        let (base_url, server) = serve_once("You had 7 kills and 4 deaths.");
        let facts = model_facts();
        let config = ModelConfig {
            base_url,
            model: "test-model".to_string(),
            api_key: None,
            timeout_seconds: 5,
        };
        let workspace =
            std::env::temp_dir().join(format!("dah-review-model-{}", std::process::id()));
        std::fs::create_dir_all(workspace.join("matches").join("42")).expect("match dir");
        let stored = review_and_write(&workspace, &facts, &config).expect("review");
        assert_eq!(stored.checked_numbers, 2);
        let path = review_path(&workspace, "42").expect("path");
        assert!(path.is_file());
        let request = server.join().expect("server");
        assert!(!request.contains("Authorization"), "no key means no header");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn a_model_call_needs_an_endpoint_and_model() {
        let facts = model_facts();
        let missing = ModelConfig {
            base_url: String::new(),
            model: "x".to_string(),
            api_key: None,
            timeout_seconds: 5,
        };
        assert!(generate_review(&facts, &missing).is_err());
        let no_model = ModelConfig {
            base_url: "http://127.0.0.1:9/v1".to_string(),
            model: String::new(),
            api_key: None,
            timeout_seconds: 5,
        };
        assert!(generate_review(&facts, &no_model).is_err());
    }

    #[test]
    fn formatting_rounds_to_two_places() {
        assert_eq!(fmt(6.0), "6");
        assert_eq!(fmt(58.90), "58.9");
        assert_eq!(fmt(30000.0), "30000");
        assert_eq!(fmt(2.444), "2.44");
    }
}
