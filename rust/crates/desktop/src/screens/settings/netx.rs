//! Settings > Network additions: a proxy sign-in kept in the OS keychain, hosts that skip the
//! proxy, a connection test, and connection diagnostics for every host Sigil talks to.
//!
//! The proxy password lives only in the keychain and in memory (`deadlock_core::net`); the
//! user name sits in options.json. Neither is logged; diagnostic lines name hosts only.

use std::time::{Duration, Instant};

use rune::{El, TextEdit, row, text};

use super::i18n::{fill, tr};
use super::{button, detail, disabled, field, keychain, say, section, setting_row, stacked_row};
use super::options;
use crate::app::State;
use crate::jobs::Job;
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

pub const KEYCHAIN_ACCOUNT: &str = "proxy";

/// What connection diagnostics probe: a name, the URL, and whether it is optional.
pub const PROBES: [(&str, &str); 6] = [
    ("GameBanana", "https://gamebanana.com/apiv11/Mod/Index?_nPerpage=1&_nPage=1&_aFilters[Generic_Game]=20948"),
    ("GameBanana files", "https://files.gamebanana.com/"),
    ("deadlock-api", "https://api.deadlock-api.com/v1/assets/heroes?only_active=true"),
    ("GitHub", "https://api.github.com/"),
    ("GitHub files", "https://raw.githubusercontent.com/"),
    ("Deadworks", "https://api.deadworks.net/"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct DiagRow {
    pub name: String,
    pub host: String,
    pub ms: Option<u128>,
    /// None when the host answered.
    pub problem: Option<String>,
    pub slow: bool,
}

#[derive(Default)]
pub struct Net {
    pub user: TextEdit,
    pub pass: TextEdit,
    pub bypass: TextEdit,
    pub has_secret: Option<bool>,
    pub test_job: Job<u128>,
    pub test: Option<Result<u128, String>>,
    pub diag_job: Job<Vec<DiagRow>>,
    pub diag: Option<Vec<DiagRow>>,
}

/// Fill the fields from options when the Network tab opens.
pub fn on_open(st: &mut State) {
    let o = options::get();
    let n = &mut st.settings.extra.net;
    n.user = TextEdit::new(o.proxy_user.clone().unwrap_or_default());
    n.pass = TextEdit::new("").secure();
    n.bypass = TextEdit::new(o.proxy_bypass.join(", "));
    n.has_secret = Some(o.proxy_user.is_some() && deadlock_core::net::has_proxy_auth());
}

/// Startup: hand the core the bypass list, and read the proxy password from the keychain on
/// a worker (a locked keychain may ask the user).
pub fn on_startup() {
    let o = options::get();
    deadlock_core::net::set_proxy_bypass(o.proxy_bypass.clone());
    if let Some(user) = o.proxy_user.clone()
        && !rune::is_headless()
    {
        std::thread::spawn(move || {
            let pass = keychain::get(KEYCHAIN_ACCOUNT).unwrap_or_default();
            deadlock_core::net::set_proxy_auth(Some((user, pass)));
        });
    }
}

pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let n = &mut st.settings.extra.net;
    if let Some(result) = n.test_job.try_take() {
        n.test = Some(result);
        changed = true;
    }
    if let Some(Ok(rows)) = n.diag_job.try_take() {
        n.diag = Some(rows);
        changed = true;
    }
    changed
}

fn save_auth(st: &mut State) {
    let user = st.settings.extra.net.user.text.trim().to_string();
    let pass = st.settings.extra.net.pass.text.clone();
    if user.is_empty() {
        return clear_auth(st);
    }
    if !pass.is_empty()
        && let Err(error) = keychain::set(KEYCHAIN_ACCOUNT, &pass)
    {
        return say(st, fill(tr("The keychain refused the password: {}"), &[&error]), true);
    }
    let stored = if pass.is_empty() { keychain::get(KEYCHAIN_ACCOUNT).unwrap_or_default() } else { pass };
    deadlock_core::net::set_proxy_auth(Some((user.clone(), stored)));
    options::edit(st, |o| o.proxy_user = Some(user));
    st.settings.extra.net.pass = TextEdit::new("").secure();
    st.settings.extra.net.has_secret = Some(true);
    say(st, tr("Saved. The password is in your keychain."), false);
}

fn clear_auth(st: &mut State) {
    let _ = keychain::delete(KEYCHAIN_ACCOUNT);
    deadlock_core::net::set_proxy_auth(None);
    options::edit(st, |o| o.proxy_user = None);
    st.settings.extra.net.user = TextEdit::new("");
    st.settings.extra.net.pass = TextEdit::new("").secure();
    st.settings.extra.net.has_secret = Some(false);
    say(st, tr("Proxy sign-in removed."), false);
}

fn save_bypass(st: &mut State) {
    let list = deadlock_core::net::parse_bypass(&st.settings.extra.net.bypass.text);
    let bad: Vec<&String> = list.iter().filter(|h| !deadlock_core::net::valid_bypass(h)).collect();
    if let Some(first) = bad.first() {
        return say(st, fill(tr("{} is not a host name."), &[first]), true);
    }
    deadlock_core::net::set_proxy_bypass(list.clone());
    options::edit(st, |o| o.proxy_bypass = list);
    say(st, tr("Saved."), false);
}

/// One request to `url` through the current proxy: the time to an answer, or the reason.
pub fn probe(url: &str) -> Result<u128, String> {
    let agent = deadlock_core::net::with_proxy(deadlock_core::net::builder())
        .middleware(deadlock_core::net::gate)
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .new_agent();
    let started = Instant::now();
    let response = agent.head(url).header("User-Agent", "Sigil").call().map_err(|e| explain(&e.to_string()))?;
    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    match status {
        403 => Err(tr("Blocked (HTTP 403)").to_string()),
        429 => Err(tr("Rate limited (HTTP 429)").to_string()),
        500..=599 => Err(fill(tr("Server error (HTTP {})"), &[&status.to_string()])),
        _ => Ok(ms),
    }
}

/// A plain reason for a network error.
pub fn explain(error: &str) -> String {
    let e = error.to_ascii_lowercase();
    if e.contains("offline mode") {
        tr("Offline mode is on.").to_string()
    } else if e.contains("dns") || e.contains("resolve") || e.contains("lookup") || e.contains("nodename") {
        tr("The name did not resolve (DNS).").to_string()
    } else if e.contains("timed out") || e.contains("timeout") {
        tr("Timed out.").to_string()
    } else if e.contains("refused") {
        tr("Connection refused.").to_string()
    } else if e.contains("proxy") {
        tr("The proxy refused the connection.").to_string()
    } else {
        tr("Unreachable.").to_string()
    }
}

pub fn proxy_rows(s: &State, u: &Ui) -> Vec<El> {
    let o = options::get();
    let n = &s.settings.extra.net;
    let stored = o.proxy_user.is_some() && n.has_secret != Some(false);
    let typed_user = n.user.text.trim().to_string();
    let auth_line = if stored {
        fill(tr("Signed in as {}. Credentials stored securely in your keychain."), &[o.proxy_user.as_deref().unwrap_or("")])
    } else {
        tr("For a proxy that asks for a user name and password. The password goes to your keychain, never a file.").to_string()
    };
    let auth = stacked_row(
        u,
        "settings-proxy-auth",
        "Proxy sign-in",
        &auth_line,
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                field(
                    rune::text_input(&n.user, BODY)
                        .key("settings-proxy-user")
                        .label(tr("User name"))
                        .single_line()
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder(tr("User name"), u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.settings.extra.net.user),
                    u,
                )
                .w(180.0),
            )
            .child(
                field(
                    rune::text_input(&n.pass, BODY)
                        .key("settings-proxy-pass")
                        .label(tr("Password"))
                        .secure()
                        .single_line()
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder(if stored { "••••••••" } else { tr("Password") }, u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.settings.extra.net.pass),
                    u,
                )
                .grow(1.0)
                .min_w(0.0),
            )
            .child(disabled(
                button(u, "settings-proxy-auth-save", "Save", ActionKind::Secondary).on_press(save_auth),
                typed_user.is_empty() || (typed_user == o.proxy_user.clone().unwrap_or_default() && n.pass.text.is_empty()),
            ))
            .child(disabled(
                button(u, "settings-proxy-auth-clear", "Remove", ActionKind::Ghost).on_press(clear_auth),
                o.proxy_user.is_none(),
            )),
    );
    let bypass_saved = o.proxy_bypass.join(", ");
    let bypass = stacked_row(
        u,
        "settings-proxy-bypass",
        "Skip the proxy for",
        "Hosts, separated by commas. .lan or *.lan covers every name under it.",
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                field(
                    rune::text_input(&n.bypass, MONO)
                        .key("settings-proxy-bypass-input")
                        .label(tr("Skip the proxy for"))
                        .single_line()
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder("localhost, 127.0.0.1, .lan", u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.settings.extra.net.bypass),
                    u,
                )
                .grow(1.0)
                .min_w(0.0),
            )
            .child(disabled(
                button(u, "settings-proxy-bypass-save", "Save", ActionKind::Secondary).on_press(save_bypass),
                deadlock_core::net::parse_bypass(&n.bypass.text).join(", ") == bypass_saved,
            )),
    );
    let testing = n.test_job.is_running();
    let test_line = match &n.test {
        _ if testing => tr("Connecting to GameBanana").to_string(),
        None => tr("Connects to GameBanana with the saved settings.").to_string(),
        Some(Ok(ms)) => fill(tr("Connected in {} ms."), &[&ms.to_string()]),
        Some(Err(e)) => e.clone(),
    };
    let test = setting_row(
        u,
        "settings-proxy-test",
        "Test connection",
        text(test_line)
            .font(SMALL)
            .color(if matches!(n.test, Some(Err(_))) { u.amber } else { u.muted })
            .truncate(),
        disabled(
            button(u, "settings-proxy-test-run", "Test", ActionKind::Secondary).on_press(|st: &mut State| {
                if st.demo || rune::is_headless() {
                    st.settings.extra.net.test = Some(Ok(142));
                    return;
                }
                let wake = st.wake.clone();
                st.settings.extra.net.test_job.start(wake, || probe("https://gamebanana.com/"));
            }),
            testing || s.prefs.offline,
        ),
    );
    vec![auth, bypass, test]
}

