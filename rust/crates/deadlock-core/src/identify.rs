//! Finding out which GameBanana mod a VPK is, for a slot Sigil has no record of.
//!
//! Two offline answers, in order:
//!
//! 1. Metadata the archive itself carries: a root `addoninfo.txt` (KeyValues) with
//!    `gamebananaid` / `gamebananafileid`, or a `modinfo.json`. Compiled by other tools, so it is
//!    exact when present.
//! 2. Sigil's own identity store ([`Identities`], `<data>/mod-identity.json`): the sha256 of a
//!    directory VPK it has seen before, with the mod it turned out to be. Every install already
//!    records the VPK hash in the install journal, and every answer given here is written down,
//!    so a second copy of a known mod is recognised with no network at all.
//!
//! A third answer is the player's: a pasted GameBanana link or id ([`parse_link`]), which the
//! caller then checks against the live page. Nothing here touches the network or the game
//! folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::util::write_atomic;
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};

pub const IDENTITY_FILE_NAME: &str = "mod-identity.json";
pub const IDENTITY_SCHEMA: u32 = 1;
/// An embedded metadata file is small; a larger one is not metadata.
pub const MAX_METADATA_BYTES: u64 = 256 * 1024;

/// What a mod's own files say about where it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embedded {
    pub gamebanana_id: Option<u64>,
    pub file_id: Option<u64>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub source_url: Option<String>,
}

impl Embedded {
    pub fn is_empty(&self) -> bool {
        self.gamebanana_id.is_none()
            && self.title.is_none()
            && self.author.is_none()
            && self.source_url.is_none()
    }
}

/// One remembered match: the bytes of a directory VPK and the mod they belong to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IdentityEntry {
    pub sha256: String,
    pub size_bytes: u64,
    pub gamebanana_id: Option<u64>,
    pub file_id: Option<u64>,
    /// The name to show for it, when there is one.
    pub name: Option<String>,
    /// `embedded`, `journal`, `linked` or `store`.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Identities {
    pub schema: u32,
    pub entries: Vec<IdentityEntry>,
}

impl Default for Identities {
    fn default() -> Self {
        Identities {
            schema: IDENTITY_SCHEMA,
            entries: Vec::new(),
        }
    }
}

impl Identities {
    pub fn find(&self, sha256: &str, size_bytes: u64) -> Option<&IdentityEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.sha256 == sha256 && entry.size_bytes == size_bytes)
    }

    /// Remember a match, replacing an older one for the same bytes.
    pub fn remember(&mut self, entry: IdentityEntry) {
        if entry.sha256.is_empty() {
            return;
        }
        self.entries
            .retain(|known| !(known.sha256 == entry.sha256 && known.size_bytes == entry.size_bytes));
        self.entries.push(entry);
        // Keep the store small: the last 500 mods is more than a library holds.
        const KEEP: usize = 500;
        if self.entries.len() > KEEP {
            let drop = self.entries.len() - KEEP;
            self.entries.drain(0..drop);
        }
    }
}

pub fn identities_path(data_dir: &Path) -> PathBuf {
    data_dir.join(IDENTITY_FILE_NAME)
}

pub fn read_identities(path: &Path) -> Identities {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_identities(path: &Path, store: &Identities) -> Result<()> {
    let text = serde_json::to_vec_pretty(store)?;
    write_atomic(path, &text)
}

/// The sha256 of a file, in lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Read the metadata a VPK carries about itself. `None` when it carries none.
pub fn read_embedded(path: &Path) -> Option<Embedded> {
    let directory = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false }).ok()?;
    let entry = |want: &str| {
        directory
            .entries
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(want))
            .filter(|entry| u64::from(entry.total_length) <= MAX_METADATA_BYTES)
    };

    if let Some(entry) = entry("addoninfo.txt")
        && let Ok(data) = read_vpk_entry_data(&directory, entry)
    {
        let embedded = parse_addon_info(&String::from_utf8_lossy(&data));
        if !embedded.is_empty() {
            return Some(embedded);
        }
    }
    if let Some(entry) = entry("modinfo.json")
        && let Ok(data) = read_vpk_entry_data(&directory, entry)
        && let Ok(text) = std::str::from_utf8(&data)
    {
        let embedded = parse_mod_info(text);
        if !embedded.is_empty() {
            return Some(embedded);
        }
    }
    None
}

