//! Online answers for a mod Sigil cannot identify offline ([`crate::identify`] runs first).
//!
//! - Name search: GameBanana's own name filter on `apiv11/Mod/Index`, the Browse search
//!   request (Deadlock Mod Manager and Grimoire both offer a search box for an unknown mod).
//!   [`search_terms`] turns a file name, a folder or the VPK's own title into the words to try.
//! - CRC detect, after Grimoire's unknown-mod detection (`unknownModDetection.ts` and
//!   `archiveCrc.ts`, MIT; reimplemented here): a mod archive on GameBanana stores each file's
//!   CRC-32 and size in its ZIP central directory. Sigil reads the CRC-32 and size of the local
//!   directory VPK, asks each candidate mod for its file list, then fetches only the tail of each
//!   ZIP with an HTTP `Range` request and compares. Nothing is downloaded in full and the archive
//!   is never unpacked. Candidates come from the caller (a name search, a hero's skins page).
//!   Grimoire also reads 7z and RAR headers; Sigil reads ZIP only and says so for the others.
//!
//! Every request goes to hosts that are already allowlisted: `gamebanana.com/apiv11` for the
//! search and the file list (the update check's `Mod/<id>?_csvProperties=_aFiles`), and the
//! GameBanana file hosts for the ranged reads, through the install allowlist on every redirect.
//! Nothing here writes to the game install.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::gamebanana::{BrowseMod, BrowseQuery, GAMEBANANA_API_BASE, GAMEBANANA_USER_AGENT};
use crate::install::{DownloadPolicy, WebUrl};

/// How much of a ZIP's tail the first ranged read asks for. Covers the central directory of
/// any archive with a few hundred files.
pub const ZIP_TAIL_BYTES: u64 = 64 * 1024;
/// The largest central directory Sigil reads for one archive.
pub const MAX_CENTRAL_DIRECTORY: u64 = 4 * 1024 * 1024;
/// Archives probed per detect run, so a broad candidate list cannot hammer the file servers.
pub const MAX_PROBES: usize = 40;
const TIMEOUT_SECONDS: u64 = 20;
const MAX_REDIRECTS: usize = 6;

// ---------------------------------------------------------------------------
// Name search
// ---------------------------------------------------------------------------

/// Words worth searching GameBanana for, best first, without duplicates: the VPK's own title,
/// then the readable file or folder name. `pak05_dir.vpk` and other slot names say nothing and
/// are skipped.
pub fn search_terms(title: Option<&str>, file_or_folder_names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |text: &str| {
        let words = clean_words(text);
        if words.len() >= 3 && !out.iter().any(|known| known.eq_ignore_ascii_case(&words)) {
            out.push(words);
        }
    };
    if let Some(title) = title {
        push(title);
    }
    for name in file_or_folder_names {
        push(name);
    }
    out
}

/// A name with its extension, slot prefix, version tags and separators removed.
fn clean_words(text: &str) -> String {
    let mut stem = text.trim().to_string();
    loop {
        let lower = stem.to_ascii_lowercase();
        let Some(suffix) = [".disabled", ".vpk", ".zip", ".7z", ".rar", "_dir"]
            .iter()
            .find(|suffix| lower.ends_with(*suffix))
        else {
            break;
        };
        stem.truncate(stem.len() - suffix.len());
    }
    let words: Vec<String> = stem
        .split(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']')
        .filter(|word| !word.is_empty())
        .filter(|word| {
            let lower = word.to_ascii_lowercase();
            let slot = lower.len() == 5 && lower.starts_with("pak") && lower[3..].chars().all(|c| c.is_ascii_digit());
            let version = lower.trim_start_matches('v').chars().all(|c| c.is_ascii_digit()) && lower.len() <= 4;
            !slot && !version && lower != "dir"
        })
        .map(str::to_string)
        .collect();
    words.join(" ")
}

/// Search Deadlock mods on GameBanana by name: one `Mod/Index` page with the name filter, most
/// liked first. The same request Browse sends for its search box.
pub fn search_by_name(query: &str, perpage: usize) -> Result<Vec<BrowseMod>> {
    let query = query.trim();
    if query.len() < 3 {
        return Err(Error::msg("Type at least three letters to search GameBanana."));
    }
    let browse = BrowseQuery { search: query.to_string(), ..BrowseQuery::default() };
    Ok(crate::gamebanana::fetch_browse(&browse, 1, perpage)?.mods)
}

// ---------------------------------------------------------------------------
// CRC detect
// ---------------------------------------------------------------------------

