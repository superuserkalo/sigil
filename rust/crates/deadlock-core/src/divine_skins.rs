//! Divine Skins installs from a pasted link.
//!
//! Divine Skins (divineskins.gg) publishes Deadlock mods and gives each mod page a
//! "Deadlock Mod Manager" button. That button sends
//! `deadlock-mod-manager://install?source=divineskins&mod=<id>&release=<id>`; this module reads
//! the manifest behind it and checks every field before the UI offers the file to the player.
//! Nothing here downloads or installs: [`validate`] returns a [`ValidatedFile`] that the existing
//! install queue can take once the player agrees.
//!
//! A pasted page link is accepted too, but only the forms that carry a mod id in the path:
//! `https://divineskins.gg/skins/<id>`, `/explore-mods/<id>` and `/download/<id>` (the `www` host
//! works as well). The site's share button copies `/<creator>/<slug>` instead. That link carries
//! no id and no release, so [`parse_link`] rejects it rather than guessing; the page's Deadlock
//! Mod Manager button, or the site's own API, is the only way to turn it into an id.
//!
//! A link without a release (a page link) is completed from
//! `https://api.divineskins.gg/api/v1/mods/<mod>/releases`, the public list the site's Download
//! button uses: the release whose `status` is `current` is taken. If none is marked current the
//! fetch fails instead of guessing.
//!
//! Network: `api.divineskins.gg` only. Manifests are capped at [`MAX_MANIFEST_BYTES`]. The
//! manifest may name an archive on `lol-mods.divine-cdn.com` and a cover image on
//! `lol-assets.divine-cdn.com` or `images.divine-cdn.com`; both are exact hosts, https only, with
//! no credentials and no custom port. The four hosts are listed in the network allowlist in
//! `docs/architecture.md`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// The manifest format Deadlock Mod Manager documents and Divine Skins sends.
pub const MANIFEST_FORMAT: &str = "dmm-one-click";
/// The only manifest version this reader understands.
pub const MANIFEST_VERSION: u32 = 1;
/// A manifest is a few hundred bytes; anything near this is not one.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The same ceiling the install pipeline applies to a staged archive.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Archive types the install pipeline can open.
pub const INSTALLABLE_EXTENSIONS: &[&str] = &["zip", "7z", "rar", "vpk"];

/// Identifies this client without naming a user or an account.
pub const USER_AGENT: &str = "Sigil";

const API_BASE: &str = "https://api.divineskins.gg/api/v1";
const SITE_HOSTS: &[&str] = &["divineskins.gg", "www.divineskins.gg"];
const DOWNLOAD_HOSTS: &[&str] = &["lol-mods.divine-cdn.com"];
const MEDIA_HOSTS: &[&str] = &["lol-assets.divine-cdn.com", "images.divine-cdn.com"];
/// `sigil:` is Sigil's own scheme, for a site that adds a Sigil button with the same ids.
const INSTALL_SCHEMES: &[&str] = &["sigil:", "deadlock-mod-manager:", "deadlock-modmanager:", "dlmm:"];
const MAX_ID_LEN: usize = 64;
const MAX_TEXT_LEN: usize = 120;
const MAX_URL_LEN: usize = 8192;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// A Divine Skins mod and, when the link names one, its release.
///
/// Both ids are checked like Deadlock Mod Manager checks them: 1 to 64 ASCII letters, digits,
/// dashes or underscores. A page link carries no release; [`fetch_manifest`] fills in the
/// current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DivineLink {
    pub mod_id: String,
    pub release_id: Option<String>,
}

/// The one-click manifest at `.../mods/{mod}/versions/{release}/dmm`, unwrapped from its
/// `data` envelope. Fields are the documented `dmm-one-click` 1 fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DivineManifest {
    pub format: String,
    pub version: u32,
    pub mod_id: String,
    pub release_id: String,
    pub name: String,
    pub author: Option<String>,
    pub page_url: String,
    pub image_url: Option<String>,
    pub file: DivineFile,
}

