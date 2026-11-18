//! "Faster downloads": a large file fetched as several byte ranges at once, then joined and
//! hashed. Deadlock Mod Manager splits files of 8 MiB or more into 4 ranges (behaviour only;
//! this is Sigil's own code). Anything unexpected (no 206, a wrong `Content-Range`, a short
//! part) makes the caller fall back to one stream, so the setting can never break a download.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Files smaller than this download in one stream.
pub const MIN_SPLIT_BYTES: u64 = 8 * 1024 * 1024;

/// The ranges, inclusive, for `total` bytes in `parts` pieces. Empty when not worth splitting.
pub fn plan(total: u64, parts: u32) -> Vec<(u64, u64)> {
    if parts < 2 || total < MIN_SPLIT_BYTES {
        return Vec::new();
    }
    let parts = u64::from(parts.min(16));
    let size = total.div_ceil(parts);
    let mut out = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + size - 1).min(total - 1);
        out.push((start, end));
        start = end + 1;
    }
    out
}

/// Whether a response allows ranges, and its length, from its headers.
pub fn splittable(accept_ranges: Option<&str>, length: Option<u64>, parts: u32) -> Option<u64> {
    let ranges = accept_ranges.is_some_and(|v| v.trim().eq_ignore_ascii_case("bytes"));
    let length = length?;
    (ranges && !plan(length, parts).is_empty()).then_some(length)
}

/// `bytes 0-99/1000` matches the range asked for and the total.
pub fn content_range_ok(header: &str, start: u64, end: u64, total: u64) -> bool {
    let Some(rest) = header.trim().strip_prefix("bytes ") else {
        return false;
    };
    let Some((span, size)) = rest.split_once('/') else {
        return false;
    };
    let Some((a, b)) = span.split_once('-') else {
        return false;
    };
    a.trim().parse::<u64>().ok() == Some(start)
        && b.trim().parse::<u64>().ok() == Some(end)
        && (size.trim() == "*" || size.trim().parse::<u64>().ok() == Some(total))
}

fn part_path(path: &Path, index: usize) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}.part{index}"))
}

