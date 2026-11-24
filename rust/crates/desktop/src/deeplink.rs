//! "Install with Sigil" links: GameBanana's 1-Click button and Sigil's own `sigil://` links.
//!
//! A link only names a GameBanana mod (and maybe one of its files). Sigil never downloads the
//! URL inside a link: it opens that mod in Browse, reads the mod from the GameBanana API as
//! Browse always does, preselects the file, and shows the normal Install dialog. Nothing is
//! installed until the player confirms there. See `docs/design/deep-links.md`.
//!
//! Accepted forms (anything else is refused with a short reason):
//!
//! - `sigil:https://gamebanana.com/mmdl/<fileId>,Mod,<modId>` (GameBanana 1-Click; `Sound` too,
//!   `/dl/<fileId>` too, on the GameBanana download hosts in `install::GAMEBANANA_DOWNLOAD_HOSTS`)
//! - `sigil://install/mod/<modId>` and `sigil://install/mod/<modId>/<fileId>` (`sound` too)
//! - `sigil://pack/mp1:<code>`: a shared pack (`screens::packs`). It only opens the pack's
//!   preview; nothing installs until the player presses Install pack there. The code must be
//!   `mp1:` plus base64url and at most `deadlock_core::packs::MAX_CODE_BYTES` long.
//! - `sigil://open/<one of the above>`: the Windows and Linux handler command wraps the link the
//!   system passes so Rune's single-instance hand-off (which forwards only `scheme://` arguments)
//!   carries GameBanana's opaque `sigil:https://...` form as well.
//!
//! Registration: Windows writes `HKCU\Software\Classes\sigil` (per user, no admin), Linux an XDG
//! desktop entry, macOS declares the scheme in the app bundle (`scripts/package-desktop.sh`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use deadlock_core::gamebanana::BrowseSection;
use deadlock_core::install::GAMEBANANA_DOWNLOAD_HOSTS;
use deadlock_core::mod_sources::{self, SourceLink};
use rune::Tx;

use crate::app::State;
use crate::screens::browse::InstallView;

/// The URL scheme Sigil registers.
pub const SCHEME: &str = "sigil";
/// The prefix the Windows and Linux handler command puts in front of the link it receives.
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub const WRAPPER: &str = "sigil://open/";
/// The longest link Sigil reads. Real ones are under 80 characters.
pub const MAX_LINK: usize = 512;
/// The longest `deadlock-mod-manager://sources/...` link: it carries a whole descriptor URL.
pub const MAX_SOURCE_LINK: usize = 16_384;
/// Deadlock Mod Manager's three schemes. Its source links are read here and never installed
/// without a confirm.
const DMM_SCHEMES: [&str; 3] = ["deadlock-mod-manager", "deadlock-modmanager", "dlmm"];
/// GameBanana ids fit in 32 bits; ten digits covers that and no more.
const MAX_ID_DIGITS: usize = 10;
/// Marker file in the data folder: present when the player turned links off.
const OFF_MARKER: &str = "links-off";

// --- parsing -------------------------------------------------------------------------

/// What a link asks for: one GameBanana submission, maybe one of its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallLink {
    pub section: BrowseSection,
    pub mod_id: u64,
    pub file_id: Option<u64>,
}

/// Why a link was refused. `Display` is the short reason shown to the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    TooLong,
    NotSigil,
    Malformed,
    NotHttps,
    NotGameBanana,
    NoModId,
    UnsupportedType,
    BadId,
    BadSourceLink,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LinkError::TooLong => "the link is too long",
            LinkError::NotSigil => "it is not a Sigil link",
            LinkError::Malformed => "the link is not in a form Sigil reads",
            LinkError::NotHttps => "the download does not use HTTPS",
            LinkError::NotGameBanana => "it points somewhere other than GameBanana",
            LinkError::NoModId => "it has no mod id, use the Sigil button on the mod's page",
            LinkError::UnsupportedType => "Sigil installs mods and sounds only",
            LinkError::BadId => "the mod or file id is not valid",
            LinkError::BadSourceLink => "the mod source link is not in a form Sigil reads",
        })
    }
}

/// What a link asks for: a GameBanana install, a custom source action, or a pack preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Install(InstallLink),
    Source(SourceLink),
    /// The `mp1:` code of a shared pack, checked for shape only; `deadlock_core::packs` reads it.
    Pack(String),
}

/// The longest pack link: the code cap plus the `sigil://open/sigil://pack/` prefix.
pub const MAX_PACK_LINK: usize = deadlock_core::packs::MAX_CODE_BYTES + 32;

