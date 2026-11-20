//! Windows Firewall rules for the relay region picker, through `netsh advfirewall`.
//!
//! Each rule blocks outbound UDP from `deadlock.exe` to the relays of one location and port
//! range, so other programs (other Steam games use the same relays) are untouched. Rule names
//! are `Sigil-Relay-<location>-<first port>-<last port>`: the prefix marks Sigil's rules, and
//! the rest only ever holds lowercase letters, digits and dashes.
//!
//! Apply and Reset write one `netsh` script (delete every Sigil relay rule there is, add the
//! planned ones) and run it as `netsh -f <script>` behind Windows' admin prompt, once. Then the
//! rules are read back with `netsh advfirewall firewall show rule name=all dir=out` (no prompt):
//! the read back, not the script's exit code, says what is in place. A half-finished run shows up
//! as partial coverage and the page offers Apply again or Reset.
//!
//! Reading needs no admin rights. `netsh` translates its field labels, so the parser goes by the
//! shape of each value (an address list, a port range), never by a label.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use super::{Catalog, Installed, InstalledRule, PlannedRule, plan, valid_location_id};
use crate::error::{Error, Result};
use crate::util::hidden_command;

/// Every Sigil relay rule's name starts with this.
pub const RULE_PREFIX: &str = "Sigil-Relay-";
/// What the rule says in Windows' own firewall window.
pub const RULE_DESCRIPTION: &str = "Sigil relay region picker: keeps Deadlock away from a Valve relay location. Remove it with Reset in Sigil, Servers, Regions.";
/// Shown on macOS and Linux.
pub const NEEDS_WINDOWS: &str = "Blocking relay regions uses Windows Firewall, so it works on Windows only.";
/// Exit code Windows gives when the admin prompt is declined.
const DECLINED: i32 = 1223;

/// Rules can be changed on this system.
pub fn blocking_supported() -> bool {
    cfg!(windows)
}

fn require_windows() -> Result<()> {
    if blocking_supported() { Ok(()) } else { Err(Error::msg(NEEDS_WINDOWS)) }
}

/// The press in the app is the approval; an agent tool never passes `true`.
fn require_approval(approve: bool, what: &str) -> Result<()> {
    if approve {
        Ok(())
    } else {
        Err(Error::msg(format!("{what} was not approved. Nothing was changed.")))
    }
}

/// `Sigil-Relay-fra-27015-27140`.
pub fn rule_name(location: &str, port_start: u16, port_end: u16) -> String {
    format!("{RULE_PREFIX}{location}-{port_start}-{port_end}")
}

/// The location and ports in a Sigil rule name. None for every other name.
pub fn parse_rule_name(name: &str) -> Option<(String, u16, u16)> {
    let rest = name.strip_prefix(RULE_PREFIX)?;
    let mut parts = rest.rsplitn(3, '-');
    let end: u16 = parts.next()?.parse().ok()?;
    let start: u16 = parts.next()?.parse().ok()?;
    let location = parts.next()?;
    (valid_location_id(location) && start > 0 && start <= end).then(|| (location.to_string(), start, end))
}

/// `<game>\game\bin\win64\deadlock.exe`, the program the rules apply to.
pub fn deadlock_exe(game_root: &Path) -> PathBuf {
    game_root.join("game").join("bin").join("win64").join("deadlock.exe")
}

/// The program path as it goes into the script: a drive or UNC path to `deadlock.exe`, with no
/// quote or control character that could end the quoted value. `\\?\` is dropped, because the
/// firewall matches plain paths.
pub fn checked_program(path: &str) -> Result<String> {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let bytes = path.as_bytes();
    let drive = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let unc = path.starts_with(r"\\") && path.len() > 4;
    let named = path.to_ascii_lowercase().ends_with(r"\deadlock.exe");
    if !(drive || unc) || !named || path.contains('"') || path.chars().any(char::is_control) || path.len() > 1024 {
        return Err(Error::msg(format!("Sigil will not write a firewall rule for this program path: {path}")));
    }
    Ok(path.to_string())
}

