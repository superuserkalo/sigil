//! GameBanana Remote Install: queue a mod on GameBanana from a phone, pick it up here.
//!
//! GameBanana keeps a queue per member. A player pops a mod into it from any device, from the
//! Remote Install entry under a mod's file list. Sigil reads that queue on a timer and hands
//! each entry to the ordinary install pipeline. Nothing here downloads, installs or stores
//! anything: the module parses a pair link, reads the queue and returns plain data.
//!
//! The protocol was recorded from Deadlock Mod Manager 2.2, the manager GameBanana has enrolled
//! (behaviour only, no code taken):
//!
//! 1. Pair. GameBanana's Pair button opens `deadlock-mod-manager:registerKey,<member id>,<token>`
//!    (also with a trailing slash after the token, and with the double slash a scheme usually
//!    carries). [`pairing_start`] reads it and holds the token; [`pairing_finish`] reads the
//!    member's public profile so the player can check the name before linking.
//! 2. Read the queue. `GET apiv11/RemoteInstall/<member id>/<token>/<manager alias>` answers a
//!    JSON array of `"<download url>,<Mod|Sound>,<mod id>"` strings, or `null` when nothing is
//!    pending. A refusal (a manager that is not enrolled, a token from an Enroll that was since
//!    replaced) arrives with HTTP 200 as `{"_sErrorCode": "..."}`, so the code is read before
//!    the list is.
//! 3. Acknowledge. There is no acknowledgement call. A queue read marks every entry it returns
//!    as retrieved on GameBanana, and the same entry never comes back. [`acknowledge`] records
//!    the installer's side and checks the id; it sends nothing.
//!
//! Deadlock Mod Manager checks the queue every five minutes while it runs, in the background,
//! and offers a Check now button. Sigil uses [`POLL_INTERVAL_SECONDS`] for the timer. Because
//! the read is the acknowledgement, a failed poll or a closed app only delays an entry: it
//! stays pending on GameBanana until a poll succeeds.
//!
//! The token is an account credential and a path segment of the queue URL. It never enters a
//! log, an error message, the transcript or a `Debug` line ([`Pairing`] prints `<redacted>`).
//! Storing it is the app's job, in the OS keychain beside the provider keys. This module only
//! receives a [`Pairing`] and returns one.
//!
//! Network: `gamebanana.com` only, HTTPS, through [`crate::net`] so offline mode and the proxy
//! setting apply. No other host is contacted.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::install::WebUrl;

/// Schemes a pair link arrives under. `sigil:` is Sigil's own; the other three are Deadlock Mod
/// Manager's, accepted because GameBanana's Pair button fires the enrolled manager's scheme and
/// a player may paste that link instead of opening it.
const PAIR_SCHEMES: &[&str] = &[
    "sigil:",
    "deadlock-mod-manager:",
    "deadlock-modmanager:",
    "dlmm:",
];

/// The payload GameBanana sends after the scheme.
pub const PAIR_PAYLOAD_PREFIX: &str = "registerKey,";

/// The manager alias GameBanana assigned as the last segment of the queue URL.
///
/// GameBanana issues one alias per enrolled manager (Deadlock Mod Manager's is
/// `DeadlockModManager`). `Sigil` is a placeholder until the owner has Sigil enrolled; until
/// then a queue read answers an `_sErrorCode`, which [`fetch_queue`] reports and the UI shows.
pub const MANAGER_ALIAS: &str = "Sigil";

/// How often a paired app reads the queue: five minutes, as Deadlock Mod Manager does, while the
/// app is open and in the background. A manual Check now may read it at any time.
pub const POLL_INTERVAL_SECONDS: u64 = 300;

/// Longest token accepted. A token becomes a URL path segment, so its shape is checked before
/// use rather than escaped.
const MAX_TOKEN_LEN: usize = 128;

/// A queue is a handful of short strings; anything near this is not one.
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;

const REQUEST_TIMEOUT_SECONDS: u64 = 30;
const RETRY_DELAY_MILLIS: u64 = 700;
/// What a redacted token prints as.
const REDACTED: &str = "<redacted>";

