//! Settings additions over `deadlock_core::diagnostics` (wave 4):
//!
//! - Network > Troubleshooting: DMM's checklist (network, internet, DNS, each service, the saved
//!   catalog, Windows TCP auto-tuning), each row with its verdict, time, address and "How to
//!   fix" steps, Copy results and Run again. Grimoire has none.
//! - Storage > Saved catalog: Grimoire's "Mod database cache" card (count, last sync, size, Sync
//!   now) and DMM's "Wipe local catalog" with its confirm. Installed mods are not touched.

use std::collections::BTreeSet;

use deadlock_core::diagnostics::{self, CatalogStatus, Check, Status};
use rune::{Color, El, Tx, col, row, text};

use super::i18n::{fill, tr};
use super::{button, detail, disabled, section, setting_row, trailing};
use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

#[derive(Default)]
pub struct DiagW4 {
    pub job: Job<Vec<Check>>,
    pub checks: Option<Vec<Check>>,
    /// Rows whose "How to fix" is open.
    pub open: BTreeSet<String>,
    pub catalog: Option<CatalogStatus>,
    pub wipe_ask: bool,
    pub wipe_job: Job<bool>,
    /// Scenes only: show one wave 4 section alone, so a snapshot needs no scrolling.
    pub scene_only: Option<&'static str>,
}