/// KeyValues pairs from an `addoninfo.txt`: `"key" "value"`, one per line, braces ignored.
pub fn parse_addon_info(text: &str) -> Embedded {
    let mut embedded = Embedded::default();
    for line in text.lines() {
        let pairs: Vec<&str> = line
            .split('"')
            .enumerate()
            .filter(|(index, part)| index % 2 == 1 && !part.trim().is_empty())
            .map(|(_, part)| part)
            .collect();
        let (Some(key), Some(value)) = (pairs.first(), pairs.get(1)) else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "gamebananaid" | "game_banana_id" => embedded.gamebanana_id = value.parse().ok(),
            "gamebananafileid" | "game_banana_file_id" => embedded.file_id = value.parse().ok(),
            "addontitle" | "title" | "name" => embedded.title = Some(value.to_string()),
            "addonauthor" | "author" => embedded.author = Some(value.to_string()),
            "sourceurl" | "url" => embedded.source_url = Some(value.to_string()),
            _ => {}
        }
    }
    embedded
}

/// The `modinfo.json` shape Grimoire and other packers write.
pub fn parse_mod_info(text: &str) -> Embedded {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Embedded::default();
    };
    let number = |key: &str| value.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    });
    let string = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    Embedded {
        gamebanana_id: number("gamebananaId").or_else(|| number("gameBananaId")),
        file_id: number("gamebananaFileId").or_else(|| number("fileId")),
        title: string("name").or_else(|| string("title")),
        author: string("author"),
        source_url: string("sourceUrl").or_else(|| string("url")),
    }
}

/// A GameBanana mod link, a mod id, or a link with `?file=`: `(mod id, file id)`.
/// Also accepts `https://gamebanana.com/sounds/<id>`.
pub fn parse_link(text: &str) -> Option<(u64, Option<u64>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse().ok().map(|id| (id, None));
    }
    let file_id = text
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("file="))
        .and_then(|value| value.trim().parse::<u64>().ok());
    let rest = text
        .split(['?', '#'])
        .next()
        .unwrap_or(text)
        .trim_end_matches('/');
    let tail = rest.rsplit('/').next()?;
    let id: u64 = tail.parse().ok()?;
    (id > 0).then_some((id, file_id))
}

/// A slot's worth of identification, before the caller writes anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Match {
    pub gamebanana_id: Option<u64>,
    pub file_id: Option<u64>,
    pub name: Option<String>,
    pub author: Option<String>,
    /// `embedded`, `store` or `journal`.
    pub source: String,
}

impl Match {
    pub fn found(&self) -> bool {
        self.gamebanana_id.is_some()
    }
}

/// Identify a slot file without the network: its own metadata, then Sigil's store, then the
/// sha256 of the install journal entry that wrote it.
pub fn identify(
    path: &Path,
    store: &Identities,
    journal: &[crate::install::InstallJournalEntry],
) -> Match {
    if let Some(embedded) = read_embedded(path)
        && embedded.gamebanana_id.is_some()
    {
        return Match {
            gamebanana_id: embedded.gamebanana_id,
            file_id: embedded.file_id,
            name: embedded.title,
            author: embedded.author,
            source: "embedded".to_string(),
        };
    }
    // A file Deadlock Mod Manager stamped keeps its mod id inside, wherever it was moved.
    if let Some(stamp) = crate::dmm_stamp::read(path)
        && let Some(gamebanana_id) = stamp.gamebanana_id()
    {
        return Match {
            gamebanana_id: Some(gamebanana_id),
            file_id: None,
            name: None,
            author: None,
            source: "dmm-stamp".to_string(),
        };
    }
    let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if let Ok(sha256) = sha256_file(path) {
        if let Some(known) = store.find(&sha256, size) {
            return Match {
                gamebanana_id: known.gamebanana_id,
                file_id: known.file_id,
                name: known.name.clone(),
                author: None,
                source: "store".to_string(),
            };
        }
        if let Some(entry) = journal
            .iter()
            .rev()
            .find(|entry| entry.vpk_sha256 == sha256)
        {
            let id = crate::mod_updates::journal_mod_id(entry);
            if id.is_some() {
                return Match {
                    gamebanana_id: id,
                    file_id: entry
                        .download_url
                        .as_deref()
                        .and_then(crate::mod_updates::file_id_from_url),
                    name: None,
                    author: None,
                    source: "journal".to_string(),
                };
            }
        }
    }
    Match::default()
}

/// A GameBanana page to open for a mod, for the identify dialog's "Open on GameBanana".
pub fn mod_url(id: u64) -> String {
    format!("https://gamebanana.com/mods/{id}")
}

