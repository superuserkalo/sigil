use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::firewall::{self, RULE_PREFIX};
use super::probe::{self, Health};
use super::*;

const SDR: &str = include_str!("fixtures/sdr_config.json");
const SHOW_EN: &str = include_str!("fixtures/netsh_show_en.txt");
const SHOW_DE: &str = include_str!("fixtures/netsh_show_de.txt");
const SHOW_NONE: &str = include_str!("fixtures/netsh_show_none_de.txt");
const STATE: &str = include_str!("fixtures/netsh_state.txt");
const EXE: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Deadlock\game\bin\win64\deadlock.exe";

fn catalog() -> Catalog {
    parse_catalog(SDR.as_bytes(), 1_700_000_000).unwrap()
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// --- catalog ----------------------------------------------------------------------------------

#[test]
fn reads_valves_catalog_and_skips_locations_without_relays() {
    let c = catalog();
    assert_eq!(c.revision, 1794268433);
    // dsto has no relays in Valve's list, so it cannot be blocked or pinged.
    assert!(c.location("dsto").is_none());
    assert_eq!(c.locations.len(), 14);
    let ams = c.location("ams").unwrap();
    assert_eq!(ams.city, "Amsterdam");
    assert_eq!(ams.country.as_deref(), Some("Netherlands"));
    assert_eq!(ams.region, Region::Europe);
    assert_eq!(ams.relays.len(), 4);
    assert_eq!(ams.relays[0].port_start, 27015);
    assert_eq!(ams.relays[0].port_end, 27060);
    // Sorted by id.
    let order = c.ids();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
}

#[test]
fn regions_come_from_the_country_not_the_swapped_coordinates() {
    let c = catalog();
    let region = |id: &str| c.location(id).unwrap().region;
    // dfra's coordinates are swapped in Valve's list ([50.11, 8.68]); its country says Europe.
    assert_eq!(region("dfra"), Region::Europe);
    assert_eq!(region("iad"), Region::NorthAmerica);
    assert_eq!(region("sea"), Region::NorthAmerica);
    assert_eq!(region("eze"), Region::SouthAmerica);
    assert_eq!(region("dxb"), Region::MiddleEast);
    assert_eq!(region("jnb"), Region::Africa);
    assert_eq!(region("hkg"), Region::Asia);
    assert_eq!(region("maa2"), Region::Asia);
    assert_eq!(region("syd"), Region::Oceania);
    assert_eq!(region("gum"), Region::Oceania);
    // No country and no known name: coordinates decide.
    assert_eq!(region_for("Somewhere", None, Some((-100.0, 40.0))), Region::NorthAmerica);
    assert_eq!(region_for("Somewhere", None, Some((140.0, 35.0))), Region::Asia);
    assert_eq!(region_for("Somewhere", None, None), Region::Other);
    assert_eq!(region_for("Somewhere", None, Some((500.0, 1.0))), Region::Other);
    let groups = c.by_region();
    assert_eq!(groups.first().map(|g| g.0), Some(Region::NorthAmerica));
    assert!(groups.iter().all(|(_, list)| !list.is_empty()));
}

#[test]
fn refuses_private_addresses_bad_ports_and_odd_ids() {
    let one = |id: &str, ip: &str, ports: &str| {
        format!(r#"{{"success":true,"revision":1,"pops":{{"{id}":{{"desc":"X (Germany)","relays":[{{"ipv4":"{ip}","port_range":{ports}}}]}}}}}}"#)
    };
    assert!(parse_catalog(one("fra", "155.133.226.68", "[27015,27060]").as_bytes(), 0).is_ok());
    for ip in ["10.0.0.1", "192.168.1.1", "127.0.0.1", "100.64.0.1", "224.0.0.1", "0.0.0.0", "not-an-ip"] {
        assert!(parse_catalog(one("fra", ip, "[27015,27060]").as_bytes(), 0).is_err(), "{ip}");
    }
    assert!(parse_catalog(one("fra", "155.133.226.68", "[27060,27015]").as_bytes(), 0).is_err());
    assert!(parse_catalog(one("fra", "155.133.226.68", "[0,27015]").as_bytes(), 0).is_err());
    assert!(parse_catalog(one("fra", "155.133.226.68", "[27015]").as_bytes(), 0).is_err());
    assert!(parse_catalog(one("FRA; x", "155.133.226.68", "[27015,27060]").as_bytes(), 0).is_err());
    assert!(parse_catalog(br#"{"success":false,"pops":{}}"#, 0).is_err());
    assert!(parse_catalog(br#"{"success":true,"pops":{}}"#, 0).is_err());
    assert!(parse_catalog(b"<html>", 0).is_err());
}

#[test]
fn cache_round_trips_and_a_young_cache_is_used_without_the_network() {
    let dir = std::env::temp_dir().join(format!("sigil-relays-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(read_cache(&dir).is_none());
    let c = catalog();
    write_cache(&dir, &c).unwrap();
    assert_eq!(read_cache(&dir).as_ref(), Some(&c));
    let (loaded, source) = load_catalog(&dir, false, c.fetched_at + 60).unwrap();
    assert_eq!(source, CatalogSource::Cached);
    assert_eq!(loaded, c);
    // A tampered cache is ignored.
    std::fs::write(cache_path(&dir), br#"{"revision":1,"fetchedAt":0,"locations":[{"id":"x y","description":"","city":"","country":null,"region":"europe","relays":[]}]}"#).unwrap();
    assert!(read_cache(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- choices -------------------------------------------------------------------------------------

#[test]
fn presets_replace_by_name_and_bad_entries_are_dropped() {
    let mut choices = Choices::default();
    choices.save_preset("EU only", &ids(&["iad", "sea", "iad"])).unwrap();
    choices.save_preset("eu ONLY", &ids(&["hkg"])).unwrap();
    assert_eq!(choices.presets.len(), 1);
    assert_eq!(choices.presets[0].name, "eu ONLY");
    assert_eq!(choices.presets[0].blocked, ids(&["hkg"]));
    assert!(choices.save_preset("  ", &[]).is_err());
    assert!(choices.save_preset("a\nb", &[]).is_err());
    assert!(choices.save_preset(&"x".repeat(61), &[]).is_err());
    choices.delete_preset("eu ONLY");
    assert!(choices.presets.is_empty());

    let read: Choices = serde_json::from_str(
        r#"{"applied":["sea","ams","ams","BAD id"],"presets":[{"name":"A","blocked":["x;y","tyo"]},{"name":"a","blocked":[]},{"name":"","blocked":[]}],"pending":true,"extra":1}"#,
    )
    .unwrap();
    let clean = read.sanitized();
    assert_eq!(clean.applied, ids(&["ams", "sea"]));
    assert_eq!(clean.presets.len(), 1);
    assert_eq!(clean.presets[0].blocked, ids(&["tyo"]));
    assert!(clean.pending);
    // Old options files have no entry at all.
    assert_eq!(serde_json::from_str::<Choices>("{}").unwrap(), Choices::default());
}

// --- plan ----------------------------------------------------------------------------------------

#[test]
fn plan_is_one_rule_per_location_and_port_range() {
    let c = catalog();
    let rules = plan(&c, &ids(&["sto", "ams", "ams"])).unwrap();
    // ams has one port range; sto has two.
    let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["Sigil-Relay-ams-27015-27060", "Sigil-Relay-sto-27015-27060", "Sigil-Relay-sto-27015-27140"]
    );
    assert_eq!(rules[0].addresses.len(), 4);
    let sto_total: usize = rules[1..].iter().map(|r| r.addresses.len()).sum();
    assert_eq!(sto_total, c.location("sto").unwrap().relays.len());
    assert!(plan(&c, &[]).unwrap().is_empty());
    assert!(plan(&c, &ids(&["gone"])).is_err());
    // Blocking everything would leave Deadlock nowhere to play.
    assert!(plan(&c, &c.ids()).is_err());
}

#[test]
fn rule_names_round_trip_and_reject_foreign_names() {
    let name = firewall::rule_name("maa2", 27015, 27078);
    assert_eq!(name, "Sigil-Relay-maa2-27015-27078");
    assert_eq!(firewall::parse_rule_name(&name), Some(("maa2".into(), 27015, 27078)));
    for other in [
        "Sigil-Deadworks-UDP-27067",
        "Sigil-Relay-",
        "Sigil-Relay-ams-27060-27015",
        "Sigil-Relay-AMS-27015-27060",
        "Sigil-Relay-a b-27015-27060",
        "Sigil-Relay-ams-0-27060",
        "DMM.ServerPicker.v1.ams.155.133.248.36.27015-27060",
    ] {
        assert_eq!(firewall::parse_rule_name(other), None, "{other}");
    }
}

#[test]
fn script_deletes_then_adds_with_deadlock_as_the_program() {
    let c = catalog();
    let rules = plan(&c, &ids(&["ams"])).unwrap();
    let text = firewall::script(&ids(&["Sigil-Relay-old-27015-27060"]), &rules, EXE).unwrap();
    let lines: Vec<&str> = text.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "advfirewall firewall delete rule name=Sigil-Relay-old-27015-27060 dir=out");
    assert!(lines[1].starts_with("advfirewall firewall add rule name=Sigil-Relay-ams-27015-27060 dir=out action=block protocol=UDP remoteip=155.133.248.36,155.133.248.37,155.133.248.40,155.133.248.41 remoteport=27015-27060 "));
    assert!(lines[1].contains(&format!("program=\"{EXE}\"")));
    assert!(lines[1].contains("profile=any enable=yes"));
    assert!(text.is_ascii());
    // Only Sigil's own names can be deleted.
    assert!(firewall::script(&ids(&["Core Networking - Teredo (UDP-Out)"]), &[], EXE).is_err());
    // A rule that does not match its name is refused.
    let mut odd = rules.clone();
    odd[0].port_end = 1;
    assert!(firewall::script(&[], &odd, EXE).is_err());
    let mut private = rules.clone();
    private[0].addresses.push(Ipv4Addr::new(192, 168, 0, 1));
    assert!(firewall::script(&[], &private, EXE).is_err());
}

#[test]
fn program_path_must_be_deadlock_and_cannot_break_out_of_its_quotes() {
    assert!(firewall::checked_program(EXE).is_ok());
    assert_eq!(
        firewall::checked_program(r"\\?\D:\Games\Deadlock\game\bin\win64\deadlock.exe").unwrap(),
        r"D:\Games\Deadlock\game\bin\win64\deadlock.exe"
    );
    for bad in [
        r"C:\Windows\System32\cmd.exe",
        r#"C:\x\deadlock.exe" dir=in action=allow program="C:\x\deadlock.exe"#,
        "C:\\x\ny\\deadlock.exe",
        "deadlock.exe",
        "/home/me/deadlock.exe",
    ] {
        assert!(firewall::checked_program(bad).is_err(), "{bad}");
    }
    let path = firewall::deadlock_exe(std::path::Path::new("Deadlock"));
    assert!(path.ends_with("game/bin/win64/deadlock.exe") || path.ends_with(r"game\bin\win64\deadlock.exe"));
}

#[test]
fn elevation_quotes_the_script_path_and_non_ascii_goes_out_as_utf16() {
    let command = firewall::elevation_command(r"C:\Users\O'Brien\AppData\Roaming\Sigil\relay.netsh");
    assert!(command.contains(r#"-ArgumentList '-f "C:\Users\O''Brien\AppData\Roaming\Sigil\relay.netsh"'"#));
    assert!(command.contains("-Verb RunAs"));
    assert!(command.contains("exit 1223"));
    assert_eq!(firewall::script_bytes("abc\r\n"), b"abc\r\n");
    let wide = firewall::script_bytes("ö");
    assert_eq!(wide, vec![0xFF, 0xFE, 0xF6, 0x00]);
}

// --- read back -----------------------------------------------------------------------------------

#[test]
fn reads_sigils_rules_from_english_netsh_output() {
    let rules = firewall::parse_show_rules(SHOW_EN);
    let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["Sigil-Relay-ams-27015-27060", "Sigil-Relay-sto-27015-27060", "Sigil-Relay-old-27015-27060"]
    );
    assert!(rules.iter().all(|r| r.name.starts_with(RULE_PREFIX)));
    assert_eq!(rules[0].location, "ams");
    assert_eq!(rules[0].addresses.len(), 4);
    assert_eq!((rules[0].port_start, rules[0].port_end), (27015, 27060));
    assert!(rules[0].enabled);
    assert!(!rules[2].enabled);
}

#[test]
fn reads_sigils_rules_from_german_netsh_output() {
    let rules = firewall::parse_show_rules(SHOW_DE);
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].location, "fra");
    assert_eq!(rules[0].addresses, vec![Ipv4Addr::new(155, 133, 226, 68), Ipv4Addr::new(155, 133, 226, 69)]);
    assert_eq!((rules[0].port_start, rules[0].port_end), (27015, 27140));
    assert!(rules[0].enabled);
    assert!(!rules[1].enabled);
    assert!(firewall::parse_show_rules(SHOW_NONE).is_empty());
    assert!(firewall::parse_show_rules("").is_empty());
}

#[test]
fn reads_the_firewall_on_or_off() {
    assert_eq!(firewall::parse_profile_state(STATE), Some(false));
    let on = STATE.replace("OFF", "ON");
    assert_eq!(firewall::parse_profile_state(&on), Some(true));
    assert_eq!(firewall::parse_profile_state("garbled"), None);
}

#[test]
fn coverage_tells_full_partial_stale_and_disabled_apart() {
    let c = catalog();
    let installed = firewall::parse_show_rules(SHOW_EN);
    let cover = coverage(Some(&c), &installed);
    // ams: every relay. sto: an address that is no longer one of its relays. old: not in the
    // catalog any more, and turned off.
    assert_eq!(cover.blocked, ids(&["ams"]));
    assert_eq!(cover.partial, ids(&["sto"]));
    assert_eq!(cover.unknown, ids(&["old"]));
    assert_eq!(cover.disabled, ids(&["old"]));
    assert_eq!(cover.rules, 3);
    assert!(cover.any());
    assert!(!cover.matches(&ids(&["ams"])));

    // A clean apply of the plan reads back as exactly the selection.
    let planned = plan(&c, &ids(&["ams", "sto"])).unwrap();
    let clean: Vec<InstalledRule> = planned
        .iter()
        .map(|r| InstalledRule {
            name: r.name.clone(),
            location: r.location.clone(),
            port_start: r.port_start,
            port_end: r.port_end,
            addresses: r.addresses.clone(),
            enabled: true,
        })
        .collect();
    let cover = coverage(Some(&c), &clean);
    assert!(cover.matches(&ids(&["sto", "ams"])));
    assert!(!cover.matches(&ids(&["ams"])));
    // Valve added a relay since: the location reads as partial, so the page asks to apply again.
    let mut grown = c.clone();
    let ams = grown.locations.iter_mut().find(|l| l.id == "ams").unwrap();
    ams.relays.push(Relay { address: Ipv4Addr::new(155, 133, 248, 99), port_start: 27015, port_end: 27060 });
    assert_eq!(coverage(Some(&grown), &clean).partial, ids(&["ams"]));
    // No catalog yet: every rule's location counts as blocked.
    assert_eq!(coverage(None, &clean).blocked, ids(&["ams", "sto"]));
    assert!(!coverage(Some(&c), &[]).any());
    assert!(coverage(Some(&c), &[]).matches(&[]));
}

// --- probe ---------------------------------------------------------------------------------------

#[test]
fn reads_ping_replies_in_any_language() {
    assert_eq!(probe::parse_ping("64 bytes from 155.133.248.36: icmp_seq=0 ttl=56 time=12.8 ms"), Some(12.8));
    assert_eq!(probe::parse_ping("Reply from 155.133.248.36: bytes=32 time=23ms TTL=56"), Some(23.0));
    assert_eq!(probe::parse_ping("Antwort von 155.133.248.36: Bytes=32 Zeit=41ms TTL=56"), Some(41.0));
    assert_eq!(probe::parse_ping("Reply from 10.0.0.1: bytes=32 time<1ms TTL=64"), Some(1.0));
    assert_eq!(probe::parse_ping("Réponse de 1.2.3.4 : octets=32 temps=7 ms TTL=56"), Some(7.0));
    assert_eq!(probe::parse_ping("Request timed out.\nPackets: Sent = 1, Received = 0, Lost = 1 (100% loss),"), None);
    assert_eq!(probe::parse_ping("Reply from 192.168.1.1: Destination host unreachable."), None);
    assert_eq!(probe::parse_ping(""), None);
    assert_eq!(probe::health(Some(30)), Health::Good);
    assert_eq!(probe::health(Some(90)), Health::Fair);
    assert_eq!(probe::health(Some(200)), Health::Poor);
    assert_eq!(probe::health(None), Health::NoAnswer);
}

#[test]
fn measure_tries_two_addresses_and_keeps_the_fastest_echo() {
    let c = catalog();
    let ams = c.location("ams").unwrap();
    let calls = AtomicUsize::new(0);
    let first = ams.relays[0].address;
    // The first relay never answers; the second answers 30, 25, 40.
    let replies = Mutex::new(vec![40.0, 25.0, 30.0]);
    let ping = |address: Ipv4Addr| {
        calls.fetch_add(1, Ordering::SeqCst);
        if address == first { None } else { replies.lock().unwrap().pop() }
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(probe::measure(ams, &cancel, &ping), Some(25));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    // Nothing answers: two addresses tried, then no answer.
    calls.store(0, Ordering::SeqCst);
    assert_eq!(probe::measure(ams, &cancel, &|_| None), None);
}

#[test]
fn sweep_reports_each_location_and_stops_when_cancelled() {
    let c = catalog();
    let seen = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    probe::sweep(&c.locations, &cancel, &|_| Some(10.0), &|id, ms| seen.lock().unwrap().push((id.to_string(), ms)));
    let mut seen = seen.into_inner().unwrap();
    seen.sort();
    assert_eq!(seen.len(), c.locations.len());
    assert!(seen.iter().all(|(_, ms)| *ms == Some(10)));

    // Cancelled before it starts: no pings, no reports.
    let calls = AtomicUsize::new(0);
    let cancel = AtomicBool::new(true);
    let reports = AtomicUsize::new(0);
    probe::sweep(
        &c.locations,
        &cancel,
        &|_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(1.0)
        },
        &|_, _| {
            reports.fetch_add(1, Ordering::SeqCst);
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(reports.load(Ordering::SeqCst), 0);

    // Cancelled mid-sweep: the rest is skipped.
    let cancel = AtomicBool::new(false);
    let calls = AtomicUsize::new(0);
    probe::sweep(
        &c.locations,
        &cancel,
        &|_| {
            if calls.fetch_add(1, Ordering::SeqCst) >= 3 {
                cancel.store(true, Ordering::SeqCst);
            }
            Some(5.0)
        },
        &|_, _| {},
    );
    assert!(calls.load(Ordering::SeqCst) < c.locations.len() * probe::SAMPLES);
}

#[test]
fn apply_and_reset_need_approval() {
    let c = catalog();
    let dir = std::env::temp_dir();
    let error = firewall::apply(&c, &ids(&["ams"]), &dir, &dir, false).unwrap_err().to_string();
    assert!(error.contains("not approved"), "{error}");
    let error = firewall::reset(&dir, false).unwrap_err().to_string();
    assert!(error.contains("not approved"), "{error}");
}