/// A pair link that has been read but not yet confirmed by the player.
///
/// Kept only long enough to look the member up and ask. Nothing is saved until the player
/// agrees, because anyone who owns the linked account can queue mods that install on this
/// machine.
pub struct PairingRequest {
    pub member_id: u64,
    token: String,
}

impl PairingRequest {
    /// The token the queue URL carries. Never log it, format it or send it anywhere else.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for PairingRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingRequest")
            .field("member_id", &self.member_id)
            .field("token", &REDACTED)
            .finish()
    }
}

/// A confirmed pairing: the member, the name the player checked, and the token.
///
/// It deliberately does not implement `Serialize`. The app persists the member id, the name and
/// [`Pairing::token`] in its own shapes (the token in the OS keychain), so a `Pairing` cannot
/// reach a settings file or a log by accident.
pub struct Pairing {
    /// The GameBanana member id the token belongs to.
    pub member_id: u64,
    /// The public member name, as the profile read it.
    pub name: String,
    /// The member's avatar, when the profile carries one.
    pub avatar_url: Option<String>,
    /// The public profile page. Empty for a pairing restored from the keychain, which does not
    /// read the profile again.
    pub profile_url: String,
    token: String,
}

impl Pairing {
    /// Build a pairing from stored values, as the app does at startup after reading the token
    /// from the keychain. The token is checked here, so a damaged keychain entry is reported at
    /// the point it is loaded instead of as a confusing queue error later.
    pub fn new(member_id: u64, name: String, token: &str) -> Result<Pairing> {
        if member_id == 0 {
            return Err(Error::msg(
                "A Remote Install pairing needs a GameBanana member id.",
            ));
        }
        validate_token(token)?;
        Ok(Pairing {
            member_id,
            name,
            avatar_url: None,
            profile_url: String::new(),
            token: token.to_string(),
        })
    }

    /// The token the queue URL carries. Never log it, format it or send it anywhere else.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for Pairing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pairing")
            .field("member_id", &self.member_id)
            .field("name", &self.name)
            .field("avatar_url", &self.avatar_url)
            .field("profile_url", &self.profile_url)
            .field("token", &REDACTED)
            .finish()
    }
}

/// One pending item from the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMod {
    /// The GameBanana submission id, the number in its mod page URL.
    pub mod_id: u64,
    /// The file the one-click link points at (`/mmdl/<id>`), when the link names one. The
    /// install pipeline follows the link itself; this is for display and bookkeeping.
    pub file_id: Option<u64>,
    /// The wire entry carries no timestamp, so this is always `None` today. A caller that wants
    /// an age can record its own poll time beside the item.
    pub queued_at: Option<u64>,
    /// A stable local id. The entry has none, so it is derived from model, mod id and file id;
    /// that is stable across polls and collapses an entry queued twice.
    pub queue_id: String,
    /// `Mod` or `Sound`, the submission model the entry names.
    pub model: String,
    /// The one-click download link: https, on `gamebanana.com` or one of its file hosts.
    pub download_url: String,
}

/// Read a pair link: `sigil:registerKey,<member id>,<token>`,
/// `deadlock-mod-manager://registerKey,<member id>,<token>/`, or the bare payload.
///
/// GameBanana issues the token when the player presses Enroll, and a later Enroll replaces it,
/// so nothing is compared with an older value here.
pub fn pairing_start(link: &str) -> Result<PairingRequest> {
    let payload = strip_scheme(link).trim_start_matches('/');
    let payload = payload.strip_prefix(PAIR_PAYLOAD_PREFIX).ok_or_else(|| {
        Error::msg(
            "That link is not a GameBanana Remote Install pair link. Press Enroll, then Pair, on GameBanana and open the link it sends.",
        )
    })?;
    let payload = payload.trim_end_matches('/');
    let (member_id, token) = payload.split_once(',').ok_or_else(|| {
        Error::msg(
            "That pair link has no key. Press Enroll again on GameBanana and open the new link.",
        )
    })?;

    let member_id = parse_member_id(member_id)?;
    validate_token(token)?;

    Ok(PairingRequest {
        member_id,
        token: token.to_string(),
    })
}