/// The `netsh` script: delete each named rule, then add each planned one. CRLF lines.
pub fn script(remove: &[String], add: &[PlannedRule], program: &str) -> Result<String> {
    let program = checked_program(program)?;
    let mut lines = Vec::new();
    for name in remove {
        if parse_rule_name(name).is_none() {
            return Err(Error::msg(format!("Not a Sigil relay rule: {name}")));
        }
        lines.push(format!("advfirewall firewall delete rule name={name} dir=out"));
    }
    for rule in add {
        if parse_rule_name(&rule.name) != Some((rule.location.clone(), rule.port_start, rule.port_end)) {
            return Err(Error::msg(format!("Not a Sigil relay rule: {}", rule.name)));
        }
        if rule.addresses.is_empty() || !rule.addresses.iter().all(|a| super::public_address(*a)) {
            return Err(Error::msg(format!("Rule {} has an address Sigil will not block.", rule.name)));
        }
        let addresses: Vec<String> = rule.addresses.iter().map(Ipv4Addr::to_string).collect();
        lines.push(format!(
            "advfirewall firewall add rule name={} dir=out action=block protocol=UDP remoteip={} remoteport={}-{} program=\"{program}\" profile=any enable=yes description=\"{RULE_DESCRIPTION}\"",
            rule.name,
            addresses.join(","),
            rule.port_start,
            rule.port_end,
        ));
    }
    Ok(lines.join("\r\n") + "\r\n")
}

// ---------------------------------------------------------------------------------------------
// Reading back
// ---------------------------------------------------------------------------------------------

fn dashes(line: &str) -> bool {
    let line = line.trim();
    line.len() >= 10 && line.bytes().all(|b| b == b'-')
}

/// The value after the label: `Remote IP:    1.2.3.4/32` gives `1.2.3.4/32`.
fn value(line: &str) -> &str {
    line.split_once(':').map_or("", |(_, v)| v.trim())
}

/// `1.2.3.4/32,5.6.7.8` as addresses. None unless every entry is a single IPv4 address.
fn address_list(value: &str) -> Option<Vec<Ipv4Addr>> {
    let mut out = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        let address = item
            .strip_suffix("/32")
            .or_else(|| item.strip_suffix("/255.255.255.255"))
            .unwrap_or(item);
        out.push(address.parse().ok()?);
    }
    (!out.is_empty()).then_some(out)
}

/// `27015-27060` or `27015`.
fn port_range(value: &str) -> Option<(u16, u16)> {
    match value.split_once('-') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => {
            let port = value.trim().parse().ok()?;
            Some((port, port))
        }
    }
}

/// "No" in the languages Windows ships, as netsh prints the Enabled field.
const NO_WORDS: &[&str] = &["no", "nein", "non", "nee", "nej", "nao", "não", "nie", "ne", "ei", "nem", "hayir", "hayır"];

/// Sigil's rules in `netsh advfirewall firewall show rule name=all dir=out` output, any language.
///
/// A block is the name line, a line of dashes, then `label: value` lines up to a blank line.
/// The name is the value above the dashes. The remote addresses are the value that is a list
/// of IPv4 addresses, the remote ports the value that is a port range (Sigil's rules leave the
/// local address and port at "Any", which is neither). The first value is Enabled; it reads as
/// off when it is a "no" word or equals a later value (Edge traversal is always "no" for an
/// outbound rule, and is the only other yes or no field).
pub fn parse_show_rules(output: &str) -> Vec<InstalledRule> {
    let lines: Vec<&str> = output.lines().collect();
    let mut rules = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index == 0 || !dashes(line) {
            continue;
        }
        let name = value(lines[index - 1]);
        let Some((location, port_start, port_end)) = parse_rule_name(name) else {
            continue;
        };
        let mut values = Vec::new();
        for (offset, line) in lines[index + 1..].iter().enumerate() {
            let next_is_dashes = lines.get(index + offset + 2).is_some_and(|l| dashes(l));
            if line.trim().is_empty() || dashes(line) || next_is_dashes {
                break;
            }
            values.push(value(line));
        }
        let enabled = values.first().is_some_and(|first| {
            let lower = first.to_lowercase();
            !NO_WORDS.contains(&lower.as_str()) && !values[1..].contains(first)
        });
        let addresses = values.iter().find_map(|v| address_list(v)).unwrap_or_default();
        let ports = values.iter().find_map(|v| port_range(v)).unwrap_or((port_start, port_end));
        rules.push(InstalledRule {
            name: name.to_string(),
            location,
            port_start: ports.0,
            port_end: ports.1,
            addresses,
            enabled,
        });
    }
    rules
}

/// `netsh advfirewall show currentprofile state`: Some(false) when any active profile is off,
/// Some(true) when every one shown is on, None when the words are not ones Sigil knows.
pub fn parse_profile_state(output: &str) -> Option<bool> {
    let mut seen_on = false;
    let lines: Vec<&str> = output.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if index == 0 || !dashes(lines[index - 1]) {
            continue;
        }
        match line.split_whitespace().last().map(str::to_ascii_uppercase).as_deref() {
            Some("OFF" | "AUS") => return Some(false),
            Some("ON" | "EIN") => seen_on = true,
            _ => {}
        }
    }
    seen_on.then_some(true)
}