/// One file on a mod page, as the probe needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModFile {
    pub file_id: u64,
    pub file_name: String,
    pub download_url: String,
    pub size: Option<u64>,
}

/// A file inside a ZIP, from its central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub crc32: u32,
    pub size: u64,
}

/// The CRC-32 and size of a local file, the fingerprint GameBanana's ZIPs carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub crc32: u32,
    pub size: u64,
}

/// A mod the local VPK was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrcMatch {
    pub gamebanana_id: u64,
    pub file_id: u64,
    pub file_name: String,
    /// The VPK's name inside the archive.
    pub entry: String,
}

/// The fingerprint of the file at `path`, streamed.
pub fn fingerprint(path: &std::path::Path) -> Result<Fingerprint> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(Fingerprint { crc32: hasher.finalize(), size })
}

/// The files of a `Mod/<id>?_csvProperties=_aFiles` (or `ProfilePage`) body. Archived files
/// count: an older upload can be the copy a player still has.
pub fn parse_mod_files(body: &str) -> Result<Vec<ModFile>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| Error::Network(format!("GameBanana file list was not JSON ({error}).")))?;
    let files = value
        .get("_aFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Network("GameBanana sent no file list.".to_string()))?;
    Ok(files
        .iter()
        .filter_map(|file| {
            let file_id = file.get("_idRow").and_then(Value::as_u64)?;
            let download_url = file
                .get("_sDownloadUrl")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://gamebanana.com/dl/{file_id}"));
            Some(ModFile {
                file_id,
                file_name: file.get("_sFile").and_then(Value::as_str).unwrap_or("").to_string(),
                download_url,
                size: file.get("_nFilesize").and_then(Value::as_u64),
            })
        })
        .collect())
}

/// The files of one mod: the update check's request.
pub fn fetch_mod_files(mod_id: u64) -> Result<Vec<ModFile>> {
    let url = format!("{GAMEBANANA_API_BASE}/Mod/{mod_id}?_csvProperties=_idRow,_aFiles");
    let body = crate::gamebanana::get_body(&url, &format!("mod {mod_id} files"))?;
    parse_mod_files(&body)
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Where the central directory sits, from the end-of-central-directory record in `tail` (the
/// last bytes of a ZIP of `total` bytes): `(offset, size)` in the whole file. ZIP64 archives
/// (over 4 GiB or 65535 entries) are refused: no Deadlock mod is that large.
pub fn find_central_directory(tail: &[u8], total: u64) -> Result<(u64, u64)> {
    let start = tail
        .windows(4)
        .rposition(|window| window == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| Error::msg("Not a ZIP archive (no end of central directory)."))?;
    let size = read_u32(tail, start + 12).ok_or_else(|| Error::msg("Truncated ZIP end record."))?;
    let offset = read_u32(tail, start + 16).ok_or_else(|| Error::msg("Truncated ZIP end record."))?;
    if size == u32::MAX || offset == u32::MAX {
        return Err(Error::msg("ZIP64 archives are not checked."));
    }
    let (offset, size) = (u64::from(offset), u64::from(size));
    if offset + size > total {
        return Err(Error::msg("The ZIP's central directory points past the end of the file."));
    }
    Ok((offset, size))
}

/// The `.vpk` entries of a ZIP central directory.
pub fn parse_central_directory(directory: &[u8]) -> Vec<ArchiveEntry> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while read_u32(directory, at) == Some(0x0201_4b50) {
        let (Some(crc32), Some(size), Some(name_len), Some(extra_len), Some(comment_len)) = (
            read_u32(directory, at + 16),
            read_u32(directory, at + 24),
            read_u16(directory, at + 28),
            read_u16(directory, at + 30),
            read_u16(directory, at + 32),
        ) else {
            break;
        };
        let name_start = at + 46;
        let Some(raw) = directory.get(name_start..name_start + usize::from(name_len)) else {
            break;
        };
        let name = String::from_utf8_lossy(raw).replace('\\', "/");
        if name.to_ascii_lowercase().ends_with(".vpk") {
            out.push(ArchiveEntry { name, crc32, size: u64::from(size) });
        }
        at = name_start + usize::from(name_len) + usize::from(extra_len) + usize::from(comment_len);
    }
    out
}

/// The entry of `entries` that is the local file: same CRC-32 and size.
pub fn matching_entry(local: Fingerprint, entries: &[ArchiveEntry]) -> Option<&ArchiveEntry> {
    entries.iter().find(|entry| entry.crc32 == local.crc32 && entry.size == local.size)
}

fn probe_agent() -> ureq::Agent {
    crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(TIMEOUT_SECONDS)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// One ranged GET (`bytes=start-end` or, with `start` None, the last `end` bytes), following
/// redirects only inside `policy`. Returns the bytes and the file's total size.
fn ranged(
    agent: &ureq::Agent,
    policy: &DownloadPolicy,
    url: &str,
    start: Option<u64>,
    end: u64,
) -> Result<(Vec<u8>, u64)> {
    let range = match start {
        Some(start) => format!("bytes={start}-{end}"),
        None => format!("bytes=-{end}"),
    };
    let mut current = WebUrl::parse(url)?;
    for _ in 0..=MAX_REDIRECTS {
        if !policy.allows(&current) {
            return Err(Error::Network(format!(
                "Refusing to read from {}: not a GameBanana file host.",
                current.origin()
            )));
        }
        let mut response = agent
            .get(&current.to_string())
            .header("User-Agent", GAMEBANANA_USER_AGENT)
            .header("Range", &range)
            .call()
            .map_err(|error| Error::Network(format!("{current} did not answer: {error}")))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| Error::Network(format!("{current} redirected without a Location.")))?
                .to_string();
            current = crate::install::resolve_location(&current, &location)?;
            continue;
        }
        if status != 206 {
            return Err(Error::Network(format!("{current} answered HTTP {status} to a range read.")));
        }
        let total = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .ok_or_else(|| Error::Network(format!("{current} sent no usable Content-Range.")))?;
        let want = match start {
            Some(start) => end - start + 1,
            None => end.min(total),
        };
        let bytes = response
            .body_mut()
            .with_config()
            .limit(want + 1)
            .read_to_vec()
            .map_err(|error| Error::Network(format!("{current} range read failed: {error}")))?;
        if bytes.len() as u64 != want {
            return Err(Error::Network(format!("{current} returned {} bytes for a {want} byte range.", bytes.len())));
        }
        return Ok((bytes, total));
    }
    Err(Error::Network(format!("{url} redirected too many times.")))
}