/// `sigil://pack/mp1:<base64url>`, maybe inside the handler's `sigil://open/` wrapper. `None`
/// when the link is not a pack link at all; an error when it is one but malformed.
fn parse_pack(raw: &str) -> Option<Result<String, LinkError>> {
    let rest = strip_scheme(raw)?;
    let rest = match strip_prefix_ci(rest, "//open/") {
        Some(inner) => strip_scheme(inner)?,
        None => rest,
    };
    let code = strip_prefix_ci(rest, "//pack/")?;
    if raw.len() > MAX_PACK_LINK {
        return Some(Err(LinkError::TooLong));
    }
    let code = code.strip_suffix('/').unwrap_or(code);
    let Some(body) = code.strip_prefix(deadlock_core::profiles::SHARE_CODE_PREFIX) else {
        return Some(Err(LinkError::Malformed));
    };
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Some(Err(LinkError::Malformed));
    }
    Some(Ok(code.to_string()))
}

/// Read any link Sigil accepts: its own `sigil:` forms and Deadlock Mod Manager's
/// `deadlock-mod-manager://sources/...` forms. Nothing is added or installed here.
pub fn parse_any(raw: &str) -> Result<Link, LinkError> {
    if let Some(rest) = strip_dmm_scheme(raw) {
        check_source_link(raw, rest)?;
        return mod_sources::parse_source_link(rest)
            .map(Link::Source)
            .map_err(|_| LinkError::BadSourceLink);
    }
    // The handler command's wrapper can carry either scheme.
    if let Some(rest) = strip_scheme(raw)
        && let Some(inner) = strip_prefix_ci(rest, "//open/")
        && let Some(dmm) = strip_dmm_scheme(inner)
    {
        check_source_link(raw, dmm)?;
        return mod_sources::parse_source_link(dmm)
            .map(Link::Source)
            .map_err(|_| LinkError::BadSourceLink);
    }
    if let Some(pack) = parse_pack(raw) {
        return pack.map(Link::Pack);
    }
    parse(raw).map(Link::Install)
}

fn check_source_link(raw: &str, rest: &str) -> Result<(), LinkError> {
    if raw.len() > MAX_SOURCE_LINK {
        return Err(LinkError::TooLong);
    }
    if rest.is_empty() || !raw.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') {
        return Err(LinkError::Malformed);
    }
    Ok(())
}

fn strip_dmm_scheme(link: &str) -> Option<&str> {
    let (scheme, rest) = link.split_once(':')?;
    DMM_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
        .then_some(rest)
}

/// Parses an incoming link. Strict on purpose: the text comes from a web page.
pub fn parse(raw: &str) -> Result<InstallLink, LinkError> {
    if raw.len() > MAX_LINK {
        return Err(LinkError::TooLong);
    }
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') {
        return Err(LinkError::Malformed);
    }
    let rest = strip_scheme(raw).ok_or(LinkError::NotSigil)?;
    // The handler command's wrapper, at most once.
    if let Some(inner) = strip_prefix_ci(rest, "//open/") {
        if strip_prefix_ci(strip_scheme(inner).ok_or(LinkError::Malformed)?, "//open/").is_some() {
            return Err(LinkError::Malformed);
        }
        return parse_unwrapped(inner);
    }
    parse_unwrapped(raw)
}

fn parse_unwrapped(link: &str) -> Result<InstallLink, LinkError> {
    let rest = strip_scheme(link).ok_or(LinkError::NotSigil)?;
    // Some launchers append one slash to a link; more than one is not a link we made.
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.ends_with('/') {
        return Err(LinkError::Malformed);
    }
    match rest.strip_prefix("//") {
        Some(path) => parse_native(path),
        None => parse_one_click(rest),
    }
}

/// `install/<mod|sound>/<modId>[/<fileId>]`.
fn parse_native(path: &str) -> Result<InstallLink, LinkError> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 3 || parts.len() > 4 || !parts[0].eq_ignore_ascii_case("install") {
        return Err(LinkError::Malformed);
    }
    let section = section_of(parts[1])?;
    let mod_id = parse_id(parts[2])?;
    let file_id = parts.get(3).map(|id| parse_id(id)).transpose()?;
    Ok(InstallLink {
        section,
        mod_id,
        file_id,
    })
}