/// Read Sigil's rules and the firewall's on or off state. No prompt, no change.
pub fn read_installed() -> Result<Installed> {
    require_windows()?;
    let output = hidden_command("netsh")
        .args(["advfirewall", "firewall", "show", "rule", "name=all", "dir=out"])
        .output()
        .map_err(|e| Error::msg(format!("netsh could not be started: {e}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    // netsh exits non-zero when no rule matches; that is an empty list, not an error.
    if !output.status.success() && text.lines().any(dashes) {
        return Err(Error::msg("netsh could not list the firewall rules."));
    }
    let rules = parse_show_rules(&text);
    let state = hidden_command("netsh")
        .args(["advfirewall", "show", "currentprofile", "state"])
        .output()
        .ok()
        .and_then(|o| parse_profile_state(&String::from_utf8_lossy(&o.stdout)));
    Ok(Installed { rules, firewall_on: state })
}

// ---------------------------------------------------------------------------------------------
// Changing the rules
// ---------------------------------------------------------------------------------------------

/// The PowerShell that shows Windows' admin prompt once and runs `netsh -f <script>`. The path
/// is in single quotes with `'` doubled, so it reads as one literal. Exit 1223: declined.
pub fn elevation_command(script_path: &str) -> String {
    let quoted = script_path.replace('\'', "''");
    format!(
        "try {{ $p = Start-Process -FilePath netsh.exe -ArgumentList '-f \"{quoted}\"' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode }} catch {{ exit {DECLINED} }}"
    )
}

/// The script's bytes: ASCII as is, anything else as UTF-16 with a byte order mark so a
/// program path with other letters survives.
pub fn script_bytes(script: &str) -> Vec<u8> {
    if script.is_ascii() {
        return script.as_bytes().to_vec();
    }
    let mut out = vec![0xFF, 0xFE];
    for unit in script.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn run_elevated(script: &str, work_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(work_dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = work_dir.join(format!("relay-rules-{}-{stamp}.netsh", std::process::id()));
    std::fs::write(&path, script_bytes(script))?;
    let shown = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let status = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
        .arg(elevation_command(&shown))
        .status();
    let _ = std::fs::remove_file(&path);
    match status?.code() {
        Some(DECLINED) => Err(Error::msg("Windows did not change the firewall: the admin prompt was declined.")),
        // netsh's exit code is not trusted either way: the read back decides.
        _ => Ok(()),
    }
}

/// Block `blocked` (location ids) for Deadlock and nothing else: every Sigil relay rule is
/// removed and the planned ones are added, behind one admin prompt. Returns the read back.
/// `work_dir` holds the script for the moment it runs.
pub fn apply(catalog: &Catalog, blocked: &[String], game_root: &Path, work_dir: &Path, approve: bool) -> Result<Installed> {
    require_approval(approve, "Blocking relay regions")?;
    require_windows()?;
    let planned = plan(catalog, blocked)?;
    let exe = deadlock_exe(game_root);
    if !exe.is_file() {
        return Err(Error::msg(format!("deadlock.exe was not found at {}. Check the game folder in Settings.", exe.display())));
    }
    let program = checked_program(&exe.to_string_lossy())?;
    let before = read_installed()?;
    let mut remove: Vec<String> = before.rules.iter().map(|r| r.name.clone()).collect();
    remove.sort();
    remove.dedup();
    if remove.is_empty() && planned.is_empty() {
        return Ok(before);
    }
    run_elevated(&script(&remove, &planned, &program)?, work_dir)?;
    read_installed()
}

/// Remove every Sigil relay rule, behind one admin prompt. Needs neither the catalog nor the
/// saved choices: the names come from the firewall itself.
pub fn reset(work_dir: &Path, approve: bool) -> Result<Installed> {
    require_approval(approve, "Unblocking relay regions")?;
    require_windows()?;
    let before = read_installed()?;
    let mut remove: Vec<String> = before.rules.iter().map(|r| r.name.clone()).collect();
    remove.sort();
    remove.dedup();
    if remove.is_empty() {
        return Ok(before);
    }
    // The program is not used by delete lines; any valid path passes the check.
    run_elevated(&script(&remove, &[], r"C:\deadlock.exe")?, work_dir)?;
    read_installed()
}
