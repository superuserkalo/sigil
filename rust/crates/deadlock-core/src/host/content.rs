//! Custom maps and addons on the player's own server, laid out the way Deadworks serves content
//! (<https://docs.deadworks.net/guides/content-discovery/>):
//!
//! - Maps: `game/citadel/maps/<name>.vpk`. The current map is always advertised; more go in
//!   `serverbrowser.extra_maps` of `game/bin/win64/configs/deadworks.jsonc`.
//! - Addons: `game/citadel/deadworks_server_addons/vpks/<name>.vpk`, listed in
//!   `serverbrowser.content_addons`.
//! - Joiners download what they lack from `serverbrowser.fastdl_url`, at
//!   `<fastdl_url>/<maps|addons>/<name>_<hash16>.vpk.zst` (zstd, window at most 128 MiB).
//!
//! Everything here writes inside the SERVER's folder only, never the player's game install. Files
//! Sigil replaces or removes go to `sigil-content-backups/<time>/` first. Sigil records what it
//! installed in `sigil-content.json`, and never replaces or removes a file it did not install
//! (the stock maps SteamCMD put there, for one).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::require_approval;
use super::setup::check_server_dir;
use crate::error::{Error, Result};
use crate::servers::{check_server_vpk, file_sha256, parse_fastdl, valid_content_name};
use crate::util::{now_iso, write_atomic};

/// Sigil's record of the content it put into the server folder.
pub const CONTENT_LEDGER: &str = "sigil-content.json";
/// Where replaced and removed files and the previous `deadworks.jsonc` go.
pub const CONTENT_BACKUPS: &str = "sigil-content-backups";
/// The default folder [`build_fastdl`] fills, ready to upload to a download host.
pub const FASTDL_DIR: &str = "sigil-fastdl";
const ZSTD_LEVEL: i32 = 19;

/// A map or a server addon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerContentKind {
    Map,
    Addon,
}

impl ServerContentKind {
    /// The folder name in the download layout (`maps`, `addons`).
    pub fn dir(self) -> &'static str {
        match self {
            Self::Map => "maps",
            Self::Addon => "addons",
        }
    }
}

/// One content VPK in the server folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerContent {
    pub kind: ServerContentKind,
    /// The name players and `map` use, without `.vpk`.
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// First 16 hex digits of the SHA-256, as Deadworks advertises it. Empty when unknown.
    pub hash: String,
    /// Sigil installed it (and may replace or remove it).
    pub ours: bool,
    /// Advertised to joiners (`extra_maps` or `content_addons` lists it).
    pub advertised: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentLedger {
    files: BTreeMap<String, LedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerEntry {
    sha256: String,
    size: u64,
    source: String,
    installed_at: String,
    /// SHA-256 of the file the player picked, when it differs from the installed file (a map
    /// taken out of an addon pak). Matches the install journal's `vpkSha256`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_sha256: Option<String>,
}

/// `<server>/game/citadel/maps`.
pub fn server_maps_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("citadel").join("maps")
}

/// `<server>/game/citadel/deadworks_server_addons/vpks`.
pub fn server_addons_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("citadel").join("deadworks_server_addons").join("vpks")
}

/// `<server>/game/bin/win64/configs/deadworks.jsonc`.
pub fn deadworks_config_path(server_dir: &Path) -> PathBuf {
    server_dir.join("game").join("bin").join("win64").join("configs").join("deadworks.jsonc")
}

fn dir_for(server_dir: &Path, kind: ServerContentKind) -> PathBuf {
    match kind {
        ServerContentKind::Map => server_maps_dir(server_dir),
        ServerContentKind::Addon => server_addons_dir(server_dir),
    }
}

fn key(kind: ServerContentKind, name: &str) -> String {
    format!("{}/{name}", kind.dir())
}

