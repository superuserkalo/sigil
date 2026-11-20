//! Headless scenes for the Regions tab, one per state. Registered in `main.rs`. Fixtures only:
//! the catalog is the test fixture from `deadlock_core::relays`, and nothing is fetched, pinged
//! or written to the firewall.

use std::collections::BTreeMap;

use deadlock_core::relays::{self, Catalog, CatalogSource, Installed, InstalledRule};

use super::{Change, Confirm, RegionsState};
use crate::app::State;
use crate::model::Screen;
use crate::screens::settings::options;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("regions-loading", loading),
    ("regions-catalog-error", catalog_error),
    ("regions-default", default_routing),
    ("regions-measuring", measuring),
    ("regions-pending", pending),
    ("regions-apply-dialog", apply_dialog),
    ("regions-waiting", waiting),
    ("regions-blocked", blocked),
    ("regions-blocked-de", blocked_german),
    ("regions-partial", partial),
    ("regions-reset-dialog", reset_dialog),
    ("regions-presets", presets),
    ("regions-all-blocked", all_blocked),
    ("regions-not-windows", not_windows),
    ("regions-indicator", indicator),
];

const FIXTURE: &str = include_str!("../../../../deadlock-core/src/relays/fixtures/sdr_config.json");

fn catalog() -> Catalog {
    relays::parse_catalog(FIXTURE.as_bytes(), super::now() - 3 * 3600).expect("the fixture parses")
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// The rules a clean Apply of `blocked` leaves, as the read back would show them.
fn rules_for(catalog: &Catalog, blocked: &[&str]) -> Vec<InstalledRule> {
    relays::plan(catalog, &ids(blocked))
        .expect("the plan")
        .into_iter()
        .map(|r| InstalledRule {
            name: r.name,
            location: r.location,
            port_start: r.port_start,
            port_end: r.port_end,
            addresses: r.addresses,
            enabled: true,
        })
        .collect()
}

fn pings() -> BTreeMap<String, Option<u32>> {
    [
        ("ams", Some(14)),
        ("dfra", Some(11)),
        ("dxb", Some(118)),
        ("eze", Some(241)),
        ("gum", None),
        ("hkg", Some(212)),
        ("iad", Some(92)),
        ("jnb", Some(176)),
        ("maa2", Some(158)),
        ("sea", Some(161)),
        ("sto", Some(31)),
        ("syd", Some(287)),
        ("tyo", Some(239)),
        ("fra", Some(9)),
    ]
    .into_iter()
    .map(|(id, ms)| (id.to_string(), ms))
    .collect()
}

fn base(s: &mut State) {
    s.load_demo();
    s.screen = Screen::Servers;
    s.game_detected = true;
    crate::screens::servers::scene(s);
    s.servers.tab = crate::screens::servers::Tab::Regions;
    s.servers.regions = RegionsState {
        windows: true,
        loaded: true,
        checked_at_start: true,
        ..RegionsState::default()
    };
    let mut o = options::get();
    o.relay_regions = relays::Choices::default();
    options::set(o);
}

fn with_catalog(s: &mut State, blocked: &[&str]) {
    base(s);
    let c = catalog();
    let rules = rules_for(&c, blocked);
    let regions = &mut s.servers.regions;
    regions.catalog = Some(c);
    regions.source = Some(CatalogSource::Cached);
    regions.installed = Some(Ok(Installed { rules, firewall_on: Some(true) }));
    regions.draft = blocked.iter().map(|s| s.to_string()).collect();
    regions.pings = pings();
    regions.probed_at = Some(super::now() - 120);
    let mut o = options::get();
    o.relay_regions.applied = ids(blocked);
    options::set(o);
}

/// A job that stays running for the snapshot.
fn hold<T: Send + 'static>(job: &mut crate::jobs::Job<T>) {
    job.start(None, || {
        std::thread::sleep(std::time::Duration::from_secs(600));
        Err(String::new())
    });
}

fn loading(s: &mut State) {
    base(s);
}