/// `https://<GameBanana download host>/<mmdl|dl>/<fileId>,<Mod|Sound>,<modId>`.
fn parse_one_click(rest: &str) -> Result<InstallLink, LinkError> {
    let Some(url) = strip_prefix_ci(rest, "https://") else {
        return Err(if strip_prefix_ci(rest, "http://").is_some() {
            LinkError::NotHttps
        } else if rest.contains("://") || rest.contains(':') {
            LinkError::NotGameBanana
        } else {
            LinkError::Malformed
        });
    };
    let parts: Vec<&str> = url.split(',').collect();
    let (address, kind, mod_id) = match parts.as_slice() {
        [address] => (*address, None, None),
        [address, kind, mod_id] => (*address, Some(*kind), Some(*mod_id)),
        _ => return Err(LinkError::Malformed),
    };
    let (host, path) = address.split_once('/').ok_or(LinkError::Malformed)?;
    if host.contains(['@', ':'])
        || !GAMEBANANA_DOWNLOAD_HOSTS
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host))
    {
        return Err(LinkError::NotGameBanana);
    }
    let file = path
        .strip_prefix("mmdl/")
        .or_else(|| path.strip_prefix("dl/"))
        .ok_or(LinkError::Malformed)?;
    let file_id = parse_id(file)?;
    let (Some(kind), Some(mod_id)) = (kind, mod_id) else {
        return Err(LinkError::NoModId);
    };
    let section = section_of(kind)?;
    let mod_id = parse_id(mod_id)?;
    Ok(InstallLink {
        section,
        mod_id,
        file_id: Some(file_id),
    })
}

fn section_of(kind: &str) -> Result<BrowseSection, LinkError> {
    if kind.eq_ignore_ascii_case("mod") {
        Ok(BrowseSection::Mods)
    } else if kind.eq_ignore_ascii_case("sound") {
        Ok(BrowseSection::Sounds)
    } else if kind.chars().all(|c| c.is_ascii_alphabetic()) && !kind.is_empty() {
        Err(LinkError::UnsupportedType)
    } else {
        Err(LinkError::Malformed)
    }
}

/// 1 to 10 ASCII digits, no leading zero, at most `u32::MAX`.
fn parse_id(text: &str) -> Result<u64, LinkError> {
    if text.is_empty()
        || text.len() > MAX_ID_DIGITS
        || text.starts_with('0')
        || !text.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(LinkError::BadId);
    }
    let id: u64 = text.parse().map_err(|_| LinkError::BadId)?;
    if id > u64::from(u32::MAX) {
        return Err(LinkError::BadId);
    }
    Ok(id)
}

/// The text after `sigil:` (any case), or none.
fn strip_scheme(link: &str) -> Option<&str> {
    let (scheme, rest) = link.split_once(':')?;
    scheme.eq_ignore_ascii_case(SCHEME).then_some(rest)
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Whether a launch argument is meant for Sigil's link handler (`sigil:` in any case), or a
/// Deadlock Mod Manager source link Sigil reads in the same handler.
pub fn is_sigil_link(arg: &str) -> bool {
    strip_scheme(arg).is_some() || strip_dmm_scheme(arg).is_some()
}

/// A launch that carries a link must carry only that link. The handler command passes exactly
/// one argument, so anything more means a crafted link broke out of its quotes (to pass
/// `--snap` or another flag): such a launch exits before Rune reads its arguments.
pub fn launch_args_safe(args: &[String]) -> bool {
    !args.iter().any(|a| is_sigil_link(a)) || args.len() == 1
}

// --- handling --------------------------------------------------------------------------

thread_local! {
    /// The link whose mod is loading, so its file can be preselected once the files are known.
    static PENDING: RefCell<Option<InstallLink>> = const { RefCell::new(None) };
}

/// `App::on_open_url`: every link the system hands to the running Sigil.
pub fn on_open_url(s: &mut State, url: &str, _tx: &mut Tx) {
    crate::single_instance::bring_forward();
    handle(s, url);
}

/// Opens a link's mod, or says why not. A source link only opens a confirm.
pub fn handle(s: &mut State, url: &str) {
    if !links_on() {
        s.status = "Could not open the link: Install with Sigil links are off in Settings.".into();
        return;
    }
    match parse_any(url) {
        Ok(Link::Install(link)) => open(s, link),
        Ok(Link::Source(link)) => source(s, link),
        // Only the preview opens; its Install pack button is the approval.
        Ok(Link::Pack(code)) => crate::screens::packs::open_link(s, &code),
        Err(error) => s.status = format!("Could not open the link: {error}."),
    }
}

/// A source link never adds, enables or installs by itself: it opens the confirm that names
/// the address and what the link asks for. The player's next press is the approval.
fn source(s: &mut State, link: SourceLink) {
    s.settings.extra.dialog = Some(crate::screens::settings::extra::Dialog::SourceLink(Box::new(link)));
}

/// Browse, with the mod's details open; its file is preselected once the details arrive.
pub fn open(s: &mut State, link: InstallLink) {
    crate::screens::browse::open_submission(s, link.section, link.mod_id);
    PENDING.with(|p| *p.borrow_mut() = Some(link));
}

/// After the mod's details load: open the Install dialog on the linked file. Never installs:
/// the dialog's own buttons do, after the player presses them. Called every `poll`.
pub fn poll(s: &mut State) -> bool {
    let Some(link) = PENDING.with(|p| *p.borrow()) else {
        return false;
    };
    let Some(d) = s.browse_view.details.as_ref() else {
        // The player closed the details first.
        PENDING.with(|p| p.borrow_mut().take());
        return false;
    };
    if d.id != link.mod_id || d.section != link.section {
        PENDING.with(|p| p.borrow_mut().take());
        return false;
    }
    if d.loading {
        return false;
    }
    PENDING.with(|p| p.borrow_mut().take());
    let (Some(file_id), Some(profile)) = (link.file_id, d.profile.as_ref()) else {
        // No file named: a mod with several files opens its file menu to pick from. A failed
        // read: the details view says so.
        if link.file_id.is_none() && d.profile.is_some() {
            let id = d.id;
            crate::screens::browse::quick::choose_after_load(s, id);
            return true;
        }
        return false;
    };
    let Some(file) = profile
        .files
        .iter()
        .chain(profile.archived_files.iter())
        .find(|f| f.id == file_id)
    else {
        s.status = "Could not find that file on the mod. Pick one under Files.".into();
        return true;
    };
    if s.install.busy {
        s.status = "Could not start this install: another install is running.".into();
        return true;
    }
    let mut view = InstallView::for_mod(d.id, &d.name);
    // As the file's own Install button does: the mod id installs its newest file; any other
    // file, and every sound file, goes by the download link GameBanana's API gave.
    let only = profile.files.len() == 1 && !file.archived;
    if (!only || d.section != BrowseSection::Mods) && !file.download_url.is_empty() {
        view.target = file.download_url.clone();
    }
    view.file = Some(file.name.clone());
    s.install = view;
    true
}

// --- the setting -----------------------------------------------------------------------

/// Whether links open in Sigil. On unless the player turned it off.
static LINKS_ON: AtomicBool = AtomicBool::new(true);
/// The last known `Registration`, for Settings.
static REGISTRATION: AtomicU8 = AtomicU8::new(Registration::Unknown as u8);

/// What this computer does with `sigil:` links for this user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Registration {
    Unknown = 0,
    /// The handler points at this Sigil.
    Registered = 1,
    /// Nothing registered (off, or not written yet).
    Missing = 2,
    /// Writing it failed.
    Failed = 3,
    /// macOS: the app bundle declares the scheme; there is nothing to write.
    Bundle = 4,
}