/// The archive a manifest offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DivineFile {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// A manifest that passed every check, ready for the install queue.
///
/// The archive and the cover image have not been fetched. `download_url` is the checked
/// `lol-mods.divine-cdn.com` address and `sha256` is the hash the queue checks after the
/// download, so both belong in Rust, not in a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedFile {
    /// The mod's display name, trimmed and capped.
    pub name: String,
    pub author: Option<String>,
    /// The mod page the manifest names, on the Divine Skins site.
    pub page_url: String,
    /// A cover image, only when it is on a Divine Skins media host.
    pub image_url: Option<String>,
    pub file_name: String,
    pub download_url: String,
    pub size: u64,
    /// Lowercase hex SHA-256 the install queue checks after the download.
    pub sha256: String,
}

/// Read a pasted link or page address.
///
/// Accepts the Deadlock Mod Manager install link (the three schemes that app registers, so a
/// button written for any of them works) and the Divine Skins page forms that carry a mod id in
/// the path. Junk, another source, a lookalike host, a custom port, credentials or an encoded path
/// trick all return `None`.
pub fn parse_link(text: &str) -> Option<DivineLink> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if INSTALL_SCHEMES
        .iter()
        .any(|scheme| text.starts_with(scheme))
    {
        return parse_install_link(text);
    }
    parse_page_link(text)
}

fn parse_install_link(text: &str) -> Option<DivineLink> {
    let rest = INSTALL_SCHEMES
        .iter()
        .find_map(|scheme| text.strip_prefix(scheme))?;
    let rest = rest.strip_prefix("//install")?;
    let query = rest.strip_prefix('/').unwrap_or(rest);
    let query = query.strip_prefix('?')?.trim_end_matches('/');

    let mut source = None;
    let mut mod_id = None;
    let mut release_id = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "source" => source = Some(value),
            "mod" => mod_id = Some(value),
            "release" => release_id = Some(value),
            _ => {}
        }
    }

    if !source.is_some_and(|value| value.eq_ignore_ascii_case("divineskins")) {
        return None;
    }
    let mod_id = mod_id?;
    let release_id = release_id?;
    if !valid_id(mod_id) || !valid_id(release_id) {
        return None;
    }
    Some(DivineLink {
        mod_id: mod_id.to_string(),
        release_id: Some(release_id.to_string()),
    })
}

fn parse_page_link(text: &str) -> Option<DivineLink> {
    let (host, rest) = https_parts(text)?;
    if !host_allowed(host, SITE_HOSTS) {
        return None;
    }
    let path = rest.split(['?', '#']).next()?;
    let path = path.trim_matches('/');
    let mut segments = path.split('/');
    let route = segments.next()?;
    let id = segments.next()?;
    if segments.next().is_some() || !matches!(route, "skins" | "explore-mods" | "download") {
        return None;
    }
    Some(DivineLink {
        mod_id: valid_id(id).then(|| id.to_string())?,
        release_id: None,
    })
}

/// Split an https URL into its host and the part after it.
///
/// The scheme is matched without case; the authority must have no credentials and no port, and
/// the URL must be plain ASCII with no spaces or control characters. This is a check, not a
/// general URL parser: the hosts this module accepts are exact names.
fn https_parts(url: &str) -> Option<(&str, &str)> {
    let url = url.trim();
    if url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_control() || c == ' ' || c == '\\') {
        return None;
    }
    let scheme = url.get(..8)?;
    if !scheme.eq_ignore_ascii_case("https://") {
        return None;
    }
    let rest = url.get(8..)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    if host.is_empty() || host.contains('@') || host.contains(':') {
        return None;
    }
    Some((host, &rest[end..]))
}