/// The section a scene asked for, alone.
pub fn scene_content(s: &State, u: &Ui) -> Option<El> {
    let c = col().w_full().gap(super::LIST_GAP);
    Some(match s.settings.w4.scene_only? {
        "troubleshoot" => c.child(section_el(s, u)),
        "catalog" => c.child(catalog_section(s, u)),
        "logs" => c.child(section(u, "Logs", super::logs_w4::rows(s, u))).child(section(
            u,
            "Report a problem",
            vec![super::logs_w4::full_log_row(u)],
        )),
        "fun" => c.child(super::misc_w4::fun_section(s, u)),
        "launch" => c.child(section(
            u,
            "Launch options",
            vec![super::misc_w4::known_options(s, u), super::misc_w4::vanilla_row(u)],
        )),
        _ => return None,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A check's name.
pub fn label(id: &str) -> String {
    match id {
        "network" => tr("Network").to_string(),
        "internet" => tr("Internet connection").to_string(),
        "dns" => tr("Name resolution (DNS)").to_string(),
        "catalog" => tr("Saved catalog").to_string(),
        "tcpAutoTuning" => tr("Windows TCP auto-tuning").to_string(),
        other => diagnostics::SERVICES
            .iter()
            .find(|s| s.id == other)
            .map(|s| tr(s.label).to_string())
            .unwrap_or_else(|| other.to_string()),
    }
}

/// A verdict key in words.
pub fn message(check: &Check, catalog: Option<&CatalogStatus>) -> String {
    let host = check.host.clone().unwrap_or_default();
    let count = catalog.map_or(0, |c| c.entries).to_string();
    match check.message.as_str() {
        "online" => tr("Connected").to_string(),
        "offline" if check.id == "network" => tr("Offline mode is on").to_string(),
        "offline" => tr("No service answered").to_string(),
        "reachable" => tr("Reachable").to_string(),
        "slow" => tr("Reachable, but answering slowly").to_string(),
        "timeout" => tr("No answer in time").to_string(),
        "unreachable" => fill(tr("{} is not reachable"), &[&host]),
        "blocked" => tr("Access denied").to_string(),
        "rateLimited" => tr("Too many requests").to_string(),
        "serverError" => tr("Server error").to_string(),
        "httpError" => tr("Unexpected answer").to_string(),
        "dnsFailed" => tr("Internet works, but names can't be resolved").to_string(),
        "dnsHostFailed" => fill(tr("{} can't be resolved. This is a DNS problem, not a blocked connection"), &[&host]),
        "catalogReady" => fill(tr("{} mods saved, browsing works offline"), &[&count]),
        "catalogStale" => fill(tr("{} mods saved, but the catalog is out of date"), &[&count]),
        "catalogEmpty" => tr("No mods saved yet").to_string(),
        "autoTuningOn" => tr("Receive window auto-tuning is on").to_string(),
        "autoTuningRestricted" => {
            tr("Auto-tuning is limited, which slows downloads from distant servers").to_string()
        }
        "skipped" => tr("Not checked on this system").to_string(),
        other => other.to_string(),
    }
}

/// A fix key in words (DMM's steps, pointed at Sigil's own settings).
pub fn fix(key: &str, host: Option<&str>) -> String {
    match key {
        "checkConnection" => tr("Check that your network cable or Wi-Fi is connected and airplane mode is off.").to_string(),
        "restartRouter" => tr("Restart your router or modem and try again.").to_string(),
        "disableVpnFirewall" => tr("Turn off your VPN, firewall or antivirus web shield for a moment to see whether it blocks Sigil.").to_string(),
        "checkProxy" => tr("A proxy is set in Settings > Network. Make sure it runs, or clear it.").to_string(),
        "changeDns" => tr("Set your DNS server to a public one such as 1.1.1.1 or 8.8.8.8, in your system's network settings or your router.").to_string(),
        "flushDnsCache" => tr("Clear the DNS cache: run \"ipconfig /flushdns\" on Windows, or restart the resolver on Linux and macOS.").to_string(),
        "fixInternetFirst" => tr("This depends on your internet connection. Fix the internet check above first.").to_string(),
        "waitAndRetry" => tr("Wait a few minutes, then run the checks again.").to_string(),
        "tryOtherNetwork" => tr("Try another network, for example a phone hotspot.").to_string(),
        "checkService" => tr("Open the site in your browser. If it does not load there either, the service has problems.").to_string(),
        "allowFirewall" => fill(tr("Allow {} in your firewall, antivirus or router."), &[host.unwrap_or("")]),
        "syncCatalog" => tr("Open Browse so the catalog syncs, or press Sync now under Settings > Storage.").to_string(),
        "restartApp" => tr("Restart Sigil. If it keeps happening, send a report from Logs and support with these results.").to_string(),
        "enableAutoTuning" => tr("Run this command in an administrator terminal, then restart any download that still runs:").to_string(),
        "turnOffOffline" => tr("Turn off Offline mode at the top of this page.").to_string(),
        other => other.to_string(),
    }
}

pub fn run(st: &mut State) {
    let data = crate::data::data_dir();
    st.settings.w4.checks = None;
    st.settings.w4.open.clear();
    if st.demo || rune::is_headless() {
        st.settings.w4.checks = Some(demo_checks());
        return;
    }
    let wake = st.wake.clone();
    st.settings.w4.job.start(wake, move || Ok(diagnostics::run(&data, now())));
}

pub fn read_catalog(st: &mut State) {
    if st.demo || rune::is_headless() {
        st.settings.w4.catalog = Some(CatalogStatus { entries: 18_420, synced_at: now() - 3_600, bytes: 9_800_000 });
        return;
    }
    st.settings.w4.catalog = Some(diagnostics::catalog_status(&crate::data::data_dir()));
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.settings.w4.job.try_take() {
        match result {
            Ok(checks) => {
                // Failed rows open their fixes at once, as DMM's dialog does.
                st.settings.w4.open = checks
                    .iter()
                    .filter(|c| matches!(c.status, Status::Error | Status::Warning) && !c.fixes.is_empty())
                    .map(|c| c.id.clone())
                    .collect();
                st.settings.w4.checks = Some(checks);
            }
            Err(error) => super::say(st, error, true),
        }
        changed = true;
    }
    if let Some(result) = st.settings.w4.wipe_job.try_take() {
        match result {
            Ok(_) => {
                read_catalog(st);
                st.browse_view.mirror_summary.clear();
                super::say(st, tr("Saved catalog wiped. Browse fills it again on its next sync."), false);
            }
            Err(error) => super::say(st, error, true),
        }
        changed = true;
    }
    changed
}

fn tone(u: &Ui, status: Status) -> (Color, &'static rune::Icon) {
    match status {
        Status::Ok => (u.teal, &icons::CIRCLE_CHECK),
        Status::Warning => (u.amber, &icons::TRIANGLE_ALERT),
        Status::Error => (u.rose, &icons::CIRCLE_X),
        Status::Skipped => (u.faint, &icons::MINUS),
    }
}

/// The overall line under the list (DMM's verdict).
pub fn verdict(checks: &[Check]) -> &'static str {
    let errors = checks.iter().filter(|c| c.status == Status::Error).count();
    let warnings = checks.iter().filter(|c| c.status == Status::Warning).count();
    match (errors, warnings) {
        (0, 0) => "Everything looks fine. The problem was probably brief, so try again.",
        (0, _) => "Mostly working, but some checks need attention. See the steps above.",
        _ => "Some checks failed. Follow the steps above, then run the checks again.",
    }
}

pub fn section_el(s: &State, u: &Ui) -> El {
    let w = &s.settings.w4;
    let running = w.job.is_running();
    let mut rows = vec![setting_row(
        u,
        "settings-troubleshoot",
        "Connection checklist",
        detail(
            u,
            if running {
                "Running checks"
            } else {
                "Mods, profiles or stats won't load? See which connection or service fails, and how to fix it."
            },
        ),
        trailing(vec![
            disabled(
                button(u, "settings-troubleshoot-copy", "Copy results", ActionKind::Ghost).on_press_tx(
                    |st: &mut State, tx: &mut Tx| {
                        if let Some(checks) = &st.settings.w4.checks {
                            tx.copy(format!("{}\n{}", diagnostics::machine_line(), diagnostics::report(checks)));
                            super::say(st, tr("Results copied."), false);
                        }
                    },
                ),
                w.checks.is_none(),
            ),
            disabled(
                button(
                    u,
                    "settings-troubleshoot-run",
                    if running { "Checking" } else if w.checks.is_some() { "Run again" } else { "Run checks" },
                    ActionKind::Secondary,
                )
                .on_press(run),
                running,
            ),
        ]),
    )];
    // The graphics stack, for a black screen or a missing 3D view: the same line the reports
    // carry, so a player can read it without sending anything.
    rows.push(
        row()
            .key("settings-troubleshoot-graphics")
            .w_full()
            .py(10.0)
            .gap(16.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(tr("Graphics")).font(BODY_MED).color(u.ink))
                    .child(text(diagnostics::machine_line()).font(SMALL).color(u.muted).wrap()),
            ),
    );
    if let Some(checks) = &w.checks {
        for check in checks {
            rows.push(check_row(s, u, check));
        }
        rows.push(
            row()
                .key("settings-troubleshoot-verdict")
                .w_full()
                .py(10.0)
                .child(text(tr(verdict(checks))).font(SMALL).color(u.muted).wrap()),
        );
    }
    section(u, "Troubleshooting", rows)
}

fn check_row(s: &State, u: &Ui, check: &Check) -> El {
    let (color, glyph) = tone(u, check.status);
    let id = check.id.clone();
    let open = s.settings.w4.open.contains(&check.id);
    let mut line = message(check, s.settings.w4.catalog.as_ref());
    if let Some(ms) = check.latency_ms {
        line.push_str(&fill(tr(" ({} ms)"), &[&ms.to_string()]));
    }
    let has_fixes = !check.fixes.is_empty();
    let mut head = row()
        .key(format!("settings-check-{}", check.id))
        .w_full()
        .min_h(44.0)
        .py(6.0)
        .gap(12.0)
        .center_y()
        .child(rune::icon(glyph, 16.0).color(color).a11y_hidden())
        .child(text(label(&check.id)).font(BODY_MED).color(u.ink).w(190.0))
        .child(text(line).font(SMALL).color(if check.status == Status::Ok { u.muted } else { color }).grow(1.0).min_w(0.0).wrap());
    if has_fixes {
        head = head
            .child(
                text(tr(if open { "Hide steps" } else { "How to fix" }))
                    .font(SMALL)
                    .color(u.accent),
            )
            .focusable(true)
            .role("button")
            .checked(open)
            .label(fill(tr("How to fix {}"), &[&label(&check.id)]))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if !st.settings.w4.open.remove(&id) {
                    st.settings.w4.open.insert(id.clone());
                }
            });
    }
    let mut c = col().w_full().border_sides([0.0, 0.0, 1.0, 0.0], u.hair).child(head);
    if open && has_fixes {
        let mut steps = col().w_full().pl(28.0).pb(10.0).gap(6.0);
        for (index, key) in check.fixes.iter().enumerate() {
            steps = steps.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(text(format!("{}.", index + 1)).font(SMALL).color(u.faint))
                    .child(text(fix(key, check.host.as_deref())).font(SMALL).color(u.prose).grow(1.0).min_w(0.0).wrap()),
            );
            if key == "enableAutoTuning" {
                steps = steps.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(text(diagnostics::ENABLE_AUTOTUNING_COMMAND).font(MONO).color(u.ink).selectable(true).grow(1.0))
                        .child(button(u, "settings-check-copy-command", "Copy command", ActionKind::Ghost).on_press_tx(
                            |st: &mut State, tx: &mut Tx| {
                                tx.copy(diagnostics::ENABLE_AUTOTUNING_COMMAND.to_string());
                                super::say(st, tr("Command copied."), false);
                            },
                        )),
                );
            }
        }
        if let Some(url) = check.url.clone() {
            steps = steps.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(text(url.clone()).font(MONO).color(u.muted).truncate().min_w(0.0).shrink())
                    .child(
                        button(u, &format!("settings-check-copy-{}", check.id), "Copy address", ActionKind::Ghost)
                            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                tx.copy(url.clone());
                                super::say(st, tr("Address copied"), false);
                            }),
                    ),
            );
        }
        c = c.child(steps);
    }
    c
}