pub fn registration() -> Registration {
    match REGISTRATION.load(Ordering::Relaxed) {
        1 => Registration::Registered,
        2 => Registration::Missing,
        3 => Registration::Failed,
        4 => Registration::Bundle,
        _ => Registration::Unknown,
    }
}

fn set_registration(r: Registration) {
    REGISTRATION.store(r as u8, Ordering::Relaxed);
}

pub fn links_on() -> bool {
    LINKS_ON.load(Ordering::Relaxed)
}

fn marker() -> PathBuf {
    crate::data::data_dir().join(OFF_MARKER)
}

/// At startup, after the single-instance claim: read the setting and make sure the handler
/// points at this executable. The registry or file work runs on its own thread.
pub fn startup() {
    if rune::is_headless() {
        return;
    }
    let on = !marker().exists();
    LINKS_ON.store(on, Ordering::Relaxed);
    let _ = std::thread::Builder::new()
        .name("sigil-links".into())
        .spawn(move || apply(on));
}

/// The Settings switch.
pub fn set_links_on(on: bool) -> Result<(), String> {
    let path = marker();
    if on {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not save the setting: {e}")),
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not save the setting: {e}"))?;
        }
        std::fs::write(&path, b"").map_err(|e| format!("Could not save the setting: {e}"))?;
    }
    LINKS_ON.store(on, Ordering::Relaxed);
    if rune::is_headless() {
        return Ok(());
    }
    apply(on)
}

/// Registers (on) or removes (off) the handler, and records the result.
fn apply(on: bool) -> Result<(), String> {
    let result = if on { os::register() } else { os::unregister() };
    set_registration(match (&result, on) {
        (Err(_), _) => Registration::Failed,
        (Ok(r), _) => *r,
    });
    result.map(|_| ())
}

/// The registry values Sigil writes under `HKEY_CURRENT_USER`: (subkey, value name, data).
/// The command wraps the link (`sigil://open/%1`) so it reaches the running Sigil intact.
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn registry_entries(exe: &str) -> Vec<(String, Option<&'static str>, String)> {
    let root = r"Software\Classes\sigil".to_string();
    vec![
        (root.clone(), None, "URL:Sigil Install Link".into()),
        (root.clone(), Some("URL Protocol"), String::new()),
        (format!(r"{root}\DefaultIcon"), None, format!("\"{exe}\",0")),
        (
            format!(r"{root}\shell\open\command"),
            None,
            format!("\"{exe}\" \"{WRAPPER}%1\""),
        ),
    ]
}