fn catalog_error(s: &mut State) {
    base(s);
    s.servers.regions.installed = Some(Ok(Installed { rules: Vec::new(), firewall_on: Some(true) }));
    s.servers.regions.catalog_error =
        Some("Valve's relay list could not be loaded: timed out after 15 s".to_string());
}

fn default_routing(s: &mut State) {
    with_catalog(s, &[]);
}

fn measuring(s: &mut State) {
    with_catalog(s, &[]);
    let regions = &mut s.servers.regions;
    regions.pings.retain(|id, _| ["ams", "dfra", "fra", "sto", "iad", "dxb"].contains(&id.as_str()));
    regions.probed_at = None;
    // The page copies the workers' map while the sweep runs.
    regions.live = std::sync::Arc::new(std::sync::Mutex::new(regions.pings.clone()));
    hold(&mut regions.probe_job);
}

fn pending(s: &mut State) {
    with_catalog(s, &[]);
    let regions = &mut s.servers.regions;
    for id in ["hkg", "tyo", "maa2", "syd", "gum"] {
        regions.draft.insert(id.to_string());
    }
    regions.edited = true;
}

fn apply_dialog(s: &mut State) {
    pending(s);
    s.servers.regions.confirm = Some(Confirm::Apply);
}

fn waiting(s: &mut State) {
    pending(s);
    s.servers.regions.change = Some(Change::Apply);
    hold(&mut s.servers.regions.change_job);
}

fn blocked(s: &mut State) {
    with_catalog(s, &["hkg", "tyo", "maa2", "syd", "gum"]);
    let mut o = options::get();
    o.relay_regions.save_preset("Europe and NA", &ids(&["hkg", "tyo", "maa2", "syd", "gum"])).ok();
    o.relay_regions.save_preset("Europe only", &ids(&["hkg", "tyo", "maa2", "syd", "gum", "iad", "sea", "eze", "jnb", "dxb"])).ok();
    options::set(o);
    crate::statusbar::refresh(s);
}

fn blocked_german(s: &mut State) {
    s.prefs.language = crate::screens::settings::i18n::Lang::German;
    crate::screens::settings::i18n::set(s.prefs.language);
    blocked(s);
}

fn partial(s: &mut State) {
    with_catalog(s, &["hkg", "tyo"]);
    let regions = &mut s.servers.regions;
    if let Some(Ok(installed)) = &mut regions.installed {
        // Valve added a relay to Tokyo since the Apply: the rule misses it.
        if let Some(rule) = installed.rules.iter_mut().find(|r| r.location == "tyo") {
            rule.addresses.truncate(2);
        }
        // A rule for a location Valve dropped, and the firewall off for this network.
        installed.rules.push(InstalledRule {
            name: relays::firewall::rule_name("lim", 27015, 27060),
            location: "lim".to_string(),
            port_start: 27015,
            port_end: 27060,
            addresses: vec!["155.133.249.1".parse().expect("address")],
            enabled: true,
        });
        installed.firewall_on = Some(false);
    }
}

fn reset_dialog(s: &mut State) {
    blocked(s);
    s.servers.regions.confirm = Some(Confirm::Reset);
}

fn presets(s: &mut State) {
    blocked(s);
    s.servers.regions.naming = true;
    s.servers.regions.preset_name = rune::TextEdit::new("Scrims with Ana");
}

fn all_blocked(s: &mut State) {
    with_catalog(s, &[]);
    let ids = s.servers.regions.catalog.as_ref().map(Catalog::ids).unwrap_or_default();
    s.servers.regions.draft = ids.into_iter().collect();
    s.servers.regions.edited = true;
}

fn not_windows(s: &mut State) {
    with_catalog(s, &[]);
    s.servers.regions.windows = false;
    s.servers.regions.installed = None;
}

/// Another page while regions are blocked: the status bar marks it, and a launch says so.
fn indicator(s: &mut State) {
    blocked(s);
    s.screen = Screen::Installed;
    super::remind_at_launch(s);
}