/// Storage > Saved catalog (Grimoire's cache card, DMM's wipe).
pub fn catalog_section(s: &State, u: &Ui) -> El {
    let w = &s.settings.w4;
    let status = w.catalog.unwrap_or_default();
    let syncing = s.browse_view.mirror_sync_job.is_running();
    let line = if status.entries == 0 {
        tr("Nothing saved yet. Browse saves the catalog as you use it, so it also works offline.").to_string()
    } else {
        let when = if status.synced_at > 0 {
            super::tabs::date_label(s, status.synced_at as u64)
        } else {
            tr("never").to_string()
        };
        fill(
            tr("{} mods saved, last synced {}, {} on disk."),
            &[&status.entries.to_string(), &when, &super::size_label(status.bytes)],
        )
    };
    let mut rows = vec![setting_row(
        u,
        "settings-catalog",
        "Saved catalog",
        detail(u, &line),
        trailing(vec![
            disabled(
                button(u, "settings-catalog-sync", if syncing { "Syncing" } else { "Sync now" }, ActionKind::Secondary).on_press(
                    |st: &mut State| {
                        st.browse_view.last_sync = None;
                        crate::screens::browse::start_mirror_load(st);
                        crate::screens::browse::start_mirror_sync(st);
                        super::say(st, tr("Syncing the catalog."), false);
                    },
                ),
                syncing || s.prefs.offline,
            ),
            disabled(
                button(u, "settings-catalog-wipe", "Wipe", ActionKind::Ghost)
                    .on_press(|st: &mut State| st.settings.w4.wipe_ask = true),
                status.entries == 0 || w.wipe_job.is_running(),
            ),
        ]),
    )];
    if w.wipe_ask {
        rows.push(
            col()
                .key("settings-catalog-wipe-ask")
                .w_full()
                .padding(12.0)
                .gap(10.0)
                .radius(crate::ui::r(8.0))
                .fill(u.rose.alpha(0.06))
                .border(1.0, u.rose.alpha(0.3))
                .child(text(tr("Wipe the saved catalog?")).font(BODY_MED).color(u.ink))
                .child(
                    text(tr("Browse shows nothing offline until the next sync fills it again. Installed mods and profiles stay."))
                        .font(BODY)
                        .color(u.muted)
                        .wrap(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(button(u, "settings-catalog-wipe-cancel", "Cancel", ActionKind::Ghost).on_press(
                            |st: &mut State| st.settings.w4.wipe_ask = false,
                        ))
                        .child(button(u, "settings-catalog-wipe-go", "Wipe catalog", ActionKind::Danger).on_press(wipe)),
                ),
        );
    }
    section(u, "Mod catalog", rows)
}

/// Stop the sync, drop the copy in memory, delete the folder on a worker (app data only).
fn wipe(st: &mut State) {
    st.settings.w4.wipe_ask = false;
    if st.demo || rune::is_headless() {
        st.settings.w4.catalog = Some(CatalogStatus::default());
        return;
    }
    st.browse_view.mirror = None;
    st.browse_view.mirror_dirty = false;
    st.browse_view.download_counts.clear();
    let data = crate::data::data_dir();
    let wake = st.wake.clone();
    st.settings.w4.wipe_job.start(wake, move || diagnostics::wipe_catalog(&data).map_err(|e| e.to_string()));
}

/// A run with every kind of row, for scenes and demo data.
pub fn demo_checks() -> Vec<Check> {
    let mut out = vec![diagnostics::evaluate_network(false)];
    let services: Vec<Check> = diagnostics::SERVICES
        .iter()
        .enumerate()
        .map(|(i, service)| {
            let outcome = match i {
                2 => diagnostics::Outcome::Response { status: 200, latency_ms: 4_200 },
                4 => diagnostics::Outcome::Timeout { latency_ms: 8_000 },
                _ => diagnostics::Outcome::Response { status: 200, latency_ms: 90 + i as u64 * 30 },
            };
            diagnostics::evaluate_probe(service, outcome, true, false)
        })
        .collect();
    out.push(diagnostics::evaluate_internet(&services, false));
    out.push(diagnostics::evaluate_dns(5, 5, true));
    out.extend(services);
    out.push(diagnostics::evaluate_catalog(CatalogStatus { entries: 18_420, synced_at: 0, bytes: 1 }, now(), true));
    out.push(diagnostics::evaluate_tcp_autotuning(Some(diagnostics::AutoTuning::Restricted)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verdict_and_fix_has_words() {
        for check in demo_checks() {
            assert_ne!(message(&check, None), check.message, "{}", check.message);
            assert!(!label(&check.id).is_empty());
            for key in &check.fixes {
                assert_ne!(fix(key, Some("example.org")), *key, "{key}");
            }
        }
        for key in [
            "checkConnection", "restartRouter", "disableVpnFirewall", "checkProxy", "changeDns", "flushDnsCache",
            "fixInternetFirst", "waitAndRetry", "tryOtherNetwork", "checkService", "allowFirewall", "syncCatalog",
            "restartApp", "enableAutoTuning", "turnOffOffline",
        ] {
            assert_ne!(fix(key, None), key);
        }
    }

    #[test]
    fn the_verdict_follows_the_worst_row() {
        let ok = diagnostics::evaluate_network(false);
        let bad = diagnostics::evaluate_network(true);
        assert!(verdict(std::slice::from_ref(&ok)).starts_with("Everything"));
        assert!(verdict(&[ok, bad]).starts_with("Some checks failed"));
    }
}
