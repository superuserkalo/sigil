//! The relay region picker (Servers > Regions): Valve's relay locations for Deadlock, how far each
//! one is, and Windows Firewall rules that keep Deadlock away from the ones the player ticks.
//!
//! Deadlock reaches its match servers through Valve's relay network (Steam Datagram Relay). The
//! client pings the relays and matchmaking places the player where those pings say. A relay
//! Deadlock cannot reach looks far away, so blocking every relay of a location keeps matchmaking
//! out of it. This module is the read side and the plan; [`firewall`] reads, writes and verifies
//! the rules; [`probe`] measures latency.
//!
//! Off by default. Nothing here runs on its own: the catalog is fetched when the Regions tab
//! opens, pings run on a press, and rules change only on the player's Apply or Reset, behind
//! Windows' admin prompt. No agent tool reaches any of it (the `approve` flag in [`firewall`]).
//!
//! Network: one anonymous GET to [`CATALOG_URL`] on `api.steampowered.com` (already on the
//! allowlist for the patch feed), cached for a day; ICMP echo to the relay addresses it lists
//! when the player presses Measure ping. Studied from Deadlock Mod Manager's Server Picker
//! (GPL-3.0, behaviour only); the code and words here are Sigil's.

pub mod firewall;
pub mod probe;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// Valve's relay configuration for Deadlock (app 1422450): every point of presence with its
/// relay addresses and UDP port ranges. Public, no key.
pub const CATALOG_URL: &str = "https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=1422450";
/// A cached catalog younger than this is used without asking Valve again.
pub const CATALOG_MAX_AGE_SECONDS: i64 = 24 * 3600;
/// The cache file under the folder the caller passes (Sigil's data folder).
pub const CACHE_FILE: &str = "relay-catalog.json";
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// The answer is about 20 KB today; anything past this is not the catalog.
const MAX_CATALOG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOCATIONS: usize = 512;
const MAX_RELAYS: usize = 128;
const USER_AGENT: &str = "Sigil/0.1 (Deadlock mod manager)";
/// Saved presets at most.
pub const MAX_PRESETS: usize = 50;

/// Where a location is, for grouping. Derived from the location's description, which names a
/// country or a US state; Valve's coordinates are a fallback because a few are swapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Region {
    NorthAmerica,
    SouthAmerica,
    Europe,
    MiddleEast,
    Africa,
    Asia,
    Oceania,
    Other,
}

impl Region {
    pub const ALL: [Region; 8] = [
        Region::NorthAmerica,
        Region::SouthAmerica,
        Region::Europe,
        Region::MiddleEast,
        Region::Africa,
        Region::Asia,
        Region::Oceania,
        Region::Other,
    ];

    /// The English name; the app translates it.
    pub fn label(self) -> &'static str {
        match self {
            Region::NorthAmerica => "North America",
            Region::SouthAmerica => "South America",
            Region::Europe => "Europe",
            Region::MiddleEast => "Middle East",
            Region::Africa => "Africa",
            Region::Asia => "Asia",
            Region::Oceania => "Oceania",
            Region::Other => "Other locations",
        }
    }
}

/// One relay: an address and the UDP ports Deadlock talks to it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relay {
    pub address: Ipv4Addr,
    pub port_start: u16,
    pub port_end: u16,
}

/// A relay location (Valve calls it a point of presence): `fra`, Frankfurt (Germany).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// Valve's short id, `fra`, `sto2`. Lowercase letters and digits.
    pub id: String,
    /// Valve's description as given: `Frankfurt (Germany)`.
    pub description: String,
    /// The part before the brackets: `Frankfurt`.
    pub city: String,
    /// The part in brackets, a country or US state: `Germany`. None when there is none.
    pub country: Option<String>,
    pub region: Region,
    pub relays: Vec<Relay>,
}

/// Every location Deadlock can be routed through, from [`CATALOG_URL`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// Valve's revision number for the configuration.
    pub revision: u64,
    /// Unix seconds when Sigil fetched it.
    pub fetched_at: i64,
    /// Sorted by id.
    pub locations: Vec<Location>,
}

impl Catalog {
    pub fn location(&self, id: &str) -> Option<&Location> {
        self.locations.iter().find(|l| l.id == id)
    }

    /// The locations by region, in [`Region::ALL`] order, each sorted by city then id. Empty
    /// regions are left out.
    pub fn by_region(&self) -> Vec<(Region, Vec<&Location>)> {
        let mut groups: BTreeMap<Region, Vec<&Location>> = BTreeMap::new();
        for location in &self.locations {
            groups.entry(location.region).or_default().push(location);
        }
        Region::ALL
            .into_iter()
            .filter_map(|region| {
                let mut list = groups.remove(&region)?;
                list.sort_by(|a, b| a.city.cmp(&b.city).then_with(|| a.id.cmp(&b.id)));
                Some((region, list))
            })
            .collect()
    }