/// One range into `part`, reading `stop` after every chunk.
fn fetch_part(
    agent: &ureq::Agent,
    url: &str,
    user_agent: &str,
    part: &Path,
    (start, end): (u64, u64),
    total: u64,
    stop: &dyn Fn() -> bool,
) -> Result<()> {
    let mut response = agent
        .get(url)
        .header("User-Agent", user_agent)
        .header("Range", &format!("bytes={start}-{end}"))
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    if response.status().as_u16() != 206 {
        return Err(Error::Network(format!("range answered HTTP {}", response.status())));
    }
    let header = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_range_ok(&header, start, end, total) {
        return Err(Error::Network(format!("unexpected Content-Range {header:?}")));
    }
    let mut file = File::create(part)?;
    let mut reader = response.body_mut().with_config().limit(end - start + 1).reader();
    let mut buffer = vec![0u8; PART_CHUNK_BYTES];
    let mut copied: u64 = 0;
    loop {
        if stop() {
            return Err(Error::Cancelled);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        copied += read as u64;
    }
    if copied != end - start + 1 {
        return Err(Error::Network(format!("range {start}-{end} came back short")));
    }
    Ok(())
}

/// Fetch `url` into `path` in ranges with `agent`. Returns the bytes written and their
/// sha256. On any error every part file is removed and nothing is left at `path`.
pub fn fetch_ranges(
    agent: &ureq::Agent,
    url: &str,
    user_agent: &str,
    path: &Path,
    total: u64,
    parts: u32,
) -> Result<(u64, String)> {
    fetch_ranges_with_cancel(agent, url, user_agent, path, total, parts, None)
}

/// Bytes each part reads between two looks at the cancel flag.
const PART_CHUNK_BYTES: usize = 256 * 1024;

/// [`fetch_ranges`] that stops when `cancel` is set. Every part reads the flag after each
/// chunk and once more before the parts are joined, so a stop lands as fast as on the
/// one-stream path. A cancel removes every part file and returns [`Error::Cancelled`]; the
/// caller must not fall back to one stream on that error. A part that fails also tells the
/// others to stop, so a broken range does not wait for the rest to finish.
pub fn fetch_ranges_with_cancel(
    agent: &ureq::Agent,
    url: &str,
    user_agent: &str,
    path: &Path,
    total: u64,
    parts: u32,
    cancel: Option<&AtomicBool>,
) -> Result<(u64, String)> {
    let cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::SeqCst));
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let abort = AtomicBool::new(false);
    let ranges = plan(total, parts);
    if ranges.is_empty() {
        return Err(Error::msg("Not worth splitting."));
    }
    let cleanup = |count: usize| {
        for i in 0..count {
            let _ = fs::remove_file(part_path(path, i));
        }
    };
    let results: Vec<Result<()>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges
            .iter()
            .enumerate()
            .map(|(index, (start, end))| {
                let (start, end) = (*start, *end);
                let part = part_path(path, index);
                let abort = &abort;
                let cancelled = &cancelled;
                scope.spawn(move || -> Result<()> {
                    let stop = || abort.load(Ordering::SeqCst) || cancelled();
                    let result = fetch_part(agent, url, user_agent, &part, (start, end), total, &stop);
                    if result.is_err() {
                        abort.store(true, Ordering::SeqCst);
                    }
                    result
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(Error::msg("A download part stopped."))))
            .collect()
    });
    if cancelled() {
        cleanup(ranges.len());
        return Err(Error::Cancelled);
    }
    // A part stopped by another part's failure reports `Cancelled`; the real cause wins.
    let mut errors: Vec<Error> = results.into_iter().filter_map(|r| r.err()).collect();
    if !errors.is_empty() {
        cleanup(ranges.len());
        let index = errors.iter().position(|e| !matches!(e, Error::Cancelled)).unwrap_or(0);
        let error = errors.swap_remove(index);
        return Err(match error {
            Error::Cancelled => Error::Network("A download part stopped.".to_string()),
            other => other,
        });
    }
    let joined = (|| -> Result<(u64, String)> {
        let partial = path.with_extension("joining");
        let mut out = File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut written = 0;
        let mut buffer = vec![0u8; 256 * 1024];
        for index in 0..ranges.len() {
            let mut part = File::open(part_path(path, index))?;
            loop {
                let read = part.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                out.write_all(&buffer[..read])?;
                hasher.update(&buffer[..read]);
                written += read as u64;
            }
        }
        out.flush()?;
        drop(out);
        if written != total {
            let _ = fs::remove_file(&partial);
            return Err(Error::Network(format!("joined {written} of {total} bytes")));
        }
        let _ = fs::remove_file(path);
        fs::rename(&partial, path)?;
        let digest = hasher.finalize();
        Ok((written, digest.iter().map(|b| format!("{b:02x}")).collect()))
    })();
    cleanup(ranges.len());
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    #[test]
    fn plans_cover_every_byte_once() {
        assert!(plan(1000, 4).is_empty(), "small files stay one stream");
        assert!(plan(MIN_SPLIT_BYTES * 2, 1).is_empty());
        let total = MIN_SPLIT_BYTES + 3;
        let ranges = plan(total, 4);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0].0, 0);
        assert_eq!(ranges.last().unwrap().1, total - 1);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1 + 1, pair[1].0);
        }
    }

    #[test]
    fn content_range_must_match() {
        assert!(content_range_ok("bytes 0-99/1000", 0, 99, 1000));
        assert!(content_range_ok("bytes 0-99/*", 0, 99, 1000));
        assert!(!content_range_ok("bytes 0-98/1000", 0, 99, 1000));
        assert!(!content_range_ok("0-99/1000", 0, 99, 1000));
        assert_eq!(splittable(Some("bytes"), Some(MIN_SPLIT_BYTES), 4), Some(MIN_SPLIT_BYTES));
        assert_eq!(splittable(Some("none"), Some(MIN_SPLIT_BYTES), 4), None);
    }

    /// A tiny HTTP server that answers Range requests for `body`.
    fn serve(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                        }
                    }
                    let mut stream = stream;
                    let (a, b) = range.unwrap();
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len(),
                        b - a + 1
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body[a..=b]);
                });
            }
        });
        format!("http://{addr}/file.bin")
    }

    #[test]
    fn ranges_join_into_the_same_bytes() {
        let total = MIN_SPLIT_BYTES as usize + 777;
        let body: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let url = serve(body.clone());
        let dir = std::env::temp_dir().join(format!("sigil-ranges-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.bin");
        let agent: ureq::Agent = crate::net::builder().build().into();
        let (bytes, sha) = fetch_ranges(&agent, &url, "test", &path, total as u64, 4).unwrap();
        assert_eq!(bytes, total as u64);
        assert_eq!(fs::read(&path).unwrap(), body);
        let expected: String = Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sha, expected);
        assert!(!part_path(&path, 0).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_set_cancel_flag_stops_the_parts_and_leaves_nothing() {
        let total = MIN_SPLIT_BYTES as usize + 11;
        let body: Vec<u8> = (0..total).map(|i| (i % 7) as u8).collect();
        let url = serve(body);
        let dir = std::env::temp_dir().join(format!("sigil-ranges-cancel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.bin");
        let agent: ureq::Agent = crate::net::builder().build().into();
        let flag = AtomicBool::new(true);
        let error =
            fetch_ranges_with_cancel(&agent, &url, "test", &path, total as u64, 4, Some(&flag))
                .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert!(!path.exists());
        for index in 0..4 {
            assert!(!part_path(&path, index).exists());
        }
        let flag = AtomicBool::new(false);
        let (bytes, _) =
            fetch_ranges_with_cancel(&agent, &url, "test", &path, total as u64, 4, Some(&flag))
                .unwrap();
        assert_eq!(bytes, total as u64);
        let _ = fs::remove_dir_all(&dir);
    }
}
