//! Sigil's own log: one text file in `<data>/logs`, capped in size and rotated, at the level
//! chosen in Settings. Every line passes through [`redact`] before it is written, so a key,
//! token or password that reaches a message never reaches the disk (AGENTS.md hard rule).
//!
//! The file never leaves the machine. "Copy last 200 lines" puts the redacted tail on the
//! clipboard for the user to paste where they choose.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

pub const FILE: &str = "sigil.log";
/// The live file rotates once it passes this size.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Rotated files kept beside the live one (`sigil.log.1`, `sigil.log.2`).
pub const KEEP: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Level {
    Off,
    Error,
    #[default]
    Info,
    Debug,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Off, Level::Error, Level::Info, Level::Debug];

    pub fn id(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Error => "error",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }

    pub fn from_id(id: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Off => "Off",
            Level::Error => "Errors",
            Level::Info => "Info",
            Level::Debug => "Debug",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Level::Off => "",
            Level::Error => "ERROR",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(0);
static FILE_LOCK: Mutex<Option<File>> = Mutex::new(None);

pub fn dir() -> PathBuf {
    crate::data::data_dir().join("logs")
}

pub fn path() -> PathBuf {
    dir().join(FILE)
}

/// Set the level. Headless runs never write.
pub fn set_level(level: Level) {
    let level = if rune::is_headless() { Level::Off } else { level };
    LEVEL.store(level as u8, Ordering::Relaxed);
    if level == Level::Off
        && let Ok(mut file) = FILE_LOCK.lock()
    {
        *file = None;
    }
}

pub fn level() -> Level {
    Level::ALL[LEVEL.load(Ordering::Relaxed) as usize % Level::ALL.len()]
}

pub fn error(message: impl AsRef<str>) {
    write(Level::Error, message.as_ref());
}

pub fn info(message: impl AsRef<str>) {
    write(Level::Info, message.as_ref());
}

pub fn debug(message: impl AsRef<str>) {
    write(Level::Debug, message.as_ref());
}

fn write(at: Level, message: &str) {
    let level = level();
    if level == Level::Off || at > level {
        return;
    }
    let line = format!("{} {} {}\n", stamp(), at.tag(), redact(message).replace('\n', " "));
    let Ok(mut slot) = FILE_LOCK.lock() else {
        return;
    };
    if slot.is_none() {
        let _ = fs::create_dir_all(dir());
        *slot = OpenOptions::new().create(true).append(true).open(path()).ok();
    }
    let Some(file) = slot.as_mut() else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
    if file.metadata().map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        *slot = None;
        rotate(&dir(), KEEP);
    }
}

/// `sigil.log.(n-1)` becomes `sigil.log.n`, the live file becomes `.1`, the oldest goes.
pub fn rotate(dir: &std::path::Path, keep: usize) {
    let name = |n: usize| {
        if n == 0 {
            dir.join(FILE)
        } else {
            dir.join(format!("{FILE}.{n}"))
        }
    };
    let _ = fs::remove_file(name(keep));
    for n in (0..keep).rev() {
        let _ = fs::rename(name(n), name(n + 1));
    }
}

fn stamp() -> String {
    chrono_like(std::time::SystemTime::now())
}