/// Read the member's public profile and return the pairing the app offers to link.
///
/// This is the only request in the pairing flow: `apiv11/Member/<member id>/ProfilePage`. The
/// token is not sent here; it travels only in a later queue URL. Nothing is stored, so the app
/// can show the name and keep the token in the keychain only after the player confirms it.
pub fn pairing_finish(request: &PairingRequest) -> Result<Pairing> {
    validate_token(&request.token)?;
    let url = format!(
        "{}/Member/{}/ProfilePage",
        crate::gamebanana::GAMEBANANA_API_BASE,
        request.member_id
    );
    let body = get_api_body(&url, "the member profile")?;
    let profile = parse_member_profile(&body)?;

    Ok(Pairing {
        member_id: request.member_id,
        name: profile.name,
        avatar_url: profile.avatar_url,
        profile_url: profile.profile_url,
        token: request.token.clone(),
    })
}

/// Read the member's pending queue.
///
/// GameBanana marks every entry it returns as retrieved, so the list must be installed once:
/// the next poll does not see those entries again. An entry that is not a valid one-click link
/// is skipped rather than failing the batch, and [`parse_queue`] collapses an entry queued
/// twice.
pub fn fetch_queue(pairing: &Pairing) -> Result<Vec<QueuedMod>> {
    let url = queue_url(pairing)?;
    let body = get_api_body(&url, "the Remote Install queue")?;
    parse_queue(&body)
}

/// Record that a queued item was taken.
///
/// GameBanana has no acknowledgement call: the queue read marks the whole batch retrieved, so an
/// item is acknowledged the moment [`fetch_queue`] returns it. This checks that the pairing and
/// the id are the shape a poll produced and sends nothing.
///
/// Keep the call: the installer records its own completion here, and a per-entry endpoint could
/// be added behind it without changing callers. A failed install cannot be retried through the
/// queue (the entry is gone from GameBanana), so the installer shows the error and offers the
/// mod page instead.
pub fn acknowledge(pairing: &Pairing, queue_id: &str) -> Result<()> {
    if pairing.member_id == 0 {
        return Err(Error::msg(
            "A Remote Install pairing needs a GameBanana member id.",
        ));
    }
    validate_token(&pairing.token)?;
    if !valid_queue_id(queue_id) {
        return Err(Error::msg(
            "That Remote Install id did not come from a queue read.",
        ));
    }
    Ok(())
}

/// Read a queue body. Pure, so a recorded body can be parsed without a request.
///
/// The recorded shape is a JSON array of `"<download url>,<Mod|Sound>,<mod id>"` strings. A
/// list under `_aRecords` is accepted too, in case GameBanana wraps it. `null` and an empty
/// array are an empty queue. An unusable entry is skipped, as Deadlock Mod Manager skips it, so
/// one bad item does not hold back the rest of the batch.
pub fn parse_queue(json: &str) -> Result<Vec<QueuedMod>> {
    let json = json.trim();
    if json.is_empty() {
        return Err(Error::msg("GameBanana sent an empty Remote Install queue."));
    }
    let value: Value = serde_json::from_str(json)
        .map_err(|error| Error::msg(format!("The Remote Install queue was not JSON ({error}).")))?;

    if let Some(code) = error_code(&value) {
        return Err(refusal(&code));
    }

    let list = match &value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        Value::Object(_) => match value.get("_aRecords") {
            Some(Value::Array(items)) => items,
            Some(Value::Null) => return Ok(Vec::new()),
            _ => return Err(no_list()),
        },
        _ => return Err(no_list()),
    };

    let mut queue: Vec<QueuedMod> = Vec::with_capacity(list.len());
    for item in list {
        let Some(entry) = item.as_str() else {
            continue;
        };
        let Some(queued) = parse_entry(entry) else {
            continue;
        };
        if !queue.iter().any(|seen| seen.queue_id == queued.queue_id) {
            queue.push(queued);
        }
    }
    Ok(queue)
}