fn read_ledger(server_dir: &Path) -> ContentLedger {
    fs::read_to_string(server_dir.join(CONTENT_LEDGER))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_ledger(server_dir: &Path, ledger: &ContentLedger) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(ledger)?;
    bytes.push(b'\n');
    write_atomic(&server_dir.join(CONTENT_LEDGER), &bytes)
}

/// Map names a VPK carries: `maps/<name>.vmap_c` entries at the top of `maps/`.
pub fn vmap_names(vpk: &Path) -> Result<Vec<String>> {
    let directory = crate::vpk::read_vpk_directory_with(vpk, &crate::vpk::ReadVpkOptions { keep_preload: false })?;
    let mut names: Vec<String> = directory
        .entries
        .iter()
        .filter_map(|e| {
            let lower = e.path.to_ascii_lowercase();
            let name = lower.strip_prefix("maps/")?.strip_suffix(".vmap_c")?;
            (!name.contains('/')).then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Decide what a VPK is and the name it installs under. A map VPK must be named after the map
/// it holds, because `map <name>` loads `maps/<name>.vpk`.
pub fn classify_content(vpk: &Path) -> Result<(ServerContentKind, String)> {
    let label = vpk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = vpk
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".vpk").or_else(|| n.strip_suffix(".VPK")))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| Error::msg(format!("{label} is not a .vpk file.")))?;
    let chunk_of = stem.rsplit_once('_').filter(|(_, n)| n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()));
    let is_chunk = chunk_of.is_some_and(|(base, _)| vpk.with_file_name(format!("{base}_dir.vpk")).is_file());
    if stem.ends_with("_dir") || is_chunk {
        return Err(Error::msg(format!(
            "{label} is part of a multi-file VPK. Server content is one <name>.vpk file."
        )));
    }
    check_server_vpk(vpk)?;
    let maps = vmap_names(vpk)?;
    let (kind, name) = match maps.as_slice() {
        [] => (ServerContentKind::Addon, stem),
        [one] => (ServerContentKind::Map, one.clone()),
        many if many.contains(&stem) => (ServerContentKind::Map, stem),
        _ => {
            return Err(Error::msg(format!(
                "{label} holds several maps ({}). Rename it after the one to load.",
                maps.join(", ")
            )));
        }
    };
    if !valid_content_name(&name) {
        return Err(Error::msg(format!(
            "{name:?} cannot be served: Deadworks content names are 1 to 64 of a-z, 0-9 and _."
        )));
    }
    Ok((kind, name))
}

pub(super) fn backup_dir(server_dir: &Path) -> PathBuf {
    let stamp = now_iso().replace([':', '.'], "-");
    server_dir.join(CONTENT_BACKUPS).join(stamp)
}

fn move_to_backup(server_dir: &Path, backups: &Path, file: &Path) -> Result<()> {
    let relative = file.strip_prefix(server_dir).map_err(|_| Error::msg("Refusing a file outside the server folder."))?;
    let target = backups.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(file, &target).is_err() {
        fs::copy(file, &target)?;
        fs::remove_file(file)?;
    }
    Ok(())
}

struct Planned {
    kind: ServerContentKind,
    name: String,
    /// The VPK copied in: the picked file, or the map taken out of it.
    from: PathBuf,
    /// The file the player picked.
    picked: PathBuf,
    picked_sha256: Option<String>,
    dest: PathBuf,
}

/// A scratch folder inside the server folder for maps taken out of addon paks, removed when
/// the install ends.
struct UnpackDir(PathBuf);

impl UnpackDir {
    fn new(server_dir: &Path) -> Self {
        UnpackDir(server_dir.join(".sigil-unpack"))
    }
}