/// The `.vpk` entries of the ZIP at `url`, read with two ranged requests at most.
pub fn probe_zip(agent: &ureq::Agent, policy: &DownloadPolicy, url: &str) -> Result<Vec<ArchiveEntry>> {
    let (tail, total) = ranged(agent, policy, url, None, ZIP_TAIL_BYTES)?;
    let (offset, size) = find_central_directory(&tail, total)?;
    if size > MAX_CENTRAL_DIRECTORY {
        return Err(Error::msg("The archive lists too many files to check."));
    }
    let tail_start = total - tail.len() as u64;
    let directory = if offset >= tail_start {
        let from = (offset - tail_start) as usize;
        tail[from..from + size as usize].to_vec()
    } else if size == 0 {
        Vec::new()
    } else {
        ranged(agent, policy, url, Some(offset), offset + size - 1)?.0
    };
    Ok(parse_central_directory(&directory))
}

/// Grimoire's CRC detect: find which of `mod_ids` ships the VPK at `vpk`. Each mod's ZIP
/// files are probed in page order until one holds a `.vpk` with the same CRC-32 and size;
/// at most [`MAX_PROBES`] archives are read. Files that are not ZIPs are skipped. `cancel`
/// stops between probes. `Ok(None)` when no candidate matched.
pub fn crc_detect(
    vpk: &std::path::Path,
    mod_ids: &[u64],
    cancel: Option<&AtomicBool>,
) -> Result<Option<CrcMatch>> {
    let local = fingerprint(vpk)?;
    let policy = DownloadPolicy::from_environment()?;
    let agent = probe_agent();
    let mut probes = 0;
    for &mod_id in mod_ids {
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(Error::Cancelled);
        }
        let Ok(files) = fetch_mod_files(mod_id) else {
            continue;
        };
        if let Some(found) = probe_files(&agent, &policy, mod_id, &files, local, &mut probes, cancel)? {
            return Ok(Some(found));
        }
        if probes >= MAX_PROBES {
            break;
        }
    }
    Ok(None)
}

