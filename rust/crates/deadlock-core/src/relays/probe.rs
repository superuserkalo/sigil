//! Latency to each relay location, measured with the system `ping` (ICMP echo), on a press.
//!
//! Bounded: eight workers at once, at most two addresses tried per location and three echoes to
//! the one that answers, one second each, so a full sweep of today's ~35 locations ends within
//! about fifteen seconds even when nothing answers. Cancellable: every worker checks the flag
//! before each echo, so Stop ends the sweep within one echo. Works on every system; blocking is
//! Windows only, measuring is not. The value shown is the fastest echo (queueing only ever adds).

use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::Location;
use crate::util::hidden_command;

/// Workers at once.
pub const WORKERS: usize = 8;
/// Echoes to the address that answered first.
pub const SAMPLES: usize = 3;
/// Addresses tried per location before it counts as not answering.
pub const ADDRESSES: usize = 2;

/// How a location's latency reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Under 60 ms.
    Good,
    /// 60 to 119 ms.
    Fair,
    /// 120 ms or more.
    Poor,
    /// Nothing answered.
    NoAnswer,
}

pub fn health(ms: Option<u32>) -> Health {
    match ms {
        None => Health::NoAnswer,
        Some(ms) if ms < 60 => Health::Good,
        Some(ms) if ms < 120 => Health::Fair,
        Some(_) => Health::Poor,
    }
}

/// The round trip in one `ping` reply, any system, any language: the first number after `=` or
/// `<` that ends in `ms` (`time=12.8 ms`, `Zeit=23ms`, `time<1ms`, `temps=7 ms`). None when no
/// reply line is there (timeouts, unreachable).
pub fn parse_ping(output: &str) -> Option<f32> {
    for line in output.lines() {
        let bytes = line.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b != b'=' && *b != b'<' {
                continue;
            }
            let rest = line[i + 1..].trim_start();
            let number: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
            if number.is_empty() {
                continue;
            }
            let after = rest[number.len()..].trim_start();
            if !after.starts_with("ms") {
                continue;
            }
            let Ok(ms) = number.replace(',', ".").parse::<f32>() else { continue };
            // `time<1ms` means under a millisecond.
            let ms = if *b == b'<' { ms.min(1.0) } else { ms };
            if ms.is_finite() && (0.0..10_000.0).contains(&ms) {
                return Some(ms);
            }
        }
    }
    None
}

/// One echo through the system `ping`, one second at most. Never through a shell.
pub fn ping_once(address: Ipv4Addr) -> Option<f32> {
    let target = address.to_string();
    let mut command = hidden_command("ping");
    #[cfg(windows)]
    command.args(["-n", "1", "-w", "1000", &target]);
    #[cfg(target_os = "macos")]
    command.args(["-n", "-c", "1", "-W", "1000", &target]).env("LC_ALL", "C");
    #[cfg(all(unix, not(target_os = "macos")))]
    command.args(["-n", "-c", "1", "-W", "1", &target]).env("LC_ALL", "C");
    let output = command.output().ok()?;
    parse_ping(&String::from_utf8_lossy(&output.stdout))
}

/// The fastest echo from a location: the first of up to [`ADDRESSES`] relays that answers,
/// then up to [`SAMPLES`] echoes to it. Rounded up to whole milliseconds.
pub fn measure(location: &Location, cancel: &AtomicBool, ping: &(dyn Fn(Ipv4Addr) -> Option<f32> + Sync)) -> Option<u32> {
    let mut addresses: Vec<Ipv4Addr> = location.relays.iter().map(|r| r.address).collect();
    addresses.dedup();
    for address in addresses.into_iter().take(ADDRESSES) {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Some(first) = ping(address) else { continue };
        let mut best = first;
        for _ in 1..SAMPLES {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if let Some(ms) = ping(address) {
                best = best.min(ms);
            }
        }
        return Some(best.ceil().max(1.0) as u32);
    }
    None
}

/// Measure every location on [`WORKERS`] threads and call `report` with each result as it
/// lands. Returns when all are done or `cancel` is set. Offline mode measures nothing.
pub fn sweep(
    locations: &[Location],
    cancel: &AtomicBool,
    ping: &(dyn Fn(Ipv4Addr) -> Option<f32> + Sync),
    report: &(dyn Fn(&str, Option<u32>) + Sync),
) {
    if crate::net::is_offline() {
        return;
    }
    let queue = Mutex::new(locations.iter());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(locations.len()) {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let next = queue.lock().ok().and_then(|mut q| q.next());
                    let Some(location) = next else { return };
                    let ms = measure(location, cancel, ping);
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    report(&location.id, ms);
                }
            });
        }
    });
}

/// One region's line in the status bar's relay health popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionHealth {
    pub region: super::Region,
    /// Locations that answered.
    pub answering: usize,
    /// Locations measured.
    pub measured: usize,
    /// The lowest ping in the region.
    pub best: Option<u32>,
}

/// What a finished sweep says about the relay network, for the status bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub answering: usize,
    pub measured: usize,
    /// The fastest location and its ping.
    pub best: Option<(String, u32)>,
    pub regions: Vec<RegionHealth>,
}

impl Summary {
    /// The bar's tone: every measured location answered, some did, or none.
    pub fn overall(&self) -> Health {
        match (self.answering, self.measured) {
            (_, 0) | (0, _) => Health::NoAnswer,
            (a, m) if a == m => Health::Good,
            _ => Health::Fair,
        }
    }
}

/// Count the pings of a sweep by region, in [`super::Region::ALL`] order.
pub fn summary(locations: &[Location], pings: &std::collections::BTreeMap<String, Option<u32>>) -> Summary {
    let mut out = Summary::default();
    for region in super::Region::ALL {
        let mut line = RegionHealth { region, answering: 0, measured: 0, best: None };
        for location in locations.iter().filter(|l| l.region == region) {
            let Some(ping) = pings.get(&location.id) else { continue };
            line.measured += 1;
            if let Some(ms) = ping {
                line.answering += 1;
                line.best = Some(line.best.map_or(*ms, |b| b.min(*ms)));
                if out.best.as_ref().is_none_or(|(_, best)| ms < best) {
                    out.best = Some((location.id.clone(), *ms));
                }
            }
        }
        if line.measured > 0 {
            out.answering += line.answering;
            out.measured += line.measured;
            out.regions.push(line);
        }
    }
    out
}