impl Drop for UnpackDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A custom map installed as an addon mod is a `pakNN_dir.vpk` whose payload holds the map as
/// `maps/<name>.vpk`. The server needs that inner file in its own `maps/` folder, so it is
/// written to `out/<name>.vpk`. None for any other VPK (a map or addon VPK used as it is).
pub(super) fn unpack_addon_map(vpk: &Path, out: &Path) -> Result<Option<PathBuf>> {
    let file = vpk.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    let is_pak = file.len() == "pak00_dir.vpk".len()
        && file.starts_with("pak")
        && file.ends_with("_dir.vpk")
        && file[3..5].bytes().all(|b| b.is_ascii_digit());
    if !is_pak {
        return Ok(None);
    }
    let directory = crate::vpk::read_vpk_directory_with(vpk, &crate::vpk::ReadVpkOptions { keep_preload: false })?;
    let names = crate::maps::map_names_in_paths(directory.entries.iter().map(|e| e.path.as_str()));
    let name = match names.as_slice() {
        [] => return Ok(None),
        [one] => one.to_ascii_lowercase(),
        many => {
            return Err(Error::msg(format!(
                "{file} carries several maps ({}). Server content is one map per file.",
                many.join(", ")
            )));
        }
    };
    if !valid_content_name(&name) {
        return Err(Error::msg(format!(
            "{name:?} cannot be served: Deadworks content names are 1 to 64 of a-z, 0-9 and _."
        )));
    }
    let entry = directory
        .entries
        .iter()
        .find(|e| e.path.replace('\\', "/").eq_ignore_ascii_case(&format!("maps/{name}.vpk")))
        .ok_or_else(|| Error::msg(format!("{file} lists the map {name} but has no maps/{name}.vpk.")))?;
    let data = crate::vpk::read_vpk_entry_data(&directory, entry)?;
    fs::create_dir_all(out)?;
    let target = out.join(format!("{name}.vpk"));
    write_atomic(&target, &data)?;
    Ok(Some(target))
}

/// Copy map and addon VPKs into the server folder and advertise them, so joiners are offered
/// them. Returns the installed names (`maps/<name>` or `addons/<name>`). An installed map mod
/// (`pakNN_dir.vpk` carrying `maps/<name>.vpk`) gives its inner map. Each VPK is checked
/// (readable, safe paths, no executables or archives) before anything is written. A stock file
/// Sigil did not install is never replaced.
pub fn install_server_content(server_dir: &Path, vpks: &[PathBuf], approve: bool) -> Result<Vec<String>> {
    require_approval(approve, "Adding content to the server")?;
    check_server_dir(server_dir, None)?;
    let mut ledger = read_ledger(server_dir);
    let mut planned = Vec::new();
    let unpacked = UnpackDir::new(server_dir);
    let mut sources: Vec<(PathBuf, Option<String>)> = Vec::new();
    for vpk in vpks {
        if !vpk.is_file() {
            return Err(Error::msg(format!("{} does not exist.", vpk.display())));
        }
        match unpack_addon_map(vpk, &unpacked.0)? {
            Some(inner) => sources.push((inner, Some(file_sha256(vpk)?))),
            None => sources.push((vpk.clone(), None)),
        }
    }
    for (index, (vpk, source_sha)) in sources.iter().enumerate() {
        let (kind, name) = classify_content(vpk)?;
        let dest = dir_for(server_dir, kind).join(format!("{name}.vpk"));
        if dest.exists() && !ledger.files.contains_key(&key(kind, &name)) {
            return Err(Error::msg(format!(
                "The server already has {name}.vpk from the game files. Sigil does not replace it; stock maps need no upload."
            )));
        }
        if planned.iter().any(|p: &Planned| p.kind == kind && p.name == name) {
            return Err(Error::msg(format!("{name} is in the list twice.")));
        }
        planned.push(Planned {
            kind,
            name,
            from: vpk.clone(),
            picked: vpks[index].clone(),
            picked_sha256: source_sha.clone(),
            dest,
        });
    }

    let backups = backup_dir(server_dir);
    let mut installed = Vec::new();
    for Planned { kind, name, from: source, picked, picked_sha256, dest } in planned {
        let parent = dest.parent().unwrap_or(server_dir);
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".{name}.vpk.sigil-tmp"));
        fs::copy(&source, &temporary)?;
        if dest.exists() {
            move_to_backup(server_dir, &backups, &dest)?;
        }
        fs::rename(&temporary, &dest)?;
        let meta = fs::metadata(&dest)?;
        ledger.files.insert(
            key(kind, &name),
            LedgerEntry {
                sha256: file_sha256(&dest)?,
                size: meta.len(),
                source: picked.display().to_string(),
                installed_at: now_iso(),
                source_sha256: picked_sha256,
            },
        );
        installed.push(key(kind, &name));
    }
    write_ledger(server_dir, &ledger)?;
    let maps: Vec<String> = names_of(&installed, ServerContentKind::Map);
    let addons: Vec<String> = names_of(&installed, ServerContentKind::Addon);
    edit_serverbrowser(server_dir, &backups, |browser| {
        add_names(browser, "extra_maps", &maps);
        add_names(browser, "content_addons", &addons);
    })?;
    Ok(installed)
}