/// `2026-11-17 21:04:11` in UTC, without a date crate.
fn chrono_like(at: std::time::SystemTime) -> String {
    let secs = at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The last `n` lines of the live log and, when it is short, the newest rotated one.
pub fn tail(n: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for file in [dir().join(format!("{FILE}.1")), path()] {
        if let Ok(text) = fs::read_to_string(file) {
            lines.extend(text.lines().map(str::to_string));
        }
    }
    let start = lines.len().saturating_sub(n);
    // Redact again: a file from an older build, or a hand edit, may hold anything.
    redact(&lines[start..].join("\n"))
}

/// Total size of the log files.
pub fn size() -> u64 {
    fs::read_dir(dir())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Delete every log file.
pub fn clear() -> Result<(), String> {
    if let Ok(mut slot) = FILE_LOCK.lock() {
        *slot = None;
    }
    let Ok(entries) = fs::read_dir(dir()) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(FILE) {
            fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// --- redaction -------------------------------------------------------------------

const MASK: &str = "[redacted]";

/// Words after which the next value is a secret: `api_key=..`, `"token": ".."`, `Bearer ..`.
const SECRET_NAMES: [&str; 14] = [
    "authorization",
    "bearer",
    "api_key",
    "api-key",
    "apikey",
    "x-api-key",
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "password",
    "secret",
    "client_secret",
    "cookie",
];

/// Prefixes of provider keys that are secret wherever they appear.
const KEY_PREFIXES: [&str; 7] = ["sk-", "sk_", "xai-", "ghp_", "gho_", "github_pat_", "AIza"];

/// Mask keys, tokens, passwords and JWTs in a line.
pub fn redact(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    // Set after a secret's name and its `:` or `=`: the next word is the value.
    let mut armed = false;
    while i < chars.len() {
        if !is_token_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_token_char(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let lower = word.to_ascii_lowercase();
        if lower == "bearer" || lower == "basic" {
            // The scheme stays; the credential after it goes.
            out.push_str(&word);
            armed = true;
            continue;
        }
        if armed || looks_secret(&word) {
            out.push_str(MASK);
            armed = false;
            continue;
        }
        out.push_str(&word);
        let named = SECRET_NAMES
            .iter()
            .any(|n| lower == *n || lower.ends_with(&format!("_{n}")) || lower.ends_with(&format!("-{n}")));
        if named {
            // Arm only when a `:` or `=` follows, across quotes and spaces.
            let mut j = i;
            while j < chars.len() && matches!(chars[j], ' ' | '"' | '\'' | '\t') {
                j += 1;
            }
            armed = j < chars.len() && matches!(chars[j], ':' | '=');
        }
    }
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+')
}

fn looks_secret(word: &str) -> bool {
    if KEY_PREFIXES.iter().any(|p| word.starts_with(p)) && word.len() >= 12 {
        return true;
    }
    // A JWT: three base64url parts, the first starting with `eyJ`.
    if word.starts_with("eyJ") && word.matches('.').count() == 2 && word.len() >= 24 {
        return true;
    }
    // A long run of mixed letters and digits with no other structure: an opaque token.
    let long = word.len() >= 32 && !word.contains('/') && !word.contains('.');
    long && word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_keys_tokens_and_passwords() {
        let cases = [
            "using key sk-ant-api03-AbCdEf1234567890xyz",
            "Authorization: Bearer abcdef0123456789",
            "refresh failed {\"access_token\": \"aZ09aZ09aZ09\"}",
            "api_key=hunter2hunter2",
            "x-api-key: 0123abcd",
            "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJlLXZhbHVl",
            "xai-0123456789abcdefABCDEF",
            "password: correcthorse",
            "github ghp_0123456789abcdefghijABCDEFGHIJ0123",
            "opaque 6f1c2a9b8d7e4f30a1b2c3d4e5f60718293a4b5c",
        ];
        for line in cases {
            let out = redact(line);
            assert!(out.contains(MASK), "{line} -> {out}");
            for secret in [
                "AbCdEf1234567890xyz",
                "abcdef0123456789",
                "aZ09aZ09aZ09",
                "hunter2hunter2",
                "0123abcd",
                "c2lnbmF0dXJlLXZhbHVl",
                "0123456789abcdefABCDEF",
                "correcthorse",
                "0123456789abcdefghijABCDEFGHIJ0123",
                "6f1c2a9b8d7e4f30",
            ] {
                assert!(!out.contains(secret), "{line} leaked {secret}: {out}");
            }
        }
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            "Installed 3 mods into pak05_dir.vpk",
            "GET https://gamebanana.com/apiv11/Mod/123 took 220 ms",
            "Discord presence connected on /tmp/discord-ipc-0",
            "token count 1200",
        ] {
            assert_eq!(redact(line), line);
        }
    }

    #[test]
    fn rotation_keeps_a_fixed_number_of_files() {
        let dir = std::env::temp_dir().join(format!("sigil-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for round in 0..4 {
            fs::write(dir.join(FILE), format!("round {round}")).unwrap();
            rotate(&dir, KEEP);
        }
        assert!(!dir.join(FILE).exists());
        assert_eq!(fs::read_to_string(dir.join(format!("{FILE}.1"))).unwrap(), "round 3");
        assert_eq!(fs::read_to_string(dir.join(format!("{FILE}.2"))).unwrap(), "round 2");
        assert!(!dir.join(format!("{FILE}.3")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stamps_are_civil_dates() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_877_451);
        assert_eq!(chrono_like(at), "2026-11-17 01:04:11");
        assert_eq!(chrono_like(std::time::UNIX_EPOCH), "1970-01-01 00:00:00");
    }
}