    /// Every location id, sorted.
    pub fn ids(&self) -> Vec<String> {
        self.locations.iter().map(|l| l.id.clone()).collect()
    }
}

/// Valve's ids: 1 to 16 lowercase ASCII letters and digits. Also what a rule name may carry,
/// so nothing else can ride into a `netsh` line.
pub fn valid_location_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 16 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// A routable public address. Private, loopback, link-local, carrier NAT, multicast and
/// reserved ranges are refused: blocking them could cut the player's own network.
pub fn public_address(address: Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
        || address.is_multicast()
        || address.is_documentation()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b)))
}

/// Read Valve's answer. Locations without a public relay are left out: there is nothing to
/// block or ping. Anything malformed fails the whole read rather than half a catalog.
pub fn parse_catalog(bytes: &[u8], fetched_at: i64) -> Result<Catalog> {
    let root: Value = serde_json::from_slice(bytes)?;
    // GetSDRConfig answers at the top level; a `response` wrapper is accepted too.
    let root = root.get("response").filter(|r| r.get("pops").is_some()).unwrap_or(&root);
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(Error::msg("Valve's relay list said it was not successful."));
    }
    let revision = root.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let pops = root
        .get("pops")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::msg("Valve's relay list has no locations."))?;
    if pops.len() > MAX_LOCATIONS {
        return Err(Error::msg("Valve's relay list has too many locations."));
    }
    let mut locations = Vec::new();
    for (id, pop) in pops {
        if !valid_location_id(id) {
            return Err(Error::msg(format!("Valve's relay list has an unexpected id: {id:?}")));
        }
        let relays = pop.get("relays").and_then(Value::as_array).cloned().unwrap_or_default();
        if relays.len() > MAX_RELAYS {
            return Err(Error::msg(format!("Relay location {id} lists too many relays.")));
        }
        let mut parsed = BTreeSet::new();
        for relay in &relays {
            let address: Ipv4Addr = relay
                .get("ipv4")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| Error::msg(format!("Relay location {id} has a relay without an address.")))?;
            let ports = relay.get("port_range").and_then(Value::as_array);
            let port = |i: usize| {
                ports
                    .and_then(|p| p.get(i))
                    .and_then(Value::as_u64)
                    .and_then(|n| u16::try_from(n).ok())
            };
            let (Some(port_start), Some(port_end)) = (port(0), port(1)) else {
                return Err(Error::msg(format!("Relay location {id} has a relay without ports.")));
            };
            if !public_address(address) || port_start == 0 || port_start > port_end {
                return Err(Error::msg(format!("Relay location {id} lists a relay Sigil will not block: {address}.")));
            }
            parsed.insert(Relay { address, port_start, port_end });
        }
        if parsed.is_empty() {
            continue;
        }
        let description = pop
            .get("desc")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|d| !d.is_empty() && d.len() <= 200 && !d.chars().any(char::is_control))
            .unwrap_or(id)
            .to_string();
        let (city, country) = split_description(&description);
        let geo = pop.get("geo").and_then(Value::as_array).and_then(|g| {
            Some((g.first()?.as_f64()?, g.get(1)?.as_f64()?))
        });
        let region = region_for(&description, country.as_deref(), geo);
        locations.push(Location {
            id: id.clone(),
            description,
            city,
            country,
            region,
            relays: parsed.into_iter().collect(),
        });
    }
    if locations.is_empty() {
        return Err(Error::msg("Valve's relay list has no relays."));
    }
    locations.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Catalog { revision, fetched_at, locations })
}

/// `Frankfurt (Germany)` into `Frankfurt` and `Germany`. `Hong Kong` has no brackets.
fn split_description(description: &str) -> (String, Option<String>) {
    if let Some(open) = description.rfind(" (")
        && description.ends_with(')')
    {
        let city = description[..open].trim().to_string();
        let country = description[open + 2..description.len() - 1].trim().to_string();
        if !city.is_empty() && !country.is_empty() {
            return (city, Some(country));
        }
    }
    (description.to_string(), None)
}

