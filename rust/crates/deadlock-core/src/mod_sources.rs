//! Third-party mod sources: a JSON descriptor at an https address names a catalog of mods and
//! the origins its API, files and images live on. Sigil reads the `dmm-source` 1.0 format that
//! Deadlock Mod Manager documents (`apps/docs/.../mod-sources.mdx`), so a community source
//! made for that app works here too. The format is data; this reader is Sigil's own.
//!
//! Network: only the origins a descriptor declares are contacted, https only, never a private
//! address, every request through [`crate::net`] (offline mode and the proxy apply). The user
//! adds each source by hand; `docs/architecture.md` lists "a mod source you add" in the
//! network allowlist.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

pub const FORMAT: &str = "dmm-source";
pub const PROTOCOL: &str = "1.0";
pub const MAX_JSON_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_PAGES: usize = 100;
pub const MAX_LISTINGS: usize = 10_000;
pub const MAX_DOCUMENTS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Catalog {
    Static { index_url: String },
    Api { list_url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub url: String,
    pub source_id: String,
    pub name: String,
    pub catalog: Catalog,
    pub api: Vec<String>,
    pub downloads: Vec<String>,
    pub media: Vec<String>,
    pub homepage: Option<String>,
    /// The descriptor URL's origin.
    pub origin: String,
    /// `capabilities.releaseHistory`: listings have a read-only changelog of past releases.
    pub release_history: bool,
    /// `catalog.releaseHistoryUrlTemplate`, resolved, with `{listingId}` left in.
    pub history_template: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Artifact {
    pub id: String,
    pub filename: String,
    pub url: Option<String>,
    pub resolver_url: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub id: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub nsfw: Option<bool>,
    pub tags: Vec<String>,
    pub image: Option<String>,
    pub updated_at: String,
    pub version: String,
    pub artifacts: Vec<Artifact>,
    pub detail_url: Option<String>,
    pub removed: bool,
    /// Every image the listing names, resolved; `image` is the first.
    pub images: Vec<String>,
    /// `descriptionFormat` is `markdown`: [`markdown_to_text`] reads it.
    pub markdown: bool,
    /// The listing's own page (`homepageUrl`).
    pub homepage: Option<String>,
    /// The current release's id.
    pub release_id: String,
    /// `releaseHistoryUrl`, resolved, when the listing names one.
    pub history_url: Option<String>,
}

/// One past or current release from a source's release history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Release {
    pub id: String,
    pub version: String,
    /// RFC 3339, as the source wrote it.
    pub published_at: String,
    /// Plain text.
    pub changelog: String,
}

/// Lowercase `scheme://host[:port]` of an absolute URL.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

fn is_private_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified();
    }
    host.starts_with('[')
}

/// An https URL with a public host, no credentials, at most 8192 characters.
pub fn check_url(url: &str) -> Result<()> {
    let url = url.trim();
    if url.len() > 8192 {
        return Err(Error::msg("That address is too long."));
    }
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("https://") else {
        return Err(Error::msg("A source address starts with https://"));
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err(Error::msg("That address has no host, or has a user name in it."));
    }
    if is_private_host(host) {
        return Err(Error::msg("Sources on this computer or the local network are not allowed."));
    }
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(Error::msg("That address has spaces in it."));
    }
    Ok(())
}