/// Take content Sigil installed out of the server folder (into the backups) and stop
/// advertising it. `names` are `maps/<name>` or `addons/<name>`, as listed. Returns those removed.
pub fn remove_server_content(server_dir: &Path, names: &[String], approve: bool) -> Result<Vec<String>> {
    require_approval(approve, "Removing content from the server")?;
    check_server_dir(server_dir, None)?;
    let mut ledger = read_ledger(server_dir);
    for name in names {
        if !ledger.files.contains_key(name) {
            return Err(Error::msg(format!("Sigil did not install {name}, so it does not remove it.")));
        }
    }
    let backups = backup_dir(server_dir);
    let mut removed = Vec::new();
    for name in names {
        let (kind, bare) = match name.split_once('/') {
            Some(("maps", bare)) => (ServerContentKind::Map, bare),
            Some(("addons", bare)) => (ServerContentKind::Addon, bare),
            _ => continue,
        };
        if !valid_content_name(bare) {
            continue;
        }
        let file = dir_for(server_dir, kind).join(format!("{bare}.vpk"));
        if file.is_file() {
            move_to_backup(server_dir, &backups, &file)?;
        }
        ledger.files.remove(name);
        removed.push(name.clone());
    }
    write_ledger(server_dir, &ledger)?;
    let maps = names_of(&removed, ServerContentKind::Map);
    let addons = names_of(&removed, ServerContentKind::Addon);
    edit_serverbrowser(server_dir, &backups, |browser| {
        remove_names(browser, "extra_maps", &maps);
        remove_names(browser, "content_addons", &addons);
    })?;
    Ok(removed)
}

/// Every map and server addon in the server folder, stock and Sigil's. Hashes are computed for
/// Sigil's files only (stock maps are large and need no upload).
pub fn list_server_content(server_dir: &Path) -> Result<Vec<ServerContent>> {
    let ledger = read_ledger(server_dir);
    let browser = read_serverbrowser(server_dir).unwrap_or_default();
    let listed = |field: &str, name: &str| {
        browser.get(field).and_then(Value::as_array).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(name)))
    };
    let mut out = Vec::new();
    for kind in [ServerContentKind::Map, ServerContentKind::Addon] {
        let Ok(entries) = fs::read_dir(dir_for(server_dir, kind)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".vpk")) else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || !valid_content_name(name) {
                continue;
            }
            let recorded = ledger.files.get(&key(kind, name));
            let advertised = match kind {
                ServerContentKind::Map => listed("extra_maps", name),
                ServerContentKind::Addon => listed("content_addons", name),
            };
            out.push(ServerContent {
                kind,
                name: name.to_string(),
                size: meta.len(),
                hash: recorded.map(|r| r.sha256.chars().take(16).collect()).unwrap_or_default(),
                ours: recorded.is_some(),
                advertised,
                path,
            });
        }
    }
    out.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    Ok(out)
}

/// Map names the server can load: every `game/citadel/maps/<name>.vpk`.
pub fn server_map_names(server_dir: &Path) -> Vec<String> {
    list_server_content(server_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.kind == ServerContentKind::Map)
        .map(|c| c.name)
        .collect()
}