/// Countries and US states by region, as Valve writes them in descriptions. Sigil's own list.
const PLACES: &[(Region, &[&str])] = &[
    (
        Region::NorthAmerica,
        &[
            "united states", "usa", "us", "canada", "mexico", "alabama", "alaska", "arizona", "arkansas",
            "california", "colorado", "connecticut", "delaware", "florida", "georgia", "hawaii", "idaho",
            "illinois", "indiana", "iowa", "kansas", "kentucky", "louisiana", "maine", "maryland",
            "massachusetts", "michigan", "minnesota", "mississippi", "missouri", "montana", "nebraska",
            "nevada", "new hampshire", "new jersey", "new mexico", "new york", "north carolina",
            "north dakota", "ohio", "oklahoma", "oregon", "pennsylvania", "rhode island",
            "south carolina", "south dakota", "tennessee", "texas", "utah", "vermont", "virginia",
            "washington", "west virginia", "wisconsin", "wyoming", "district of columbia", "ontario",
            "quebec", "british columbia", "alberta",
        ],
    ),
    (
        Region::SouthAmerica,
        &[
            "brazil", "argentina", "chile", "peru", "colombia", "venezuela", "ecuador", "bolivia",
            "uruguay", "paraguay",
        ],
    ),
    (
        Region::Europe,
        &[
            "netherlands", "germany", "england", "scotland", "wales", "united kingdom", "uk", "ireland",
            "france", "spain", "portugal", "italy", "sweden", "norway", "denmark", "finland", "iceland",
            "austria", "switzerland", "poland", "czechia", "czech republic", "slovakia", "hungary",
            "romania", "bulgaria", "greece", "belgium", "luxembourg", "lithuania", "latvia", "estonia",
            "ukraine", "russia", "turkey", "serbia", "croatia", "slovenia", "moldova",
        ],
    ),
    (
        Region::MiddleEast,
        &[
            "united arab emirates", "uae", "saudi arabia", "qatar", "bahrain", "kuwait", "oman",
            "israel", "jordan", "iraq", "iran",
        ],
    ),
    (
        Region::Africa,
        &["south africa", "egypt", "nigeria", "kenya", "morocco", "ghana", "tunisia"],
    ),
    (
        Region::Asia,
        &[
            "japan", "south korea", "korea", "india", "china", "hong kong", "singapore", "taiwan",
            "philippines", "thailand", "malaysia", "indonesia", "vietnam", "pakistan", "bangladesh",
            "kazakhstan",
        ],
    ),
    (Region::Oceania, &["australia", "new zealand", "guam"]),
];

fn place_region(name: &str) -> Option<Region> {
    let name = name.trim().to_ascii_lowercase();
    PLACES
        .iter()
        .find(|(_, names)| names.contains(&name.as_str()))
        .map(|(region, _)| *region)
}

/// The region for a location: the bracketed country or state, then the whole description
/// (`Hong Kong`, `Singapore`, `Guam`), then Valve's `[longitude, latitude]` as a last resort.
pub fn region_for(description: &str, country: Option<&str>, geo: Option<(f64, f64)>) -> Region {
    if let Some(region) = country.and_then(place_region) {
        return region;
    }
    if let Some(region) = place_region(description) {
        return region;
    }
    let Some((lon, lat)) = geo.filter(|(lon, lat)| lon.abs() <= 180.0 && lat.abs() <= 90.0) else {
        return Region::Other;
    };
    match (lon, lat) {
        (lon, lat) if lon < -30.0 && lat >= 12.0 => Region::NorthAmerica,
        (lon, _) if lon < -30.0 => Region::SouthAmerica,
        (lon, lat) if lon >= 110.0 && lat < -10.0 => Region::Oceania,
        (lon, lat) if (-30.0..60.0).contains(&lon) && lat >= 36.0 => Region::Europe,
        (lon, lat) if (34.0..60.0).contains(&lon) && (12.0..36.0).contains(&lat) => Region::MiddleEast,
        (lon, lat) if (-20.0..52.0).contains(&lon) && lat < 36.0 => Region::Africa,
        (lon, _) if lon >= 60.0 => Region::Asia,
        _ => Region::Other,
    }
}

// ---------------------------------------------------------------------------------------------
// Fetch and cache
// ---------------------------------------------------------------------------------------------