/// Resolve `reference` against `base` (absolute, root-relative or relative).
pub fn resolve(base: &str, reference: &str) -> String {
    let r = reference.trim();
    if r.contains("://") {
        return r.to_string();
    }
    let origin = origin_of(base).unwrap_or_default();
    if r.starts_with('/') {
        return format!("{origin}{r}");
    }
    let path_start = origin.len();
    let base_path = base.get(path_start..).unwrap_or("/").split(['?', '#']).next().unwrap_or("/");
    let mut parts: Vec<&str> = base_path.split('/').collect();
    parts.pop();
    for piece in r.split('/') {
        match piece {
            "." | "" => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            other => parts.push(other),
        }
    }
    let mut path = parts.join("/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    format!("{origin}{path}")
}

fn origins(v: &Value, key: &str) -> Result<Vec<String>> {
    let list = v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    if list.len() > 16 {
        return Err(Error::msg(format!("networkOrigins.{key} lists more than 16 origins.")));
    }
    let mut out = Vec::new();
    for item in list {
        let s = item.as_str().unwrap_or("").trim().trim_end_matches('/').to_ascii_lowercase();
        check_url(&s)?;
        if origin_of(&s).as_deref() != Some(s.as_str()) {
            return Err(Error::msg(format!("{s} is not a bare origin (no path or query).")));
        }
        out.push(s);
    }
    Ok(out)
}

/// Validate a descriptor fetched from `url`.
pub fn parse_descriptor(url: &str, body: &str) -> Result<Descriptor> {
    check_url(url)?;
    let v: Value = serde_json::from_str(body).map_err(|e| Error::msg(format!("Not JSON: {e}")))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    if s("format") != FORMAT || s("protocolVersion") != PROTOCOL {
        return Err(Error::msg("Not a mod source: format must be dmm-source 1.0."));
    }
    if s("gameId") != "deadlock" {
        return Err(Error::msg("That source is not for Deadlock."));
    }
    let source_id = s("sourceId");
    if source_id.is_empty() || source_id.len() > 256 || source_id.chars().any(char::is_control) {
        return Err(Error::msg("The source has no valid sourceId."));
    }
    let name: String = s("name").chars().take(120).collect();
    let net = v.get("networkOrigins").cloned().unwrap_or(Value::Null);
    let api = origins(&net, "api")?;
    let downloads = origins(&net, "downloads")?;
    let media = origins(&net, "media")?;
    if api.is_empty() || downloads.is_empty() {
        return Err(Error::msg("The source must declare api and downloads origins."));
    }
    let cat = v.get("catalog").cloned().unwrap_or(Value::Null);
    let kind = cat.get("kind").and_then(Value::as_str).unwrap_or("");
    let catalog = match kind {
        "static" => Catalog::Static {
            index_url: resolve(url, cat.get("indexUrl").and_then(Value::as_str).unwrap_or("")),
        },
        "api" => Catalog::Api {
            list_url: resolve(url, cat.get("listUrl").and_then(Value::as_str).unwrap_or("")),
        },
        _ => return Err(Error::msg("The catalog kind must be static or api.")),
    };
    let catalog_url = match &catalog {
        Catalog::Static { index_url } => index_url,
        Catalog::Api { list_url } => list_url,
    };
    let origin = origin_of(url).unwrap_or_default();
    check_on(catalog_url, &api, "catalog")?;
    Ok(Descriptor {
        url: url.trim().to_string(),
        name: if name.is_empty() { source_id.clone() } else { name },
        source_id,
        catalog,
        api,
        downloads,
        media,
        homepage: v.get("homepageUrl").and_then(Value::as_str).map(str::to_string),
        origin,
        release_history: v
            .get("capabilities")
            .and_then(|c| c.get("releaseHistory"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        history_template: cat
            .get("releaseHistoryUrlTemplate")
            .and_then(Value::as_str)
            .map(|t| resolve(url, t)),
    })
}

/// `url` must be https on one of `allowed`.
pub fn check_on(url: &str, allowed: &[String], role: &str) -> Result<()> {
    check_url(url)?;
    let o = origin_of(url).unwrap_or_default();
    if allowed.iter().any(|a| *a == o) {
        Ok(())
    } else {
        Err(Error::msg(format!("{url} is not on the source's declared {role} origins.")))
    }
}

fn parse_artifact(base: &str, v: &Value) -> Option<Artifact> {
    let d = v.get("download")?;
    Some(Artifact {
        id: v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        filename: v.get("filename").and_then(Value::as_str).unwrap_or("").to_string(),
        url: d.get("url").and_then(Value::as_str).map(|u| resolve(base, u)),
        resolver_url: d.get("resolverUrl").and_then(Value::as_str).map(|u| resolve(base, u)),
        size: v.get("size").and_then(Value::as_u64),
        sha256: v
            .get("hashes")
            .and_then(|h| h.get("sha256"))
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
    })
}

/// One listing from a catalog item or a detail document at `base`.
pub fn parse_listing(base: &str, v: &Value) -> Option<Listing> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let release = v.get("release");
    let images: Vec<String> = v
        .get("images")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|i| i.as_str().or_else(|| i.get("url").and_then(Value::as_str)))
                .take(20)
                .map(|u| resolve(base, u))
                .collect()
        })
        .unwrap_or_default();
    Some(Listing {
        images,
        markdown: v.get("descriptionFormat").and_then(Value::as_str) == Some("markdown"),
        homepage: v.get("homepageUrl").and_then(Value::as_str).map(|u| resolve(base, u)),
        release_id: release
            .and_then(|r| r.get("id").and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
        history_url: v.get("releaseHistoryUrl").and_then(Value::as_str).map(|u| resolve(base, u)),
        title: s("title"),
        author: s("author"),
        description: s("description").chars().take(4000).collect(),
        nsfw: v.get("isNsfw").and_then(Value::as_bool),
        tags: v
            .get("tags")
            .and_then(Value::as_array)
            .map(|t| t.iter().filter_map(Value::as_str).take(100).map(str::to_string).collect())
            .unwrap_or_default(),
        image: v
            .get("images")
            .and_then(Value::as_array)
            .and_then(|i| i.first())
            .and_then(|i| i.as_str().or_else(|| i.get("url").and_then(Value::as_str)))
            .map(|u| resolve(base, u)),
        updated_at: s("updatedAt"),
        version: release
            .and_then(|r| r.get("version").and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
        artifacts: release
            .and_then(|r| r.get("artifacts"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| parse_artifact(base, x)).take(100).collect())
            .unwrap_or_default(),
        detail_url: v.get("detailUrl").and_then(Value::as_str).map(|u| resolve(base, u)),
        removed: v.get("availability").and_then(Value::as_str) == Some("removed"),
        id,
    })
}

/// A catalog page: its listings and the next page's URL when there is one.
pub fn parse_page(base: &str, body: &str) -> Result<(Vec<Listing>, Option<String>)> {
    let v: Value = serde_json::from_str(body).map_err(|e| Error::msg(format!("Not JSON: {e}")))?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The catalog has no items list."))?;
    let listings = items.iter().filter_map(|i| parse_listing(base, i)).collect();
    let next = v
        .get("nextCursor")
        .or_else(|| v.get("next"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|n| {
            if n.contains('/') || n.contains("://") {
                resolve(base, n)
            } else {
                let sep = if base.contains('?') { '&' } else { '?' };
                format!("{base}{sep}cursor={n}")
            }
        });
    Ok((listings, next))
}

fn get_json(url: &str) -> Result<String> {
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", "Sigil")
        .header("Accept", "application/json")
        .call()
        .map_err(|e| Error::Network(format!("{url}: {e}")))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_JSON_BYTES)
        .read_to_string()
        .map_err(|e| Error::Network(format!("{url}: {e}")))
}

/// Fetch and validate a descriptor.
pub fn fetch_descriptor(url: &str) -> Result<Descriptor> {
    check_url(url)?;
    parse_descriptor(url, &get_json(url.trim())?)
}

/// Fetch every page of the catalog, then the detail document of each listing that has no
/// release inline. Removed listings are dropped.
pub fn fetch_catalog(d: &Descriptor) -> Result<Vec<Listing>> {
    let mut next = Some(match &d.catalog {
        Catalog::Static { index_url } => index_url.clone(),
        Catalog::Api { list_url } => list_url.clone(),
    });
    let mut out: Vec<Listing> = Vec::new();
    let mut pages = 0;
    while let Some(url) = next.take() {
        pages += 1;
        if pages > MAX_PAGES {
            return Err(Error::msg("The catalog has more than 100 pages."));
        }
        check_on(&url, &d.api, "api")?;
        let (listings, after) = parse_page(&url, &get_json(&url)?)?;
        out.extend(listings);
        if out.len() > MAX_LISTINGS {
            return Err(Error::msg("The catalog has more than 10,000 mods."));
        }
        next = after;
    }
    let mut documents = 0;
    for listing in out.iter_mut() {
        if !listing.artifacts.is_empty() {
            continue;
        }
        let Some(detail) = listing.detail_url.clone() else { continue };
        documents += 1;
        if documents > MAX_DOCUMENTS || check_on(&detail, &d.api, "api").is_err() {
            continue;
        }
        if let Ok(body) = get_json(&detail)
            && let Ok(v) = serde_json::from_str::<Value>(&body)
            && let Some(full) = parse_listing(&detail, &v)
        {
            *listing = Listing {
                detail_url: listing.detail_url.clone(),
                ..full
            };
        }
    }
    out.retain(|l| !l.removed);
    Ok(out)
}

/// Where a listing's release history lives: its own `releaseHistoryUrl`, else the catalog's
/// template with the id filled in. `None` when the source does not declare the capability.
pub fn history_url(d: &Descriptor, listing: &Listing) -> Option<String> {
    if !d.release_history {
        return None;
    }
    listing.history_url.clone().or_else(|| {
        let id: String = listing
            .id
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        d.history_template.as_ref().map(|t| t.replace("{listingId}", &id))
    })
}

/// One page of release history: `{ items, nextCursor }`. A cursor becomes `?cursor=` on the
/// first page's address, as for catalog pages.
pub fn parse_history_page(base: &str, body: &str) -> Result<(Vec<Release>, Option<String>)> {
    let v: Value = serde_json::from_str(body).map_err(|e| Error::msg(format!("Not JSON: {e}")))?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The release history has no items list."))?;
    let s = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let releases = items
        .iter()
        .take(200)
        .map(|r| Release {
            id: s(r, "id"),
            version: s(r, "version"),
            published_at: s(r, "publishedAt"),
            changelog: s(r, "changelog").chars().take(4000).collect(),
        })
        .collect();
    let next = v.get("nextCursor").and_then(Value::as_str).filter(|n| !n.is_empty()).map(|n| {
        if n.contains('/') || n.contains("://") {
            resolve(base, n)
        } else {
            let sep = if base.contains('?') { '&' } else { '?' };
            format!("{base}{sep}cursor={n}&limit=100")
        }
    });
    Ok((releases, next))
}

/// Fetch a listing's release history, newest first as the source orders it. At most 20 pages,
/// every one on the source's api origins.
pub fn fetch_history(d: &Descriptor, listing: &Listing) -> Result<Vec<Release>> {
    let first = history_url(d, listing).ok_or_else(|| Error::msg("This source has no release history."))?;
    let mut next = Some(first.clone());
    let mut out = Vec::new();
    let mut pages = 0;
    while let Some(url) = next.take() {
        pages += 1;
        if pages > 20 {
            break;
        }
        check_on(&url, &d.api, "api")?;
        let (releases, after) = parse_history_page(&first, &get_json(&url)?)?;
        out.extend(releases);
        next = after;
    }
    Ok(out)
}

/// An image a source names may load only from its declared media or api origins.
pub fn image_allowed(d: &Descriptor, url: &str) -> bool {
    check_on(url, &d.media, "media").is_ok() || check_on(url, &d.api, "api").is_ok()
}

/// Markdown as plain text for a reader without a Markdown view: headings, emphasis, code
/// marks and link targets are dropped, list items become bullets, images are left out (DMM
/// drops inline images too), table rows keep their cells.
pub fn markdown_to_text(markdown: &str) -> String {
    static LINK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap());
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for raw in markdown.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push(line.to_string());
            continue;
        }
        // Table rules (`|---|:--:|`) and horizontal rules carry no words.
        if !trimmed.is_empty() && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '*' | '_')) && trimmed.len() >= 3 {
            continue;
        }
        let mut text = trimmed.trim_start_matches('#').trim_start().to_string();
        if trimmed.starts_with('>') {
            text = trimmed.trim_start_matches('>').trim_start().to_string();
        }
        for marker in ["- [ ] ", "- [x] ", "* [ ] ", "* [x] ", "- ", "* ", "+ "] {
            if let Some(rest) = text.strip_prefix(marker) {
                text = format!("• {rest}");
                break;
            }
        }
        text = LINK
            .replace_all(&text, |caps: &regex::Captures| {
                if caps[0].starts_with('!') { String::new() } else { caps[1].to_string() }
            })
            .into_owned();
        if text.starts_with('|') {
            text = text.trim_matches('|').split('|').map(str::trim).collect::<Vec<_>>().join("  ·  ");
        }
        for mark in ["**", "__", "~~", "`"] {
            text = text.replace(mark, "");
        }
        out.push(text.trim_end().to_string());
    }
    let joined = out.join("\n");
    let mut text = String::with_capacity(joined.len());
    let mut blank = 0;
    for line in joined.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        text.push_str(line);
        text.push('\n');
    }
    text.trim().to_string()
}