/// One `"<download url>,<Mod|Sound>,<mod id>"` entry, or `None` when it is not one.
fn parse_entry(entry: &str) -> Option<QueuedMod> {
    let parts: Vec<&str> = entry.split(',').collect();
    if parts.len() != 3 {
        return None;
    }

    let url = WebUrl::parse(parts[0]).ok()?;
    if url.scheme != "https" || !gamebanana_host(&url.host) {
        return None;
    }

    let model = match parts[1].trim().to_ascii_lowercase().as_str() {
        "mod" => "Mod",
        "sound" => "Sound",
        _ => return None,
    };
    let mod_id: u64 = parts[2].trim().parse().ok()?;
    if mod_id == 0 {
        return None;
    }

    let file_id = file_id_of(&url.path);
    Some(QueuedMod {
        mod_id,
        file_id,
        queued_at: None,
        queue_id: queue_id(model, mod_id, file_id),
        model: model.to_string(),
        download_url: parts[0].trim().to_string(),
    })
}

/// `gamebanana.com` or one of its subdomains. The one-click links use the apex host and the
/// `files.` and `filecacheNN.` file hosts.
fn gamebanana_host(host: &str) -> bool {
    host == "gamebanana.com" || host.ends_with(".gamebanana.com")
}

/// The file id in `/mmdl/<id>`, or GameBanana's `/dl/<id>` redirect form.
fn file_id_of(path: &str) -> Option<u64> {
    let rest = path
        .strip_prefix("/mmdl/")
        .or_else(|| path.strip_prefix("/dl/"))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|id| *id > 0)
}

/// `<model>:<mod id>:<file id or 0>`, the stable id a queue entry lacks.
fn queue_id(model: &str, mod_id: u64, file_id: Option<u64>) -> String {
    format!(
        "{}:{mod_id}:{}",
        model.to_ascii_lowercase(),
        file_id.unwrap_or(0)
    )
}

/// True for an id [`queue_id`] can produce.
fn valid_queue_id(id: &str) -> bool {
    let mut parts = id.split(':');
    let (Some(model), Some(mod_id), Some(file_id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    matches!(model, "mod" | "sound")
        && mod_id.parse::<u64>().is_ok_and(|id| id > 0)
        && file_id.parse::<u64>().is_ok()
}

/// GameBanana issues 1 to [`MAX_TOKEN_LEN`] ASCII letters, digits, dashes or underscores. The
/// token becomes a path segment of the queue URL, so anything else is refused rather than
/// escaped, as Deadlock Mod Manager refuses it. A rejection never repeats the token.
fn validate_token(token: &str) -> Result<()> {
    let shaped = (1..=MAX_TOKEN_LEN).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if shaped {
        Ok(())
    } else {
        Err(Error::msg(
            "The pair link's key is empty or has characters GameBanana never sends. Press Enroll again and reopen the new link.",
        ))
    }
}

fn parse_member_id(text: &str) -> Result<u64> {
    let id: u64 = text.trim().parse().map_err(|_| {
        Error::msg(
            "That pair link has no GameBanana member id. Press Enroll again and open the new link.",
        )
    })?;
    if id == 0 {
        return Err(Error::msg(
            "That pair link has no GameBanana member id. Press Enroll again and open the new link.",
        ));
    }
    Ok(id)
}

/// The scheme a link starts with, if it is one of ours; the link is returned unchanged otherwise.
fn strip_scheme(link: &str) -> &str {
    let link = link.trim();
    PAIR_SCHEMES
        .iter()
        .find_map(|scheme| link.strip_prefix(scheme))
        .unwrap_or(link)
}

/// The queue URL. The token is the second path segment, which is why its shape is checked
/// first; the URL is never put in an error message or a log line.
fn queue_url(pairing: &Pairing) -> Result<String> {
    if pairing.member_id == 0 {
        return Err(Error::msg(
            "A Remote Install pairing needs a GameBanana member id.",
        ));
    }
    validate_token(&pairing.token)?;
    Ok(format!(
        "{}/RemoteInstall/{}/{}/{MANAGER_ALIAS}",
        crate::gamebanana::GAMEBANANA_API_BASE,
        pairing.member_id,
        pairing.token
    ))
}

/// A member profile: the name the player checks, and the public fields beside it.
struct MemberProfile {
    name: String,
    avatar_url: Option<String>,
    profile_url: String,
}

/// `Member/<id>/ProfilePage`. The name is the one field that cannot be missing: a pairing with
/// nobody named is not one the player can confirm.
fn parse_member_profile(body: &str) -> Result<MemberProfile> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        Error::msg(format!(
            "GameBanana sent a member profile that was not JSON ({error})."
        ))
    })?;
    if let Some(code) = error_code(&value) {
        return Err(Error::msg(format!(
            "GameBanana rejected the member profile ({code})."
        )));
    }

    let name = text_field(&value, "_sName");
    if name.is_empty() {
        return Err(Error::msg(
            "GameBanana sent a member profile with no name. Check the account and pair again.",
        ));
    }

    Ok(MemberProfile {
        name,
        avatar_url: Some(text_field(&value, "_sAvatarUrl")).filter(|url| !url.is_empty()),
        profile_url: text_field(&value, "_sProfileUrl"),
    })
}