fn host_allowed(host: &str, hosts: &[&str]) -> bool {
    hosts
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

fn is_https_on(url: &str, hosts: &[&str]) -> bool {
    https_parts(url).is_some_and(|(host, _)| host_allowed(host, hosts))
}

fn valid_id(id: &str) -> bool {
    (1..=MAX_ID_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Fetch the manifest a link names.
///
/// A link without a release reads the current release from the public release list first. The
/// answer must describe the mod and release that were asked for; a manifest for anything else is
/// refused, so a link can never install a different mod from the one the player chose.
pub fn fetch_manifest(link: &DivineLink) -> Result<DivineManifest> {
    if !valid_id(&link.mod_id) {
        return Err(Error::msg("That Divine Skins link has no valid mod id."));
    }
    if let Some(release) = &link.release_id
        && !valid_id(release)
    {
        return Err(Error::msg(
            "That Divine Skins link has no valid release id.",
        ));
    }

    let release_id = match &link.release_id {
        Some(release) => release.clone(),
        None => current_release(&link.mod_id)?,
    };
    let url = format!("{API_BASE}/mods/{}/versions/{release_id}/dmm", link.mod_id);
    let manifest = parse_manifest(&get_json(&url)?)?;
    if manifest.mod_id != link.mod_id || manifest.release_id != release_id {
        return Err(Error::msg(
            "Divine Skins sent a manifest for a different mod. Nothing was installed.",
        ));
    }
    Ok(manifest)
}

/// The release the site's Download button serves: the one the public release list marks
/// `current`. Older releases are archived and are not picked. No current release is an error,
/// never a guess.
fn current_release(mod_id: &str) -> Result<String> {
    let url = format!("{API_BASE}/mods/{mod_id}/releases");
    let body = get_json(&url)?;
    let document: Value = serde_json::from_str(&body).map_err(|error| {
        Error::msg(format!(
            "Divine Skins sent a release list Sigil cannot read: {error}"
        ))
    })?;
    let releases = document
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("Divine Skins sent no release list for that mod."))?;
    releases
        .iter()
        .find(|release| release.get("status").and_then(Value::as_str) == Some("current"))
        .and_then(|release| release.get("id"))
        .and_then(|id| {
            id.as_u64()
                .map(|n| n.to_string())
                .or_else(|| id.as_str().map(str::to_string))
        })
        .filter(|id| valid_id(id))
        .ok_or_else(|| Error::msg("Divine Skins has no current release for that mod."))
}

/// Turn the endpoint's answer into a manifest. Kept separate from the request so a saved body can
/// be read without network access.
pub fn parse_manifest(body: &str) -> Result<DivineManifest> {
    let mut document: Value = serde_json::from_str(body).map_err(|error| {
        Error::msg(format!(
            "Divine Skins sent something that is not an install manifest: {error}"
        ))
    })?;
    let data = document
        .get_mut("data")
        .map(Value::take)
        .ok_or_else(|| Error::msg("Divine Skins sent no install manifest."))?;
    serde_json::from_value(data).map_err(|error| {
        Error::msg(format!(
            "Divine Skins sent an invalid install manifest: {error}"
        ))
    })
}

/// Check a manifest and return the plain data the install dialog shows.
///
/// The page stays on the Divine Skins site, the archive on `lol-mods.divine-cdn.com`, and the
/// cover image on a Divine Skins media host, all https, exact hosts, no credentials, no port. A
/// cover image on another host is dropped rather than failing the install. The file must be a
/// zip, 7z, rar or vpk, between 1 byte and [`MAX_FILE_BYTES`], with a safe file name and a
/// SHA-256.
pub fn validate(manifest: &DivineManifest) -> Result<ValidatedFile> {
    if manifest.format.trim() != MANIFEST_FORMAT || manifest.version != MANIFEST_VERSION {
        return Err(invalid("its format is not dmm-one-click 1"));
    }
    let name = clean_text(&manifest.name).ok_or_else(|| invalid("it has no mod name"))?;
    let page_url = manifest.page_url.trim();
    if !is_https_on(page_url, SITE_HOSTS) {
        return Err(invalid("the mod page is not on divineskins.gg"));
    }
    let file_name = crate::download_queue::safe_file_name(&manifest.file.name)
        .ok_or_else(|| invalid("the file name is unsafe"))?;
    if !has_installable_extension(&file_name) {
        return Err(invalid("the file is not a zip, 7z, rar or vpk"));
    }
    let download_url = manifest.file.url.trim();
    if !is_https_on(download_url, DOWNLOAD_HOSTS) {
        return Err(invalid("the file is not on lol-mods.divine-cdn.com"));
    }
    if manifest.file.size == 0 || manifest.file.size > MAX_FILE_BYTES {
        return Err(invalid("the file size is out of range"));
    }
    if !is_sha256(&manifest.file.sha256) {
        return Err(invalid("the file has no SHA-256"));
    }
    // The cover is decoration: a link somewhere else is dropped, not fatal.
    let image_url = manifest
        .image_url
        .as_deref()
        .map(str::trim)
        .filter(|url| is_https_on(url, MEDIA_HOSTS))
        .map(str::to_string);

    Ok(ValidatedFile {
        name,
        author: manifest.author.as_deref().and_then(clean_text),
        page_url: page_url.to_string(),
        image_url,
        file_name,
        download_url: download_url.to_string(),
        size: manifest.file.size,
        sha256: manifest.file.sha256.to_ascii_lowercase(),
    })
}

fn invalid(reason: &str) -> Error {
    Error::msg(format!(
        "Divine Skins sent an invalid install manifest: {reason}."
    ))
}

fn has_installable_extension(file_name: &str) -> bool {
    std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            INSTALLABLE_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

/// Drop control characters, trim, and cap the length. `None` when nothing is left.
fn clean_text(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(MAX_TEXT_LEN).collect())
}

fn is_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// One GET with the shared TLS setup, the offline gate, the proxy from Settings and a response
/// cap. Redirects are off: the addresses are fixed, so a redirect could only leave the list.
fn get_json(url: &str) -> Result<String> {
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
        .max_redirects(0)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(status) => {
                Error::Network(format!("Divine Skins answered HTTP {status}."))
            }
            other => Error::Network(format!("Divine Skins could not be reached: {other}")),
        })?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST_BYTES)
        .read_to_string()
        .map_err(|error| Error::Network(format!("Divine Skins sent an unreadable answer: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "fcfe4bdeb53cf44dcdd5966e1a4538807ad17dccab80df76400f9397c114f6a5";

    const BODY: &str = r#"{"data": {
      "format": "dmm-one-click", "version": 1, "modId": "123", "releaseId": "456",
      "name": "Abrams Ult Airhorn", "author": "someone",
      "pageUrl": "https://divineskins.gg/deadlock/abrams-ult-airhorn",
      "imageUrl": "https://lol-assets.divine-cdn.com/123/cover.webp",
      "file": {"name": "abrams-ult-airhorn-1.0.zip",
        "url": "https://lol-mods.divine-cdn.com/123/7f1c.zip",
        "size": 1667455,
        "sha256": "FCFE4BDEB53CF44DCDD5966E1A4538807AD17DCCAB80DF76400F9397C114F6A5"}
    }}"#;

    fn manifest() -> DivineManifest {
        DivineManifest {
            format: MANIFEST_FORMAT.to_string(),
            version: MANIFEST_VERSION,
            mod_id: "123".to_string(),
            release_id: "456".to_string(),
            name: "Abrams Ult Airhorn".to_string(),
            author: Some("someone".to_string()),
            page_url: "https://divineskins.gg/deadlock/abrams-ult-airhorn".to_string(),
            image_url: Some("https://lol-assets.divine-cdn.com/123/cover.webp".to_string()),
            file: DivineFile {
                name: "abrams-ult-airhorn-1.0.zip".to_string(),
                url: "https://lol-mods.divine-cdn.com/123/7f1c.zip".to_string(),
                size: 1_667_455,
                sha256: SHA.to_uppercase(),
            },
        }
    }

    fn link() -> DivineLink {
        DivineLink {
            mod_id: "123".to_string(),
            release_id: Some("456".to_string()),
        }
    }

    #[test]
    fn parses_the_link_a_site_sends() {
        // Sites hardcode this shape, so a change here breaks their buttons.
        assert_eq!(
            parse_link("deadlock-mod-manager://install?source=divineskins&mod=123&release=456"),
            Some(link())
        );
        for text in [
            "deadlock-mod-manager://install/?source=divineskins&mod=123&release=456",
            "deadlock-mod-manager://install?release=456&mod=123&source=divineskins",
            "deadlock-mod-manager://install?source=divineskins&mod=123&release=456/",
            "deadlock-modmanager://install?source=divineskins&mod=123&release=456",
            "dlmm://install?source=divineskins&mod=123&release=456",
            "sigil://install?source=divineskins&mod=123&release=456",
        ] {
            assert_eq!(parse_link(text), Some(link()), "{text}");
        }
    }

    #[test]
    fn parses_page_links_that_carry_an_id() {
        for text in [
            "https://divineskins.gg/skins/4020",
            "https://www.divineskins.gg/skins/4020/",
            "https://divineskins.gg/explore-mods/4020?from=catalog#top",
            "https://divineskins.gg/download/4020",
            "  https://DIVINESKINS.GG/skins/4020  ",
        ] {
            assert_eq!(
                parse_link(text),
                Some(DivineLink {
                    mod_id: "4020".to_string(),
                    release_id: None,
                }),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_links_that_are_not_a_divine_skins_install() {
        for text in [
            "",
            "junk",
            "divineskins.gg/skins/4020",
            "http://divineskins.gg/skins/4020",
            "https://evil.example/skins/4020",
            "https://divineskins.gg.evil.example/skins/4020",
            "https://divineskins.gg./skins/4020",
            "https://user:pass@divineskins.gg/skins/4020",
            "https://divineskins.gg:8443/skins/4020",
            // Share links copy the creator and the slug, not an id.
            "https://divineskins.gg/SoliD737/osic-elite-haze",
            "https://divineskins.gg/deadlock/abrams-ult-airhorn",
            // Only the three page routes that take an id.
            "https://divineskins.gg/mods/4020",
            "https://divineskins.gg/skins",
            "https://divineskins.gg/skins/",
            "https://divineskins.gg/skins/4020/extra",
            "https://divineskins.gg/skins//4020",
            "https://divineskins.gg/skins/..%2F4020",
            "https://divineskins.gg/skins/4020%3Fevil",
        ] {
            assert_eq!(parse_link(text), None, "{text}");
        }
    }

    #[test]
    fn rejects_unsafe_install_links() {
        for text in [
            "deadlock-mod-manager://install?source=evil&mod=123&release=456",
            "deadlock-mod-manager://install?source=divineskins&mod=..%2F123&release=456",
            "deadlock-mod-manager://install?source=divineskins&mod=123&release=4%3F6",
            "deadlock-mod-manager://install?source=divineskins&mod=&release=456",
            "deadlock-mod-manager://install?source=divineskins&mod=123",
            "deadlock-mod-manager://installer?source=divineskins&mod=123&release=456",
            "deadlock-mod-manager://install",
        ] {
            assert_eq!(parse_link(text), None, "{text}");
        }
        let long = "1".repeat(MAX_ID_LEN + 1);
        assert_eq!(
            parse_link(&format!(
                "deadlock-mod-manager://install?source=divineskins&mod={long}&release=456"
            )),
            None
        );
        let longest = "1".repeat(MAX_ID_LEN);
        assert!(
            parse_link(&format!(
                "deadlock-mod-manager://install?source=divineskins&mod={longest}&release=456"
            ))
            .is_some()
        );
    }

    #[test]
    fn reads_the_manifest_from_the_source_envelope() {
        let parsed = parse_manifest(BODY).expect("should parse");
        assert_eq!(parsed, manifest());
        assert!(parse_manifest(r#"{"format": "dmm-one-click"}"#).is_err());
        assert!(parse_manifest(r#"{"data": {"modId": 123}}"#).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn accepts_a_valid_manifest() {
        let validated = validate(&manifest()).expect("should be valid");
        assert_eq!(validated.name, "Abrams Ult Airhorn");
        assert_eq!(validated.file_name, "abrams-ult-airhorn-1.0.zip");
        assert_eq!(validated.size, 1_667_455);
        assert_eq!(validated.sha256, SHA);
        assert_eq!(validated.download_url, manifest().file.url);
        assert!(validated.image_url.is_some());
    }

    #[test]
    fn only_plain_https_on_the_named_hosts_is_allowed() {
        for url in [
            "http://lol-mods.divine-cdn.com/123/7f1c.zip",
            "https://evil.example/123/7f1c.zip",
            "https://lol-mods.divine-cdn.com.evil.example/123/7f1c.zip",
            "https://cdn.lol-mods.divine-cdn.com/123/7f1c.zip",
            "https://user:pass@lol-mods.divine-cdn.com/123/7f1c.zip",
            "https://lol-mods.divine-cdn.com:8443/123/7f1c.zip",
        ] {
            let mut bad = manifest();
            bad.file.url = url.to_string();
            assert!(validate(&bad).is_err(), "{url}");
        }

        for url in [
            "http://divineskins.gg/deadlock/abrams-ult-airhorn",
            "https://evil.example/deadlock/abrams-ult-airhorn",
            "https://divineskins.gg:8443/deadlock/abrams-ult-airhorn",
        ] {
            let mut bad = manifest();
            bad.page_url = url.to_string();
            assert!(validate(&bad).is_err(), "{url}");
        }
    }

    #[test]
    fn refuses_unsafe_or_unsupported_files() {
        for name in ["../mod.zip", "folder/mod.zip", "mod.exe", "mod", "", ".zip"] {
            let mut bad = manifest();
            bad.file.name = name.to_string();
            assert!(validate(&bad).is_err(), "{name:?}");
        }
        for size in [0, MAX_FILE_BYTES + 1] {
            let mut bad = manifest();
            bad.file.size = size;
            assert!(validate(&bad).is_err(), "{size}");
        }
        let not_hex = "g".repeat(64);
        for hash in ["", "abc", not_hex.as_str()] {
            let mut bad = manifest();
            bad.file.sha256 = hash.to_string();
            assert!(validate(&bad).is_err(), "{hash}");
        }
        let mut wrong_format = manifest();
        wrong_format.format = "something-else".to_string();
        assert!(validate(&wrong_format).is_err());
        let mut wrong_version = manifest();
        wrong_version.version = 2;
        assert!(validate(&wrong_version).is_err());
    }

    #[test]
    fn drops_a_preview_on_another_host() {
        let mut foreign = manifest();
        foreign.image_url = Some("https://tracker.example/pixel.png".to_string());
        assert_eq!(validate(&foreign).expect("still valid").image_url, None);
        for url in [
            "https://images.divine-cdn.com/123/cover.webp",
            "https://lol-assets.divine-cdn.com/123/cover.webp",
        ] {
            let mut good = manifest();
            good.image_url = Some(url.to_string());
            assert_eq!(
                validate(&good).expect("valid").image_url.as_deref(),
                Some(url)
            );
        }
    }

    #[test]
    fn cleans_text_fields() {
        let mut messy = manifest();
        messy.name = "  Abrams\u{0007} Skin  ".to_string();
        messy.author = Some("   ".to_string());
        let validated = validate(&messy).expect("should be valid");
        assert_eq!(validated.name, "Abrams Skin");
        assert_eq!(validated.author, None);

        let mut long = manifest();
        long.name = "n".repeat(MAX_TEXT_LEN + 40);
        assert_eq!(
            validate(&long).expect("valid").name.chars().count(),
            MAX_TEXT_LEN
        );
    }
}