/// The XDG desktop entry Sigil writes on Linux for `x-scheme-handler/sigil`.
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub fn desktop_entry(exe: &str) -> String {
    let mut quoted = String::from('"');
    for c in exe.chars() {
        match c {
            // Two levels: the quoting rule escapes these with a backslash, and the string
            // rule before it doubles every backslash in the file.
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=Sigil\nComment=Opens Install with Sigil links\n\
         Exec={quoted} {WRAPPER}%u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/{SCHEME};\n"
    )
}

#[cfg(target_os = "windows")]
mod os {
    use super::{Registration, registry_entries};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn read(sub: &str, name: Option<&str>) -> Option<String> {
        let sub = wide(sub);
        let name = name.map(wide);
        let mut buf = vec![0u16; 2048];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: NUL-terminated strings; `buf` holds `bytes` bytes and the call writes at most that.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let len = (bytes as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    fn write(sub: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let sub_w = wide(sub);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: NUL-terminated subkey; `key` receives an open handle closed below.
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub_w.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("Could not register the link handler (error {rc})."));
        }
        let data = wide(value);
        let name = name.map(wide);
        // SAFETY: `key` is open; `data` is a NUL-terminated UTF-16 string of the given byte size.
        let rc = unsafe {
            let rc = RegSetValueExW(
                key,
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(key);
            rc
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("Could not register the link handler (error {rc})."));
        }
        Ok(())
    }

    fn exe() -> Result<String, String> {
        let exe =
            std::env::current_exe().map_err(|e| format!("Could not find Sigil's path: {e}"))?;
        Ok(exe.display().to_string())
    }

    /// Writes only the values that differ, so a second run changes nothing.
    pub fn register() -> Result<Registration, String> {
        let exe = exe()?;
        for (sub, name, value) in registry_entries(&exe) {
            if read(&sub, name).as_deref() != Some(value.as_str()) {
                write(&sub, name, &value)?;
            }
        }
        Ok(Registration::Registered)
    }

    pub fn unregister() -> Result<Registration, String> {
        let root = wide(r"Software\Classes\sigil");
        // SAFETY: NUL-terminated subkey of HKCU.
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, root.as_ptr()) };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            return Err(format!("Could not remove the link handler (error {rc})."));
        }
        Ok(Registration::Missing)
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Registration, desktop_entry};
    use std::path::PathBuf;

    const FILE: &str = "sigil-links.desktop";

    fn applications() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("applications"))
    }

    pub fn register() -> Result<Registration, String> {
        let dir = applications().ok_or("No applications folder.")?;
        let exe =
            std::env::current_exe().map_err(|e| format!("Could not find Sigil's path: {e}"))?;
        let entry = desktop_entry(&exe.display().to_string());
        let path = dir.join(FILE);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Could not register the link handler: {e}"))?;
            std::fs::write(&path, entry)
                .map_err(|e| format!("Could not register the link handler: {e}"))?;
            // Best effort: desktops without xdg-utils read MimeType from the entry itself.
            let _ = std::process::Command::new("xdg-mime")
                .args(["default", FILE, "x-scheme-handler/sigil"])
                .status();
        }
        Ok(Registration::Registered)
    }

    pub fn unregister() -> Result<Registration, String> {
        let Some(dir) = applications() else {
            return Ok(Registration::Missing);
        };
        match std::fs::remove_file(dir.join(FILE)) {
            Ok(()) => Ok(Registration::Missing),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registration::Missing),
            Err(e) => Err(format!("Could not remove the link handler: {e}")),
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod os {
    use super::Registration;

    /// macOS: `CFBundleURLTypes` in the bundle's Info.plist registers the scheme when the app
    /// is installed; turning links off makes Sigil ignore them.
    pub fn register() -> Result<Registration, String> {
        Ok(Registration::Bundle)
    }

    pub fn unregister() -> Result<Registration, String> {
        Ok(Registration::Bundle)
    }
}

// --- scenes ----------------------------------------------------------------------------

/// A headless scene: its name and the function that stages it.
pub type Scene = (&'static str, fn(&mut State));

/// Headless scenes for the link-opened states.
pub const SCENES: [Scene; 2] = [
    ("link-install", scene_link_install),
    ("link-rejected", scene_link_rejected),
];

/// A 1-Click link opened: the mod's details with the Install dialog on the linked file.
fn scene_link_install(s: &mut State) {
    crate::screens::browse::scene_details(s);
    let Some(d) = s.browse_view.details.as_ref() else {
        return;
    };
    let file_id = d
        .profile
        .as_ref()
        .and_then(|p| p.files.last())
        .map(|f| f.id);
    let link = InstallLink {
        section: d.section,
        mod_id: d.id,
        file_id,
    };
    PENDING.with(|p| *p.borrow_mut() = Some(link));
    poll(s);
}

/// A link from somewhere other than GameBanana: refused, with the reason in the status line.
fn scene_link_rejected(s: &mut State) {
    crate::screens::browse::scene_fixture(s);
    handle(s, "sigil:https://gamebanana.com.evil.example/mmdl/1,Mod,2");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(section: BrowseSection, mod_id: u64, file_id: Option<u64>) -> InstallLink {
        InstallLink {
            section,
            mod_id,
            file_id,
        }
    }

    #[test]
    fn reads_gamebanana_one_click_links() {
        assert_eq!(
            parse("sigil:https://gamebanana.com/mmdl/1234567,Mod,741478"),
            Ok(link(BrowseSection::Mods, 741478, Some(1234567)))
        );
        assert_eq!(
            parse("sigil:https://files.gamebanana.com/mmdl/13,Sound,42"),
            Ok(link(BrowseSection::Sounds, 42, Some(13)))
        );
        assert_eq!(
            parse("SIGIL:HTTPS://WWW.GAMEBANANA.COM/dl/5,mod,6"),
            Ok(link(BrowseSection::Mods, 6, Some(5)))
        );
        // One trailing slash, as some launchers add.
        assert_eq!(
            parse("sigil:https://gamebanana.com/mmdl/12,Mod,42/"),
            Ok(link(BrowseSection::Mods, 42, Some(12)))
        );
    }

    #[test]
    fn reads_native_and_wrapped_links() {
        assert_eq!(
            parse("sigil://install/mod/741478"),
            Ok(link(BrowseSection::Mods, 741478, None))
        );
        assert_eq!(
            parse("sigil://install/sound/83198/99"),
            Ok(link(BrowseSection::Sounds, 83198, Some(99)))
        );
        assert_eq!(
            parse("sigil://open/sigil:https://gamebanana.com/mmdl/12,Mod,42"),
            Ok(link(BrowseSection::Mods, 42, Some(12)))
        );
        assert_eq!(
            parse("sigil://open/sigil://install/mod/7"),
            Ok(link(BrowseSection::Mods, 7, None))
        );
    }

    #[test]
    fn refuses_other_hosts() {
        for bad in [
            "sigil:https://gamebanana.com.evil.test/mmdl/12,Mod,42",
            "sigil:https://evil.test/gamebanana.com/mmdl/12,Mod,42",
            "sigil:https://evilgamebanana.com/mmdl/12,Mod,42",
            "sigil:https://images.gamebanana.com/mmdl/12,Mod,42",
            "sigil:https://filecache1.gamebanana.com/mmdl/12,Mod,42",
            "sigil:https://user@gamebanana.com/mmdl/12,Mod,42",
            "sigil:https://gamebanana.com@evil.test/mmdl/12,Mod,42",
            "sigil:https://gamebanana.com:8443/mmdl/12,Mod,42",
            "sigil:https://127.0.0.1/mmdl/12,Mod,42",
            "sigil:https://xn--gamebanana-xyz.com/mmdl/12,Mod,42",
        ] {
            assert_eq!(parse(bad), Err(LinkError::NotGameBanana), "{bad}");
        }
    }

    #[test]
    fn refuses_scheme_confusion() {
        assert_eq!(
            parse("sigil:http://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotHttps)
        );
        assert_eq!(
            parse("grimoire:https://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotSigil)
        );
        assert_eq!(
            parse("dlmm:https://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotSigil)
        );
        assert_eq!(
            parse("https://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotSigil)
        );
        assert_eq!(
            parse("sigilx:https://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotSigil)
        );
        assert_eq!(
            parse("sigil:javascript:alert(1)"),
            Err(LinkError::NotGameBanana)
        );
        assert_eq!(
            parse("sigil:file:///etc/passwd"),
            Err(LinkError::NotGameBanana)
        );
        assert_eq!(
            parse("sigil:ftp://gamebanana.com/mmdl/12,Mod,42"),
            Err(LinkError::NotGameBanana)
        );
        assert_eq!(parse("sigil:"), Err(LinkError::Malformed));
        // The wrapper only once, and only around a Sigil link.
        assert_eq!(
            parse("sigil://open/sigil://open/sigil://install/mod/7"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            parse("sigil://open/https://gamebanana.com/mmdl/1,Mod,2"),
            Err(LinkError::Malformed)
        );
    }

    #[test]
    fn refuses_path_tricks() {
        for bad in [
            "sigil:https://gamebanana.com/mmdl/../../dl/12,Mod,42",
            "sigil:https://gamebanana.com/mods/12,Mod,42",
            "sigil:https://gamebanana.com/mmdl/12/x,Mod,42",
            "sigil:https://gamebanana.com/mmdl/12?x=1,Mod,42",
            "sigil:https://gamebanana.com/mmdl/12#f,Mod,42",
            "sigil:https://gamebanana.com/mmdl/%31%32,Mod,42",
            "sigil:https://gamebanana.com/mmdl/12,Mod,42,extra",
            "sigil:https://gamebanana.com/mmdl/12,Mod",
            "sigil://install/mod/../../7",
            "sigil://install/mod/7/8/9",
            "sigil://install/mod/7//",
            "sigil://update/mod/7",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse(r"sigil:https://gamebanana.com\@evil.test/mmdl/1,Mod,2"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            parse("sigil:https://gamebanana.com/mmdl/12"),
            Err(LinkError::NoModId)
        );
        assert_eq!(
            parse("sigil:https://gamebanana.com/mmdl/12,Wip,42"),
            Err(LinkError::UnsupportedType)
        );
        assert_eq!(
            parse("sigil:https://gamebanana.com/mmdl/12,Map,42"),
            Err(LinkError::UnsupportedType)
        );
    }

    #[test]
    fn refuses_bad_ids_and_text() {
        for bad in [
            "sigil:https://gamebanana.com/mmdl/0,Mod,42",
            "sigil:https://gamebanana.com/mmdl/012,Mod,42",
            "sigil:https://gamebanana.com/mmdl/12,Mod,-42",
            "sigil:https://gamebanana.com/mmdl/12,Mod,+42",
            "sigil:https://gamebanana.com/mmdl/12,Mod,4294967296",
            "sigil:https://gamebanana.com/mmdl/12,Mod,99999999999999999999999",
            "sigil:https://gamebanana.com/mmdl/12,Mod,42.0",
            "sigil:https://gamebanana.com/mmdl/12,Mod,\u{ff14}\u{ff12}",
            "sigil://install/mod/0",
            "sigil://install/mod/abc",
        ] {
            assert!(
                matches!(parse(bad), Err(LinkError::BadId | LinkError::Malformed)),
                "{bad}"
            );
        }
        assert_eq!(
            parse("sigil://install/mod/4294967295"),
            Ok(link(BrowseSection::Mods, 4294967295, None))
        );
        assert_eq!(parse(" sigil://install/mod/7"), Err(LinkError::Malformed));
        assert_eq!(parse("sigil://install/mod/7\n"), Err(LinkError::Malformed));
        assert_eq!(parse("sigil://install/mod/7\0"), Err(LinkError::Malformed));
        assert_eq!(
            parse("sigil://install/mod/7 --snap x.png"),
            Err(LinkError::Malformed)
        );
        let long = format!("sigil://install/mod/7/{}", "1".repeat(MAX_LINK));
        assert_eq!(parse(&long), Err(LinkError::TooLong));
    }

    #[test]
    fn reads_dmm_source_links() {
        use deadlock_core::mod_sources::SourceAction;
        let add = "deadlock-mod-manager://sources/add?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json";
        assert_eq!(
            parse_any(add),
            Ok(Link::Source(SourceLink {
                action: SourceAction::Add,
                url: "https://mods.example.org/dmm-source.json".into(),
                source: String::new(),
                listing: None,
                release: None,
                artifact: None,
            }))
        );
        let install = "DLMM://sources/install?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json&source=community&listing=blue%20haze&release=clean-hud-r2&artifact=blue";
        let Ok(Link::Source(link)) = parse_any(install) else { panic!("{install}") };
        assert_eq!(link.action, SourceAction::Install);
        assert_eq!(link.listing.as_deref(), Some("blue haze"));
        assert_eq!(link.release.as_deref(), Some("clean-hud-r2"));
        assert_eq!(link.artifact.as_deref(), Some("blue"));
        // The handler command's wrapper carries either scheme.
        let wrapped = "sigil://open/deadlock-mod-manager://sources/add?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json";
        assert!(matches!(parse_any(wrapped), Ok(Link::Source(_))));
        assert!(is_sigil_link(
            "deadlock-mod-manager://sources/add?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json"
        ));
        assert!(is_sigil_link(
            "deadlock-modmanager://sources/add?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json"
        ));
    }

    #[test]
    fn refuses_hostile_dmm_source_links() {
        let url = "url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json";
        for bad in [
            "deadlock-mod-manager://sources/add?url=http%3A%2F%2Fmods.example.org%2Fx.json".to_string(),
            "deadlock-mod-manager://sources/add?url=javascript%3Aalert(1)".to_string(),
            "deadlock-mod-manager://sources/add?url=https%3A%2F%2Flocalhost%2Fx.json".to_string(),
            format!("deadlock-mod-manager://sources/add?{url}&{url}"),
            format!("deadlock-mod-manager://sources/add?{url}&"),
            "deadlock-mod-manager://sources/add?url=%ZZ".to_string(),
            format!("deadlock-mod-manager://sources/install?{url}&source=community&listing=%0A"),
            format!("deadlock-mod-manager://sources/install?{url}&source=community"),
            "deadlock-mod-manager://sources/remove?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json".to_string(),
            "deadlock-mod-manager://registerKey,4765184,DF385693F33BABD5".to_string(),
            "sigil://open/deadlock-mod-manager://sources/add?url=%ZZ".to_string(),
        ] {
            assert!(parse_any(&bad).is_err(), "{bad}");
        }
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(!launch_args_safe(&args(&[
            "deadlock-mod-manager://sources/add?url=https%3A%2F%2Fmods.example.org%2Fdmm-source.json",
            "--snap",
            "C:\\evil.png"
        ])));
    }

    #[test]
    fn reads_pack_links_and_refuses_hostile_ones() {
        let pack = deadlock_core::packs::Pack {
            name: "Friday".into(),
            note: None,
            mods: vec![deadlock_core::packs::PackMod { mod_id: 5, file_id: Some(6), enabled: true }],
        };
        let link = deadlock_core::packs::to_link(&pack).unwrap();
        let code = link.trim_start_matches("sigil://pack/").to_string();
        assert_eq!(parse_any(&link), Ok(Link::Pack(code.clone())));
        assert_eq!(parse_any(&format!("sigil://open/{link}")), Ok(Link::Pack(code.clone())));
        assert_eq!(parse_any(&format!("SIGIL://PACK/{code}")), Ok(Link::Pack(code.clone())));
        assert_eq!(parse_any(&format!("{link}/")), Ok(Link::Pack(code.clone())));
        for bad in [
            "sigil://pack/".to_string(),
            "sigil://pack/mp1:".to_string(),
            "sigil://pack/https://evil.example/x".to_string(),
            "sigil://pack/mp1:abc/../../etc".to_string(),
            "sigil://pack/mp1:abc%2F..".to_string(),
            "sigil://pack/mp1:abc def".to_string(),
            "sigil://pack/mp1:abc\\x".to_string(),
            "sigil://pack/mp1:a\u{200B}b".to_string(),
            format!("sigil://pack/mp1:{}", "A".repeat(MAX_PACK_LINK)),
            "sigil://pack/mp2:abc".to_string(),
        ] {
            assert!(parse_any(&bad).is_err(), "{bad}");
        }
        // A pack link never reads as an install of anything.
        assert!(parse(&link).is_err());
    }

    #[test]
    fn a_link_launch_carries_nothing_else() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(launch_args_safe(&args(&[])));
        assert!(launch_args_safe(&args(&["--snap", "x.png"])));
        assert!(launch_args_safe(&args(&[
            "sigil://open/sigil://install/mod/7"
        ])));
        assert!(!launch_args_safe(&args(&[
            "sigil://open/x",
            "--snap",
            "C:\\evil.png"
        ])));
        assert!(!launch_args_safe(&args(&[
            "--script",
            "a.json",
            "SIGIL:https://x"
        ])));
    }

    #[test]
    fn registry_command_wraps_the_link() {
        let entries = registry_entries(r"C:\Program Files\Sigil\Sigil.exe");
        let command = entries
            .iter()
            .find(|(sub, _, _)| sub.ends_with(r"shell\open\command"))
            .unwrap();
        assert_eq!(
            command.2,
            r#""C:\Program Files\Sigil\Sigil.exe" "sigil://open/%1""#
        );
        assert!(
            entries
                .iter()
                .any(|(sub, name, value)| sub == r"Software\Classes\sigil"
                    && *name == Some("URL Protocol")
                    && value.is_empty())
        );
        // Every subkey stays under the per-user classes root.
        assert!(
            entries
                .iter()
                .all(|(sub, _, _)| sub.starts_with(r"Software\Classes\sigil"))
        );
    }

    #[test]
    fn desktop_entry_quotes_the_path() {
        let entry = desktop_entry("/opt/my $igil/100%/sigil");
        assert!(entry.contains(r#"Exec="/opt/my \\$igil/100%%/sigil" sigil://open/%u"#));
        assert!(entry.contains("MimeType=x-scheme-handler/sigil;\n"));
    }
}