/// Set (or clear with None) `serverbrowser.fastdl_url`, where joiners download content the
/// server advertises. Absolute http(s), at most 128 characters, no credentials, query or fragment.
pub fn set_fastdl_url(server_dir: &Path, url: Option<&str>, approve: bool) -> Result<()> {
    require_approval(approve, "Changing the download address")?;
    check_server_dir(server_dir, None)?;
    let url = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(raw) => Some(parse_fastdl(raw).filter(|u| u.len() <= 128).ok_or_else(|| {
            Error::msg("Use an http or https address of at most 128 characters, without a login, ? or #.")
        })?),
        None => None,
    };
    let backups = backup_dir(server_dir);
    edit_serverbrowser(server_dir, &backups, |browser| match &url {
        Some(url) => {
            browser.insert("fastdl_url".into(), Value::String(url.clone()));
        }
        None => {
            browser.remove("fastdl_url");
        }
    })
}

/// The `fastdl_url` in the server's config, if any.
pub fn fastdl_url(server_dir: &Path) -> Option<String> {
    read_serverbrowser(server_dir)?.get("fastdl_url")?.as_str().map(str::to_string)
}

/// Write the download layout for Sigil's content: `<out>/maps/<name>_<hash16>.vpk.zst` and
/// `<out>/addons/...`, zstd level 19 (window well under the 128 MiB joiners accept). Upload the
/// folder's contents to the host `fastdl_url` points at. Returns the written files.
pub fn build_fastdl(server_dir: &Path, out_dir: &Path, approve: bool) -> Result<Vec<PathBuf>> {
    require_approval(approve, "Building the download folder")?;
    let mut written = Vec::new();
    for item in list_server_content(server_dir)?.into_iter().filter(|c| c.ours) {
        let hash = file_sha256(&item.path)?[..16].to_string();
        let target = out_dir.join(item.kind.dir()).join(format!("{}_{hash}.vpk.zst", item.name));
        if target.is_file() {
            written.push(target);
            continue;
        }
        fs::create_dir_all(target.parent().unwrap_or(out_dir))?;
        let temporary = target.with_extension("zst.sigil-tmp");
        {
            let mut input = BufReader::new(File::open(&item.path)?);
            let output = BufWriter::new(File::create(&temporary)?);
            let mut encoder = zstd::stream::Encoder::new(output, ZSTD_LEVEL)?;
            std::io::copy(&mut input, &mut encoder)?;
            encoder.finish()?.flush()?;
        }
        fs::rename(&temporary, &target)?;
        written.push(target);
    }
    Ok(written)
}

/// One piece of content Sigil put on the server, and the GameBanana file it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentOrigin {
    /// `maps/<name>` or `addons/<name>`.
    pub key: String,
    pub kind: ServerContentKind,
    pub name: String,
    /// None: not installed from GameBanana through Sigil (a local file, another manager).
    /// Friends then need the download port for it.
    pub item: Option<crate::servers::invite::InviteItem>,
}

/// Where each piece of Sigil's server content came from: the install journal entry that wrote
/// the VPK the player picked (matched by SHA-256, else by path). `mod_id_for_slot` names the
/// mod for installs the journal labels by link only.
pub fn content_origins(
    server_dir: &Path,
    journal: &[crate::install::InstallJournalEntry],
    mod_id_for_slot: &dyn Fn(u16) -> Option<u64>,
) -> Vec<ContentOrigin> {
    use crate::servers::invite::{item_for_sha256, journal_item};
    let ledger = read_ledger(server_dir);
    let mut out = Vec::new();
    for (key, entry) in &ledger.files {
        let Some((dir, name)) = key.split_once('/') else { continue };
        let kind = match dir {
            "maps" => ServerContentKind::Map,
            "addons" => ServerContentKind::Addon,
            _ => continue,
        };
        let picked_sha = entry.source_sha256.as_deref().unwrap_or(&entry.sha256);
        let by_path = || {
            journal
                .iter()
                .rev()
                .filter(|e| e.target.display().to_string() == entry.source)
                .find_map(|e| journal_item(e, mod_id_for_slot))
        };
        let item = item_for_sha256(journal, picked_sha, mod_id_for_slot).or_else(by_path);
        out.push(ContentOrigin { key: key.clone(), kind, name: name.to_string(), item });
    }
    out
}