/// Ask Valve for the catalog: one GET, 15 seconds at most, 2 MB at most. Goes through the
/// offline switch and the proxy settings like every other request.
pub fn fetch_catalog(now: i64) -> Result<Catalog> {
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(FETCH_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(CATALOG_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| Error::Network(format!("Valve's relay list could not be loaded: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("Valve's relay list answered HTTP {status}.")));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_CATALOG_BYTES)
        .read_to_vec()
        .map_err(|e| Error::Network(format!("Valve's relay list could not be read: {e}")))?;
    parse_catalog(&bytes, now)
}

pub fn cache_path(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE)
}

/// The cached catalog, when there is one and it still passes the checks.
pub fn read_cache(dir: &Path) -> Option<Catalog> {
    let bytes = fs::read(cache_path(dir)).ok()?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return None;
    }
    let catalog: Catalog = serde_json::from_slice(&bytes).ok()?;
    let valid = !catalog.locations.is_empty()
        && catalog.locations.len() <= MAX_LOCATIONS
        && catalog.locations.iter().all(|l| {
            valid_location_id(&l.id)
                && !l.relays.is_empty()
                && l.relays.len() <= MAX_RELAYS
                && l.relays
                    .iter()
                    .all(|r| public_address(r.address) && r.port_start > 0 && r.port_start <= r.port_end)
        });
    valid.then_some(catalog)
}

pub fn write_cache(dir: &Path, catalog: &Catalog) -> Result<()> {
    crate::util::write_atomic(&cache_path(dir), &serde_json::to_vec_pretty(catalog)?)
}

/// Where the catalog came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogSource {
    /// Fetched from Valve just now.
    Fresh,
    /// The cache, younger than a day.
    Cached,
    /// Valve could not be reached; the older cache stands in. The reason.
    Stale(String),
}

/// The catalog: the cache while it is younger than a day (unless `force`), otherwise Valve,
/// falling back to an older cache when Valve cannot be reached. `dir` holds the cache.
pub fn load_catalog(dir: &Path, force: bool, now: i64) -> Result<(Catalog, CatalogSource)> {
    let cached = read_cache(dir);
    if !force
        && let Some(catalog) = &cached
        && now - catalog.fetched_at < CATALOG_MAX_AGE_SECONDS
        && now >= catalog.fetched_at
    {
        return Ok((catalog.clone(), CatalogSource::Cached));
    }
    match fetch_catalog(now) {
        Ok(catalog) => {
            let _ = write_cache(dir, &catalog);
            Ok((catalog, CatalogSource::Fresh))
        }
        Err(error) => match cached {
            Some(catalog) => Ok((catalog, CatalogSource::Stale(error.to_string()))),
            None => Err(error),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// The player's choices (kept in the app's options)
// ---------------------------------------------------------------------------------------------

/// A named set of blocked locations: "EU only", "Friends in NA".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub blocked: Vec<String>,
}

/// What the app keeps: the presets, the locations Sigil last saw blocked, and whether a firewall
/// change started and did not report back (the app closed during the admin prompt).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    /// Locations Sigil's rules blocked after the last Apply that was read back. Empty: default
    /// routing. The firewall read back is the truth; this is what the app shows before it reads.
    pub applied: Vec<String>,
    pub presets: Vec<Preset>,
    /// An Apply or Reset began and its result was never read back.
    pub pending: bool,
}

/// 1 to 60 characters, no control characters, counted as characters.
pub fn valid_preset_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= 60 && !name.chars().any(char::is_control)
}

/// Sorted, deduplicated, valid ids only.
pub fn normalize_selection<'a>(ids: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let set: BTreeSet<&String> = ids.into_iter().filter(|id| valid_location_id(id)).collect();
    set.into_iter().cloned().collect()
}

impl Choices {
    /// Drop what does not pass the checks, after reading from disk.
    pub fn sanitized(self) -> Choices {
        let mut presets: Vec<Preset> = Vec::new();
        for preset in self.presets {
            if valid_preset_name(&preset.name)
                && !presets.iter().any(|p| p.name.eq_ignore_ascii_case(preset.name.trim()))
                && presets.len() < MAX_PRESETS
            {
                presets.push(Preset {
                    name: preset.name.trim().to_string(),
                    blocked: normalize_selection(&preset.blocked),
                });
            }
        }
        Choices {
            applied: normalize_selection(&self.applied),
            presets,
            pending: self.pending,
        }
    }

    /// Save `blocked` under `name`, replacing a preset with the same name (any case).
    pub fn save_preset(&mut self, name: &str, blocked: &[String]) -> Result<()> {
        let name = name.trim();
        if !valid_preset_name(name) {
            return Err(Error::msg("A preset name is 1 to 60 characters."));
        }
        let blocked = normalize_selection(blocked);
        if let Some(existing) = self.presets.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            existing.name = name.to_string();
            existing.blocked = blocked;
            return Ok(());
        }
        if self.presets.len() >= MAX_PRESETS {
            return Err(Error::msg("Sigil keeps at most 50 presets. Delete one first."));
        }
        self.presets.push(Preset { name: name.to_string(), blocked });
        Ok(())
    }

    pub fn delete_preset(&mut self, name: &str) {
        self.presets.retain(|p| p.name != name);
    }
}