/// Check a pasted link before the app fetches anything. Returns the id to look up.
pub fn link_target(text: &str) -> std::result::Result<(u64, Option<u64>), String> {
    parse_link(text).ok_or_else(|| {
        "That is not a GameBanana link or id. Copy a link like https://gamebanana.com/mods/123456."
            .to_string()
    })
}

/// A readable error for a file that no longer exists under the slot name.
pub fn missing_file_error(path: &Path) -> Error {
    Error::msg(format!("{} is not there any more. Scan again.", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk_write::{PackOptions, pack_vpk};

    fn root(tag: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.scratch/identify-test")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));
        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        pack_vpk(&PackOptions::new(staging, root.join("out"), name))
            .unwrap()
            .dir_path
    }

    #[test]
    fn addoninfo_and_modinfo_name_the_mod() {
        let root = root("embedded");
        let info = vpk(
            &root,
            "pak01",
            &[(
                "addoninfo.txt",
                b"\"addontitle\" \"Violet recolor\"\n\"gamebananaid\" \"715100\"\n\"gamebananafileid\" \"445566\"\n",
            )],
        );
        let embedded = read_embedded(&info).unwrap();
        assert_eq!(embedded.gamebanana_id, Some(715_100));
        assert_eq!(embedded.file_id, Some(445_566));
        assert_eq!(embedded.title.as_deref(), Some("Violet recolor"));

        let json = vpk(
            &root,
            "pak02",
            &[(
                "modinfo.json",
                br#"{"kind":"mod","name":"Mirage","gamebananaId":"42","author":"ash"}"#,
            )],
        );
        let embedded = read_embedded(&json).unwrap();
        assert_eq!(embedded.gamebanana_id, Some(42));
        assert_eq!(embedded.author.as_deref(), Some("ash"));

        let plain = vpk(&root, "pak03", &[("materials/a.vmat_c", b"a")]);
        assert!(read_embedded(&plain).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_store_answers_by_hash_and_follows_the_journal() {
        let root = root("store");
        let file = vpk(&root, "pak04", &[("materials/a.vmat_c", b"a")]);
        let size = fs::metadata(&file).unwrap().len();
        let sha = sha256_file(&file).unwrap();

        let path = identities_path(&root);
        let mut store = read_identities(&path);
        assert!(store.find(&sha, size).is_none());
        store.remember(IdentityEntry {
            sha256: sha.clone(),
            size_bytes: size,
            gamebanana_id: Some(9001),
            name: Some("Known".into()),
            source: "linked".into(),
            ..IdentityEntry::default()
        });
        write_identities(&path, &store).unwrap();
        let back = read_identities(&path);
        let matched = identify(&file, &back, &[]);
        assert_eq!(matched.gamebanana_id, Some(9001));
        assert_eq!(matched.source, "store");

        // A journal entry carrying the same hash names the mod when the store is empty.
        let entry = crate::install::InstallJournalEntry {
            vpk_sha256: sha.clone(),
            source_label: "gamebanana:777".to_string(),
            download_url: Some("https://gamebanana.com/dl/888".to_string()),
            ..sample_entry()
        };
        let matched = identify(&file, &Identities::default(), &[entry]);
        assert_eq!(matched.gamebanana_id, Some(777));
        assert_eq!(matched.file_id, Some(888));
        let _ = fs::remove_dir_all(&root);
    }

    fn sample_entry() -> crate::install::InstallJournalEntry {
        crate::install::InstallJournalEntry {
            schema: 1,
            plan_id: String::new(),
            installed_at: String::new(),
            source: String::new(),
            source_label: String::new(),
            download_url: None,
            archive: PathBuf::new(),
            archive_sha256: String::new(),
            slot: 1,
            file: String::new(),
            target: PathBuf::new(),
            chunk_files: Vec::new(),
            entries: 0,
            bytes: 0,
            vpk_bytes: 0,
            vpk_sha256: String::new(),
            backups: Vec::new(),
            removed_chunks: Vec::new(),
            removed_files: Vec::new(),
            gameinfo_repaired: false,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn links_and_ids_parse() {
        assert_eq!(parse_link("https://gamebanana.com/mods/715100"), Some((715_100, None)));
        assert_eq!(
            parse_link("https://gamebanana.com/mods/715100?file=445566"),
            Some((715_100, Some(445_566)))
        );
        assert_eq!(parse_link("https://gamebanana.com/sounds/42/"), Some((42, None)));
        assert_eq!(parse_link(" 715100 "), Some((715_100, None)));
        assert_eq!(parse_link("not a link"), None);
        assert_eq!(parse_link(""), None);
        assert!(link_target("nope").is_err());
        assert!(link_target("715100").is_ok());
    }
}