/// Case-insensitive search over title, author and tags.
pub fn search<'a>(listings: &'a [Listing], query: &str) -> Vec<&'a Listing> {
    let q = query.trim().to_lowercase();
    listings
        .iter()
        .filter(|l| {
            q.is_empty()
                || l.title.to_lowercase().contains(&q)
                || l.author.to_lowercase().contains(&q)
                || l.tags.iter().any(|t| t.to_lowercase().contains(&q))
        })
        .collect()
}

/// Download one artifact into `dir`, checking every hop stays on the declared download
/// origins and that a declared sha256 matches. Returns the archive on disk.
pub fn download_artifact(d: &Descriptor, a: &Artifact, dir: &Path) -> Result<PathBuf> {
    let url = match (&a.url, &a.resolver_url) {
        (Some(url), _) => url.clone(),
        (None, Some(resolver)) => {
            check_on(resolver, &d.api, "api")?;
            let v: Value = serde_json::from_str(&get_json(resolver)?)
                .map_err(|e| Error::msg(format!("The resolver answered something else: {e}")))?;
            v.get("url").and_then(Value::as_str).map(|u| resolve(resolver, u)).ok_or_else(|| Error::msg("The resolver gave no url."))?
        }
        _ => return Err(Error::msg("That file has no download address.")),
    };
    check_on(&url, &d.downloads, "downloads")?;
    let mut policy = crate::install::DownloadPolicy {
        allowed_origins: Vec::new(),
        allowed_domains: Vec::new(),
        ..crate::install::DownloadPolicy::gamebanana()
    };
    for origin in &d.downloads {
        policy = policy.with_mirror(origin)?;
    }
    let report = crate::install::download(&url, dir, &policy)?;
    if let Some(expected) = &a.sha256
        && !expected.eq_ignore_ascii_case(&report.sha256)
    {
        let _ = std::fs::remove_file(&report.path);
        return Err(Error::msg("The file's sha256 does not match what the source says. Nothing was installed."));
    }
    Ok(report.path)
}