fn run_diagnostics(st: &mut State) {
    if st.demo || rune::is_headless() {
        st.settings.extra.net.diag = Some(demo_rows());
        return;
    }
    let wake = st.wake.clone();
    st.settings.extra.net.diag_job.start(wake, || {
        let rows = std::thread::scope(|scope| {
            let handles: Vec<_> = PROBES
                .iter()
                .map(|(name, url)| {
                    scope.spawn(move || {
                        let host = deadlock_core::mod_sources::origin_of(url)
                            .unwrap_or_default()
                            .trim_start_matches("https://")
                            .to_string();
                        let result = probe(url);
                        DiagRow {
                            name: (*name).to_string(),
                            host,
                            slow: matches!(result, Ok(ms) if ms > 3000),
                            ms: result.as_ref().ok().copied(),
                            problem: result.err(),
                        }
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect::<Vec<_>>()
        });
        Ok(rows)
    });
}

pub fn diagnostics_section(s: &State, u: &Ui) -> El {
    let n = &s.settings.extra.net;
    let running = n.diag_job.is_running();
    let mut rows = vec![setting_row(
        u,
        "settings-diagnostics",
        "Connection diagnostics",
        detail(
            u,
            if running {
                "Checking each host"
            } else {
                "Checks every host Sigil uses, through your proxy when one is set."
            },
        ),
        disabled(
            button(u, "settings-diagnostics-run", if running { "Checking" } else { "Run" }, ActionKind::Secondary)
                .on_press(run_diagnostics),
            running || s.prefs.offline,
        ),
    )];
    for (index, d) in n.diag.iter().flatten().enumerate() {
        let (line, warn) = match (&d.problem, d.ms) {
            (Some(p), _) => (p.clone(), true),
            (None, Some(ms)) if d.slow => (fill(tr("Slow: {} ms"), &[&ms.to_string()]), true),
            (None, Some(ms)) => (fill(tr("{} ms"), &[&ms.to_string()]), false),
            _ => (String::new(), false),
        };
        rows.push(
            row()
                .key(format!("settings-diagnostics-{index}"))
                .w_full()
                .h(44.0)
                .gap(16.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(d.name.clone()).font(BODY_MED).color(u.ink).w(140.0))
                .child(text(d.host.clone()).font(MONO).color(u.muted).grow(1.0).min_w(0.0).truncate())
                .child(text(line).font(SMALL).color(if warn { u.amber } else { u.muted })),
        );
    }
    section(u, "Diagnostics", rows)
}

fn demo_rows() -> Vec<DiagRow> {
    PROBES
        .iter()
        .enumerate()
        .map(|(i, (name, url))| DiagRow {
            name: (*name).to_string(),
            host: deadlock_core::mod_sources::origin_of(url).unwrap_or_default().trim_start_matches("https://").to_string(),
            ms: (i != 5).then_some(80 + i as u128 * 37),
            problem: (i == 5).then(|| tr("Timed out.").to_string()),
            slow: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_plain_reasons() {
        assert_eq!(explain("io: failed to lookup address information"), "The name did not resolve (DNS).");
        assert_eq!(explain("Timeout: global"), "Timed out.");
        assert_eq!(explain("Connection refused (os error 61)"), "Connection refused.");
        assert_eq!(explain("weird"), "Unreachable.");
    }

    #[test]
    fn every_probe_is_https_on_an_allowlisted_host() {
        for (_, url) in PROBES {
            assert!(url.starts_with("https://"));
            let host = deadlock_core::mod_sources::origin_of(url).unwrap();
            assert!(
                ["gamebanana.com", "deadlock-api.com", "github.com", "githubusercontent.com", "deadworks.net"]
                    .iter()
                    .any(|h| host.ends_with(h)),
                "{host}"
            );
        }
    }
}