fn probe_files(
    agent: &ureq::Agent,
    policy: &DownloadPolicy,
    mod_id: u64,
    files: &[ModFile],
    local: Fingerprint,
    probes: &mut usize,
    cancel: Option<&AtomicBool>,
) -> Result<Option<CrcMatch>> {
    for file in files {
        if !file.file_name.to_ascii_lowercase().ends_with(".zip") {
            continue;
        }
        if *probes >= MAX_PROBES {
            return Ok(None);
        }
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(Error::Cancelled);
        }
        *probes += 1;
        let Ok(entries) = probe_zip(agent, policy, &file.download_url) else {
            continue;
        };
        if let Some(entry) = matching_entry(local, &entries) {
            return Ok(Some(CrcMatch {
                gamebanana_id: mod_id,
                file_id: file.file_id,
                file_name: file.file_name.clone(),
                entry: entry.name.clone(),
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn search_terms_skip_slot_names_and_versions() {
        assert_eq!(
            search_terms(Some("Neon Haze"), &["pak05_dir.vpk", "neon_haze_v2_dir.vpk", "Gold-Abrams (fixed).zip"]),
            vec!["Neon Haze".to_string(), "Gold Abrams fixed".to_string()]
        );
        assert!(search_terms(None, &["pak01_dir.vpk"]).is_empty());
    }

    #[test]
    fn reads_vpk_entries_from_a_zip_tail() {
        let vpk = vec![7u8; 5000];
        let archive = zip_with(&[("readme.txt", b"hi"), ("mods/pak01_dir.vpk", &vpk)]);
        let tail_len = 300.min(archive.len());
        let tail = &archive[archive.len() - tail_len..];
        let (offset, size) = find_central_directory(tail, archive.len() as u64).unwrap();
        let entries = parse_central_directory(&archive[offset as usize..(offset + size) as usize]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "mods/pak01_dir.vpk");
        assert_eq!(entries[0].size, 5000);
        let local = Fingerprint { crc32: crc32fast::hash(&vpk), size: 5000 };
        assert!(matching_entry(local, &entries).is_some());
        assert!(matching_entry(Fingerprint { crc32: 1, size: 5000 }, &entries).is_none());
        assert!(find_central_directory(b"not a zip", 9).is_err());
    }

    #[test]
    fn parses_a_mod_file_list() {
        let body = r#"{"_idRow":9,"_aFiles":[
            {"_idRow":11,"_sFile":"a.zip","_nFilesize":100,"_sDownloadUrl":"https://gamebanana.com/dl/11"},
            {"_idRow":12,"_sFile":"b.7z"},{"_sFile":"no id"}]}"#;
        let files = parse_mod_files(body).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].size, Some(100));
        assert_eq!(files[1].download_url, "https://gamebanana.com/dl/12");
        assert!(parse_mod_files("{}").is_err());
    }

    /// Serves `body` for Range requests, suffix and explicit, counting the requests.
    fn serve(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                counted.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = v.trim().to_string();
                    }
                }
                let total = body.len();
                let (a, b) = match range.split_once('-') {
                    Some(("", n)) => (total - n.parse::<usize>().unwrap().min(total), total - 1),
                    Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                    None => (0, total - 1),
                };
                let mut stream = stream;
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    b - a + 1
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body[a..=b]);
            }
        });
        (format!("http://{addr}"), hits)
    }

    #[test]
    fn probes_a_served_zip_with_range_reads() {
        // Many files push the central directory out of the first tail read.
        let vpk: Vec<u8> = (0..20_000u32).map(|i| (i % 253) as u8).collect();
        let names: Vec<String> = (0..1500).map(|i| format!("padding/file_{i:04}_with_a_long_name.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), b"x".as_slice())).collect();
        entries.push(("pak01_dir.vpk", &vpk));
        let archive = zip_with(&entries);
        let (origin, hits) = serve(archive);
        let policy = DownloadPolicy::gamebanana().with_mirror(&origin).unwrap();
        let agent = probe_agent();
        let found = probe_zip(&agent, &policy, &format!("{origin}/a.zip")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "tail, then the central directory");
        let local = Fingerprint { crc32: crc32fast::hash(&vpk), size: vpk.len() as u64 };
        let files = vec![
            ModFile { file_id: 1, file_name: "other.7z".into(), download_url: format!("{origin}/b.7z"), size: None },
            ModFile { file_id: 2, file_name: "mod.zip".into(), download_url: format!("{origin}/a.zip"), size: None },
        ];
        let mut probes = 0;
        let hit = probe_files(&agent, &policy, 77, &files, local, &mut probes, None).unwrap().unwrap();
        assert_eq!((hit.gamebanana_id, hit.file_id, hit.entry.as_str()), (77, 2, "pak01_dir.vpk"));
        assert_eq!(probes, 1, "the 7z was skipped");

        let outside = DownloadPolicy::gamebanana();
        assert!(probe_zip(&agent, &outside, &format!("{origin}/a.zip")).is_err(), "only allowlisted hosts");
    }
}