/// `_sErrorCode` from a refusal body: the shape GameBanana uses for posts and Remote Install.
/// Some refusals arrive with HTTP 200, so the body is checked, not the status.
fn error_code(value: &Value) -> Option<String> {
    value
        .get("_sErrorCode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string)
}

fn refusal(code: &str) -> Error {
    Error::msg(format!(
        "GameBanana rejected the Remote Install queue ({code}). Press Enroll and Pair again on GameBanana."
    ))
}

fn no_list() -> Error {
    Error::msg("The Remote Install queue had no list of entries.")
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// One GET against the public API with the shared TLS setup, timeout, offline gate and proxy.
/// `what` names the request in an error; the URL never does, because a queue URL holds a token.
/// Some refusals arrive as HTTP 200 with an `_sErrorCode`, so the body is always returned for
/// the caller's parser to read.
fn get_api_body(url: &str, what: &str) -> Result<String> {
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
        .build()
        .new_agent();

    let send = || {
        agent
            .get(url)
            .header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT)
            .header("Accept", "application/json")
            .call()
    };
    // One retry after a short pause: a poll that meets a rate limit or a restarted server should
    // not cost the player another five minutes.
    let mut response = match send() {
        Err(error) if retryable(&error) => {
            std::thread::sleep(Duration::from_millis(RETRY_DELAY_MILLIS));
            send()
        }
        other => other,
    }
    .map_err(|error| {
        Error::Network(format!(
            "GameBanana could not be reached for {what}: {error}"
        ))
    })?;

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|error| {
            Error::Network(format!(
                "GameBanana sent an unreadable answer for {what}: {error}"
            ))
        })?;

    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "GameBanana answered HTTP {status} for {what}."
        )));
    }

    Ok(body)
}