/// A stable key for a source: its origin and id.
pub fn source_key(d: &Descriptor) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{}\0{}", d.origin, d.source_id).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTOR: &str = r#"{
      "format": "dmm-source", "protocolVersion": "1.0", "sourceId": "my-community",
      "name": "My Community Mods", "gameId": "deadlock",
      "catalog": { "kind": "static", "indexUrl": "./catalog.json" },
      "networkOrigins": { "api": ["https://mods.example.org"], "downloads": ["https://cdn.example.org"], "media": [] }
    }"#;

    #[test]
    fn descriptors_resolve_and_check_origins() {
        let d = parse_descriptor("https://mods.example.org/src/dmm-source.json", DESCRIPTOR).unwrap();
        assert_eq!(d.name, "My Community Mods");
        assert_eq!(
            d.catalog,
            Catalog::Static {
                index_url: "https://mods.example.org/src/catalog.json".into()
            }
        );
        assert_eq!(d.origin, "https://mods.example.org");
        assert_eq!(source_key(&d).len(), 16);
        let bad = DESCRIPTOR.replace("\"deadlock\"", "\"tf2\"");
        assert!(parse_descriptor("https://mods.example.org/x.json", &bad).is_err());
        let off = DESCRIPTOR.replace("./catalog.json", "https://evil.example.com/c.json");
        assert!(parse_descriptor("https://mods.example.org/x.json", &off).is_err());
    }

    #[test]
    fn only_public_https_is_allowed() {
        assert!(check_url("https://mods.example.org/a.json").is_ok());
        assert!(check_url("http://mods.example.org/a.json").is_err());
        assert!(check_url("https://192.168.1.2/a.json").is_err());
        assert!(check_url("https://localhost/a.json").is_err());
        assert!(check_url("https://me:pw@mods.example.org/").is_err());
    }

    #[test]
    fn relative_references_resolve() {
        let b = "https://h.org/a/b/c.json";
        assert_eq!(resolve(b, "./d.json"), "https://h.org/a/b/d.json");
        assert_eq!(resolve(b, "../files/x.zip"), "https://h.org/a/files/x.zip");
        assert_eq!(resolve(b, "/root.json"), "https://h.org/root.json");
        assert_eq!(resolve(b, "https://o.org/z"), "https://o.org/z");
    }

    #[test]
    fn release_history_follows_the_listing_then_the_template() {
        let with = DESCRIPTOR
            .replace(r#""indexUrl": "./catalog.json""#, r#""indexUrl": "./catalog.json", "releaseHistoryUrlTemplate": "./history/{listingId}""#)
            .replace(r#""gameId": "deadlock","#, r#""gameId": "deadlock", "capabilities": {"releaseHistory": true},"#);
        let d = parse_descriptor("https://mods.example.org/src/dmm-source.json", &with).unwrap();
        assert!(d.release_history);
        let listing = Listing { id: "blue haze".into(), ..Listing::default() };
        assert_eq!(history_url(&d, &listing).as_deref(), Some("https://mods.example.org/src/history/blue%20haze"));
        let own = Listing { history_url: Some("https://mods.example.org/h/a.json".into()), ..listing.clone() };
        assert_eq!(history_url(&d, &own).as_deref(), Some("https://mods.example.org/h/a.json"));
        // Without the capability there is no history, whatever the template says.
        let plain = parse_descriptor("https://mods.example.org/src/x.json", DESCRIPTOR).unwrap();
        assert_eq!(history_url(&plain, &own), None);
        let (releases, next) = parse_history_page(
            "https://mods.example.org/src/history/a",
            r#"{"items":[{"id":"r2","version":"2.0","publishedAt":"2026-02-01T00:00:00Z"},
                         {"id":"r1","version":"1.0","changelog":"Initial release"}],"nextCursor":"older"}"#,
        )
        .unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[1].changelog, "Initial release");
        assert_eq!(next.as_deref(), Some("https://mods.example.org/src/history/a?cursor=older&limit=100"));
        let (_, end) = parse_history_page("https://h.org/x", r#"{"items":[],"nextCursor":null}"#).unwrap();
        assert_eq!(end, None);
    }

    #[test]
    fn markdown_reads_as_plain_text() {
        let md = "# Blue Haze\n\nA **bold** recolor with `code`.\n\n- one\n- [x] two\n\n![shot](a.png)\nSee [the page](https://x.org).\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```\nraw *text*\n```";
        assert_eq!(
            markdown_to_text(md),
            "Blue Haze\n\nA bold recolor with code.\n\n• one\n• two\n\n\nSee the page.\n\na  ·  b\n1  ·  2\n\nraw *text*"
                .replace("\n\n\n", "\n\n")
        );
    }

    #[test]
    fn listing_images_markdown_and_release_id() {
        let v: Value = serde_json::from_str(
            r#"{"id":"a","title":"T","descriptionFormat":"markdown","images":["./a.png",{"url":"https://cdn.example.org/b.png"}],
                "homepageUrl":"https://mods.example.org/a","release":{"id":"r1","version":"1"}}"#,
        )
        .unwrap();
        let l = parse_listing("https://mods.example.org/src/catalog.json", &v).unwrap();
        assert!(l.markdown);
        assert_eq!(l.images, ["https://mods.example.org/src/a.png", "https://cdn.example.org/b.png"]);
        assert_eq!(l.image.as_deref(), Some("https://mods.example.org/src/a.png"));
        assert_eq!(l.release_id, "r1");
        let d = parse_descriptor("https://mods.example.org/src/x.json", DESCRIPTOR).unwrap();
        assert!(image_allowed(&d, &l.images[0]));
        assert!(!image_allowed(&d, "https://evil.example.com/x.png"));
    }

    #[test]
    fn catalog_pages_and_listings() {
        let body = r#"{"snapshotId":"s","items":[
          {"id":"a","title":"Blue Haze","author":"Kalo","isNsfw":false,"tags":["haze"],
           "release":{"id":"r","version":"1.2","artifacts":[{"id":"z","filename":"a.zip","download":{"url":"../files/a.zip"},"hashes":{"sha256":"AB"}}]}},
          {"id":"b","title":"Gone","availability":"removed"},
          {"id":"c","title":"Detail only","detailUrl":"./mods/c.json"}
        ],"nextCursor":"p2"}"#;
        let (items, next) = parse_page("https://h.org/src/catalog.json", body).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].artifacts[0].url.as_deref(), Some("https://h.org/files/a.zip"));
        assert_eq!(items[0].artifacts[0].sha256.as_deref(), Some("ab"));
        assert_eq!(items[0].version, "1.2");
        assert!(items[1].removed);
        assert_eq!(items[2].detail_url.as_deref(), Some("https://h.org/src/mods/c.json"));
        assert_eq!(next.as_deref(), Some("https://h.org/src/catalog.json?cursor=p2"));
        assert_eq!(search(&items, "haze").len(), 1);
        assert_eq!(search(&items, "kalo").len(), 1);
    }
}