/// The invite for this server: where to join, whether it asks for a password, and the
/// GameBanana files of its content. The second list names the content friends cannot get from
/// GameBanana (they need the download port).
pub fn host_invite(
    target: crate::servers::invite::InviteTarget,
    password: bool,
    origins: &[ContentOrigin],
) -> Result<(crate::servers::invite::Invite, Vec<String>)> {
    let items = origins.iter().filter_map(|o| o.item);
    let invite = crate::servers::invite::Invite::new(target, password, items)?;
    let elsewhere = origins.iter().filter(|o| o.item.is_none()).map(|o| o.name.clone()).collect();
    Ok((invite, elsewhere))
}

/// `<server>/sigil-fastdl`.
pub fn default_fastdl_dir(server_dir: &Path) -> PathBuf {
    server_dir.join(FASTDL_DIR)
}

/// The address a hosted server advertises for its own download server: the placeholder host
/// (`http://0.0.0.0:<port>`), which a joining Sigil swaps for the address it connects to.
pub fn placeholder_fastdl_url(port: u16) -> String {
    format!("http://{}:{port}", crate::servers::FASTDL_PLACEHOLDER_HOST)
}

/// Remove download files [`build_fastdl`] wrote earlier that are not in `keep` (content Sigil
/// has since removed or replaced). Only files named like the layout in `maps/` and `addons/`
/// are touched.
fn prune_fastdl(out_dir: &Path, keep: &[PathBuf]) -> Result<()> {
    for folder in [ServerContentKind::Map.dir(), ServerContentKind::Addon.dir()] {
        let Ok(entries) = fs::read_dir(out_dir.join(folder)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let named = path.file_name().and_then(|n| n.to_str()).is_some_and(super::fastdl_server::served_name);
            if named && entry.file_type().is_ok_and(|t| t.is_file()) && !keep.contains(&path) {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

/// Before a start, inside the player's Start press: when the server folder holds content Sigil
/// installed, bind the download port, refresh `sigil-fastdl/`, advertise
/// `http://0.0.0.0:<port>` in `deadworks.jsonc` and serve the folder. Returns the running
/// download server, or None when there is nothing to serve.
///
/// A download address the player set themselves (any host other than the placeholder) is
/// left alone and nothing is served. With no custom content, Sigil's own placeholder address
/// is taken out again so joiners are not sent to a port nobody listens on.
pub(super) fn start_downloads(server_dir: &Path, port: u16) -> Result<Option<super::FastdlServer>> {
    let every_interface = std::net::SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), port);
    start_downloads_on(server_dir, every_interface)
}

/// [`start_downloads`] on one address (tests bind `127.0.0.1:0`). The advertised port is the
/// one actually bound.
pub(super) fn start_downloads_on(
    server_dir: &Path,
    addr: std::net::SocketAddr,
) -> Result<Option<super::FastdlServer>> {
    let current = fastdl_url(server_dir);
    if current.as_deref().is_some_and(|url| !crate::servers::is_placeholder_fastdl(url)) {
        return Ok(None);
    }
    let ours = list_server_content(server_dir)?.iter().any(|c| c.ours);
    if !ours {
        if current.is_some() {
            set_fastdl_url(server_dir, None, true)?;
        }
        return Ok(None);
    }
    let out = default_fastdl_dir(server_dir);
    fs::create_dir_all(&out)?;
    // Bind first: a taken port fails before minutes of compression.
    let server = super::FastdlServer::start_on(&out, addr, super::FastdlLimits::default())?;
    let written = build_fastdl(server_dir, &out, true)?;
    prune_fastdl(&out, &written)?;
    set_fastdl_url(server_dir, Some(&placeholder_fastdl_url(server.port())), true)?;
    Ok(Some(server))
}

// ---------------------------------------------------------------------------------------------
// deadworks.jsonc
// ---------------------------------------------------------------------------------------------

/// JSONC to JSON: drop `//` and `/* */` comments outside strings and trailing commas.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            (',', _) => {
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn read_config(server_dir: &Path) -> Result<Option<Value>> {
    match fs::read_to_string(deadworks_config_path(server_dir)) {
        Ok(text) => {
            let value: Value = serde_json::from_str(&strip_jsonc(&text)).map_err(|e| {
                Error::msg(format!("deadworks.jsonc could not be read ({e}). Fix it or remove it, then try again."))
            })?;
            Ok(Some(value))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn read_serverbrowser(server_dir: &Path) -> Option<Map<String, Value>> {
    read_config(server_dir).ok()??.get("serverbrowser")?.as_object().cloned()
}

/// Change `serverbrowser` in `deadworks.jsonc`, keeping every other key. The previous file goes
/// to `backups` first. Comments in the old file are kept only in that backup.
fn edit_serverbrowser(server_dir: &Path, backups: &Path, edit: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    let path = deadworks_config_path(server_dir);
    let mut config = read_config(server_dir)?.unwrap_or_else(|| Value::Object(Map::new()));
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::msg("deadworks.jsonc is not a JSON object. Nothing was changed."))?;
    let browser = root
        .entry("serverbrowser")
        .or_insert_with(|| Value::Object(Map::new()));
    let browser = browser
        .as_object_mut()
        .ok_or_else(|| Error::msg("serverbrowser in deadworks.jsonc is not an object. Nothing was changed."))?;
    let before = browser.clone();
    edit(browser);
    if *browser == before && path.exists() {
        return Ok(());
    }
    if path.is_file() {
        let target = backups.join("deadworks.jsonc");
        fs::create_dir_all(backups)?;
        fs::copy(&path, target)?;
    }
    let mut text = String::from("// Edited by Sigil (Host > Content). The previous copy is in sigil-content-backups.\n");
    text.push_str(&serde_json::to_string_pretty(&config)?);
    text.push('\n');
    write_atomic(&path, text.as_bytes())
}

/// One value of `serverbrowser` in `deadworks.jsonc`.
pub(super) fn serverbrowser_value(server_dir: &Path, key: &str) -> Option<Value> {
    read_serverbrowser(server_dir)?.get(key).cloned()
}

/// Set (or with None remove) one value of `serverbrowser`, keeping the rest. The previous file
/// goes to the backups when it changes.
pub(super) fn set_serverbrowser_value(server_dir: &Path, key: &str, value: Option<Value>) -> Result<()> {
    let backups = backup_dir(server_dir);
    edit_serverbrowser(server_dir, &backups, |browser| match value {
        Some(value) => {
            browser.insert(key.to_string(), value);
        }
        None => {
            browser.remove(key);
        }
    })
}

fn names_of(keys: &[String], kind: ServerContentKind) -> Vec<String> {
    keys.iter()
        .filter_map(|k| k.strip_prefix(kind.dir())?.strip_prefix('/').map(str::to_string))
        .collect()
}

fn add_names(browser: &mut Map<String, Value>, field: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let list = browser.entry(field).or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    if let Some(array) = list.as_array_mut() {
        for name in names {
            if !array.iter().any(|v| v.as_str() == Some(name)) {
                array.push(Value::String(name.clone()));
            }
        }
    }
}

fn remove_names(browser: &mut Map<String, Value>, field: &str, names: &[String]) {
    if let Some(array) = browser.get_mut(field).and_then(Value::as_array_mut) {
        array.retain(|v| !v.as_str().is_some_and(|s| names.iter().any(|n| n == s)));
    }
}