/// A failure worth one more try: HTTP 429 or 5xx, a timeout, or a reset connection. Offline
/// mode's refusal is not, and neither is a name that does not resolve.
fn retryable(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => true,
        ureq::Error::Io(io) => io.kind() != std::io::ErrorKind::PermissionDenied,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token shaped like the one GameBanana issued in the recorded pair link.
    const TOKEN: &str = "DF385693F33BABD5";

    fn pairing() -> Pairing {
        Pairing::new(4_765_184, "Kalo".to_string(), TOKEN).unwrap()
    }

    #[test]
    fn reads_the_recorded_queue_shape() {
        let body = r#"[
            "https://gamebanana.com/mmdl/1414905,Mod,715100",
            "https://files.gamebanana.com/mmdl/13,Sound,42"
        ]"#;
        let queue = parse_queue(body).unwrap();
        assert_eq!(queue.len(), 2);

        let first = &queue[0];
        assert_eq!(first.mod_id, 715_100);
        assert_eq!(first.file_id, Some(1_414_905));
        assert_eq!(first.model, "Mod");
        assert_eq!(first.download_url, "https://gamebanana.com/mmdl/1414905");
        assert_eq!(first.queue_id, "mod:715100:1414905");
        assert_eq!(first.queued_at, None);

        let sound = &queue[1];
        assert_eq!(sound.mod_id, 42);
        assert_eq!(sound.file_id, Some(13));
        assert_eq!(sound.model, "Sound");
        assert_eq!(sound.queue_id, "sound:42:13");
    }

    #[test]
    fn accepts_a_wrapped_list_and_reads_null_as_empty() {
        let wrapped = r#"{"_aRecords":["https://gamebanana.com/mmdl/7,Sound,42"],"_aMetadata":{"_nRecordCount":1}}"#;
        assert_eq!(parse_queue(wrapped).unwrap().len(), 1);
        assert!(parse_queue("null").unwrap().is_empty());
        assert!(parse_queue("[]").unwrap().is_empty());
        assert!(parse_queue(r#"{"_aRecords":null}"#).unwrap().is_empty());
    }

    #[test]
    fn skips_bad_entries_without_dropping_the_batch() {
        let body = r#"[
            "http://gamebanana.com/mmdl/1,Mod,2",
            "https://gamebanana.com.evil.test/mmdl/1,Mod,2",
            "https://gamebanana.com/mmdl/1,Map,2",
            "https://gamebanana.com/mmdl/1,Mod,0",
            "https://gamebanana.com/mmdl/1,Mod",
            "https://gamebanana.com/mmdl/9,Mod,7,extra",
            {"_sUrl":"https://gamebanana.com/mmdl/1,Mod,2"},
            7,
            "https://gamebanana.com/mmdl/1414905,Mod,715100"
        ]"#;
        let queue = parse_queue(body).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].mod_id, 715_100);
    }

    #[test]
    fn a_malformed_or_refused_answer_is_an_error() {
        assert!(parse_queue("").is_err());
        assert!(parse_queue("   ").is_err());
        assert!(parse_queue("not json").is_err());
        assert!(parse_queue("{}").is_err());
        assert!(parse_queue(r#""https://gamebanana.com/mmdl/1,Mod,2""#).is_err());
        assert!(parse_queue(r#"{"_aRecords":"nope"}"#).is_err());

        let refused = parse_queue(r#"{"_sErrorCode":"NOT_ENROLLED"}"#)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("NOT_ENROLLED"), "{refused}");
        assert!(refused.contains("GameBanana"), "{refused}");
    }

    #[test]
    fn the_same_entry_queued_twice_appears_once() {
        let body = r#"[
            "https://gamebanana.com/mmdl/1414905,Mod,715100",
            "https://filecache12.gamebanana.com/mmdl/1414905,Mod,715100",
            "https://gamebanana.com/mmdl/8,Mod,715100"
        ]"#;
        let queue = parse_queue(body).unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].queue_id, "mod:715100:1414905");
        assert_eq!(queue[1].queue_id, "mod:715100:8");
    }

    #[test]
    fn debug_redacts_the_token() {
        let pairing = pairing();
        let debug = format!("{pairing:?}");
        assert!(debug.contains("4765184"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains(TOKEN), "{debug}");

        let request = pairing_start("sigil://registerKey,4765184,DF385693F33BABD5").unwrap();
        let debug = format!("{request:?}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains(TOKEN), "{debug}");
    }

    #[test]
    fn pair_links_are_read_with_or_without_a_scheme() {
        for link in [
            "sigil:registerKey,4765184,DF385693F33BABD5",
            "sigil://registerKey,4765184,DF385693F33BABD5",
            "sigil://registerKey,4765184,DF385693F33BABD5/",
            "deadlock-mod-manager:registerKey,4765184,DF385693F33BABD5",
            "deadlock-modmanager:registerKey,4765184,DF385693F33BABD5/",
            "  registerKey,4765184,DF385693F33BABD5  ",
        ] {
            let request = pairing_start(link).unwrap_or_else(|error| panic!("{link}: {error}"));
            assert_eq!(request.member_id, 4_765_184, "{link}");
            assert_eq!(request.token(), TOKEN, "{link}");
        }
    }

    #[test]
    fn pair_links_that_are_not_ours_are_refused_without_echoing_the_key() {
        let long = "a".repeat(MAX_TOKEN_LEN + 1);
        for link in [
            "https://gamebanana.com/mods/42",
            "sigil://install?mod=1",
            "sigil:registerKey,not-a-member,DF385693F33BABD5",
            "sigil:registerKey,0,DF385693F33BABD5",
            "sigil:registerKey,4765184,",
            "sigil:registerKey,4765184,key/../other",
            "sigil:registerKey,4765184,with space",
            "sigil:registerKey,4765184",
            "sigil:registerKey",
        ] {
            let error = pairing_start(link).unwrap_err().to_string();
            assert!(!error.contains("key/../other"), "{link}: {error}");
            assert!(!error.contains("with space"), "{link}: {error}");
        }
        assert!(pairing_start(&format!("sigil:registerKey,1,{long}")).is_err());
    }

    #[test]
    fn tokens_are_url_safe_and_bounded() {
        assert!(validate_token("DF385693F33BABD5").is_ok());
        assert!(validate_token("a-b_c").is_ok());
        assert!(validate_token(&"a".repeat(MAX_TOKEN_LEN)).is_ok());
        for bad in ["", "a b", "a/b", "a.b", "a?b", "a%b", "aéb"] {
            assert!(validate_token(bad).is_err(), "{bad}");
        }
        assert!(validate_token(&"a".repeat(MAX_TOKEN_LEN + 1)).is_err());
    }

    #[test]
    fn the_queue_url_names_the_member_the_token_and_the_alias() {
        let url = queue_url(&pairing()).unwrap();
        assert!(
            url.starts_with(
                "https://gamebanana.com/apiv11/RemoteInstall/4765184/DF385693F33BABD5/"
            ),
            "{url}"
        );
        assert!(url.ends_with(&format!("/{MANAGER_ALIAS}")), "{url}");
        assert_eq!(POLL_INTERVAL_SECONDS, 300);
    }

    #[test]
    fn acknowledge_accepts_a_polled_id_and_refuses_anything_else() {
        let pairing = pairing();
        for good in ["mod:715100:1414905", "sound:42:13", "mod:715100:0"] {
            assert!(acknowledge(&pairing, good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "715100",
            "mod:715100",
            "mod:0:1",
            "map:1:1",
            "mod:715100:1414905:extra",
            "mod:x:1",
            "mod:1:x",
            "MOD:1:1",
        ] {
            assert!(acknowledge(&pairing, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_a_member_profile() {
        let body = r#"{"_idRow":4765184,"_sName":"Kalo","_sAvatarUrl":"https://images.gamebanana.com/img/av/1.jpg","_sProfileUrl":"https://gamebanana.com/members/4765184"}"#;
        let profile = parse_member_profile(body).unwrap();
        assert_eq!(profile.name, "Kalo");
        assert_eq!(
            profile.avatar_url.as_deref(),
            Some("https://images.gamebanana.com/img/av/1.jpg")
        );
        assert_eq!(
            profile.profile_url,
            "https://gamebanana.com/members/4765184"
        );

        let sparse = parse_member_profile(r#"{"_sName":" Kalo "}"#).unwrap();
        assert_eq!(sparse.name, "Kalo");
        assert_eq!(sparse.avatar_url, None);
        assert_eq!(sparse.profile_url, "");

        assert!(parse_member_profile(r#"{"_sName":""}"#).is_err());
        assert!(parse_member_profile(r#"{"_sErrorCode":"NOT_FOUND"}"#).is_err());
        assert!(parse_member_profile("nope").is_err());
    }

    #[test]
    fn a_restored_pairing_checks_its_values() {
        let restored = pairing();
        assert_eq!(restored.member_id, 4_765_184);
        assert_eq!(restored.name, "Kalo");
        assert_eq!(restored.avatar_url, None);
        assert_eq!(restored.profile_url, "");
        assert_eq!(restored.token(), TOKEN);

        assert!(Pairing::new(0, "Kalo".to_string(), TOKEN).is_err());
        let error = Pairing::new(1, "Kalo".to_string(), "not a token")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("not a token"), "{error}");
    }
}