// ---------------------------------------------------------------------------------------------
// The rule plan and what is installed
// ---------------------------------------------------------------------------------------------

/// One Windows Firewall rule Sigil adds: outbound UDP from Deadlock to every relay of one
/// location that shares a port range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedRule {
    pub name: String,
    pub location: String,
    pub addresses: Vec<Ipv4Addr>,
    pub port_start: u16,
    pub port_end: u16,
}

/// The rules that block `blocked`: one per location and port range. Fails when a location is
/// not in the catalog, or when every location would be blocked (Deadlock could not connect).
pub fn plan(catalog: &Catalog, blocked: &[String]) -> Result<Vec<PlannedRule>> {
    let blocked = normalize_selection(blocked);
    if !blocked.is_empty() && catalog.locations.iter().all(|l| blocked.contains(&l.id)) {
        return Err(Error::msg("Leave at least one location open, or Deadlock cannot find a match."));
    }
    let mut rules = Vec::new();
    for id in &blocked {
        let location = catalog
            .location(id)
            .ok_or_else(|| Error::msg(format!("Relay location {id} is no longer in Valve's list. Untick it and apply again.")))?;
        let mut by_ports: BTreeMap<(u16, u16), BTreeSet<Ipv4Addr>> = BTreeMap::new();
        for relay in &location.relays {
            by_ports.entry((relay.port_start, relay.port_end)).or_default().insert(relay.address);
        }
        for ((port_start, port_end), addresses) in by_ports {
            rules.push(PlannedRule {
                name: firewall::rule_name(id, port_start, port_end),
                location: id.clone(),
                addresses: addresses.into_iter().collect(),
                port_start,
                port_end,
            });
        }
    }
    Ok(rules)
}

/// A Sigil rule read back from Windows Firewall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledRule {
    pub name: String,
    pub location: String,
    pub port_start: u16,
    pub port_end: u16,
    pub addresses: Vec<Ipv4Addr>,
    pub enabled: bool,
}

/// What Windows Firewall holds right now, as far as Sigil's rules go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    pub rules: Vec<InstalledRule>,
    /// Some(false): the firewall is off for the active network, so no rule does anything.
    pub firewall_on: Option<bool>,
}

/// The installed rules measured against the catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Every relay of these locations is blocked.
    pub blocked: Vec<String>,
    /// Some relays are blocked, others are not (Valve added relays since the Apply, or a rule
    /// was removed by hand). Matchmaking can still use the open ones.
    pub partial: Vec<String>,
    /// Rules for locations the catalog no longer lists.
    pub unknown: Vec<String>,
    /// Locations with a Sigil rule someone turned off.
    pub disabled: Vec<String>,
    /// Sigil rules found, enabled or not.
    pub rules: usize,
}

impl Coverage {
    /// Any Sigil rule is there.
    pub fn any(&self) -> bool {
        self.rules > 0
    }

    /// The rules are exactly the clean result of applying `desired`.
    pub fn matches(&self, desired: &[String]) -> bool {
        self.partial.is_empty()
            && self.unknown.is_empty()
            && self.disabled.is_empty()
            && self.blocked == normalize_selection(desired)
    }
}

/// Which locations the installed rules block, given the catalog. Without a catalog every
/// location with a rule counts as blocked.
pub fn coverage(catalog: Option<&Catalog>, installed: &[InstalledRule]) -> Coverage {
    let mut by_location: BTreeMap<&str, Vec<&InstalledRule>> = BTreeMap::new();
    for rule in installed {
        by_location.entry(rule.location.as_str()).or_default().push(rule);
    }
    let mut out = Coverage { rules: installed.len(), ..Coverage::default() };
    for (id, rules) in by_location {
        if rules.iter().any(|r| !r.enabled) {
            out.disabled.push(id.to_string());
        }
        let Some(catalog) = catalog else {
            out.blocked.push(id.to_string());
            continue;
        };
        let Some(location) = catalog.location(id) else {
            out.unknown.push(id.to_string());
            continue;
        };
        let covered = |relay: &Relay| {
            rules.iter().any(|r| {
                r.enabled
                    && r.addresses.contains(&relay.address)
                    && r.port_start <= relay.port_start
                    && r.port_end >= relay.port_end
            })
        };
        if location.relays.iter().all(covered) {
            out.blocked.push(id.to_string());
        } else {
            out.partial.push(id.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests;
