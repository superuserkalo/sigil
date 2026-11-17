//! The offline switch every HTTP path in the core goes through.
//!
//! Offline mode is one process-wide flag. The desktop app sets it from Settings; a child
//! process (the `dah` CLI, the agent host) inherits it through `DAH_OFFLINE=1`. Every
//! `ureq::Agent` the core builds carries [`gate`] as middleware, so a request made while
//! offline fails before it resolves a name or opens a socket.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by `DAH_OFFLINE=1` in the environment or by [`set_offline`].
static OFFLINE: AtomicBool = AtomicBool::new(false);

/// The message every blocked request carries.
pub const OFFLINE_MESSAGE: &str = "Offline mode is on. Turn it off in Settings to go online.";

/// The environment variable a child process reads.
pub const OFFLINE_ENV: &str = "DAH_OFFLINE";

pub fn set_offline(offline: bool) {
    OFFLINE.store(offline, Ordering::SeqCst);
}

#[cfg(test)]
thread_local! {
    /// Lets a test go offline on its own thread without touching the process flag.
    static TEST_OFFLINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when network access is switched off for this process.
pub fn is_offline() -> bool {
    #[cfg(test)]
    if TEST_OFFLINE.with(std::cell::Cell::get) {
        return true;
    }
    OFFLINE.load(Ordering::SeqCst) || env_says_offline(std::env::var(OFFLINE_ENV).ok().as_deref())
}

fn env_says_offline(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// Ok while online; the offline message otherwise. For code that talks to the network
/// without a core `ureq::Agent` (a socket, a child process).
pub fn check() -> Result<(), String> {
    if is_offline() {
        Err(OFFLINE_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

/// `ureq` middleware: refuse the request while offline mode is on.
pub fn gate(
    request: ureq::http::Request<ureq::SendBody>,
    next: ureq::middleware::MiddlewareNext,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    if is_offline() {
        return Err(ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            OFFLINE_MESSAGE,
        )));
    }
    next.handle(request)
}

/// An agent with the gate and a global timeout, for new call sites.
pub fn agent(timeout: std::time::Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .middleware(gate)
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

static PROXY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Bumped whenever the proxy, its login or its bypass list changes, so a shared agent built
/// for the old settings is built again.
static PROXY_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The proxy settings' version: compare it to rebuild a cached agent.
pub fn proxy_generation() -> u64 {
    PROXY_GENERATION.load(Ordering::Relaxed)
}

fn bump_proxy() {
    PROXY_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Route GameBanana browsing and mod downloads through an HTTP or SOCKS proxy. None goes back
/// to a direct connection (or the `HTTPS_PROXY` environment, which ureq reads by default).
pub fn set_proxy(url: Option<String>) -> Result<(), String> {
    bump_proxy();
    let url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    if let Some(url) = &url {
        validate_proxy(url)?;
    }
    if let Ok(mut slot) = PROXY.lock() {
        *slot = url;
    }
    Ok(())
}

/// Ok for `http://` or `https://` with a host and a port. SOCKS would need ureq's socks
/// feature; a user name and password go through [`set_proxy_auth`], never the address.
pub fn validate_proxy(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let rest = ["http://", "https://"]
        .iter()
        .find_map(|scheme| lower.strip_prefix(scheme))
        .ok_or("Start the proxy with http:// or https://")?;
    if rest.contains('@') {
        return Err("Put the user name and password under Proxy sign-in, not in the address".to_string());
    }
    let host_port = rest.trim_end_matches('/');
    let (host, port) = host_port.rsplit_once(':').ok_or("Add the port, like :8080")?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        return Err("The proxy needs a host and a port".to_string());
    }
    ureq::Proxy::new(url).map(|_| ()).map_err(|e| e.to_string())
}

/// The proxy for download paths, applied on top of a builder. Without one the builder keeps
/// ureq's default, which honours the environment.
pub fn with_proxy(
    builder: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>,
) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    match current_proxy() {
        Some(proxy) => builder.proxy(Some(proxy)),
        None => builder,
    }
}

/// A user name and password for the proxy. Held in memory only: the desktop app reads the
/// password from the OS keychain at startup and hands it here; it is never written to disk
/// by the core and never logged.
static PROXY_AUTH: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
/// Hosts that skip the proxy, in NO_PROXY form (`example.com`, `.lan`, `*.lan`, `*`).
static PROXY_BYPASS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Set or clear the proxy's sign-in. An empty user name clears it.
pub fn set_proxy_auth(auth: Option<(String, String)>) {
    bump_proxy();
    if let Ok(mut slot) = PROXY_AUTH.lock() {
        *slot = auth.filter(|(user, _)| !user.trim().is_empty());
    }
}

/// True when a proxy sign-in is held.
pub fn has_proxy_auth() -> bool {
    PROXY_AUTH.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

/// Set the bypass list. Entries that are not a host, `.domain`, `*.domain` or `*` are dropped.
pub fn set_proxy_bypass(hosts: Vec<String>) {
    bump_proxy();
    let clean = hosts
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| valid_bypass(h))
        .collect();
    if let Ok(mut slot) = PROXY_BYPASS.lock() {
        *slot = clean;
    }
}

/// One bypass entry: a host name, `.domain`, `*.domain` or `*`.
pub fn valid_bypass(entry: &str) -> bool {
    let body = entry
        .strip_prefix("*.")
        .or_else(|| entry.strip_prefix('.'))
        .unwrap_or(entry);
    entry == "*"
        || (!body.is_empty()
            && body.len() <= 253
            && body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == ':')
            && !body.starts_with('.')
            && !body.ends_with('.'))
}

/// Split a bypass list typed as text: commas, spaces or new lines.
pub fn parse_bypass(text: &str) -> Vec<String> {
    text.split([',', ' ', '\n', ';'])
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The proxy every core agent uses: the address from Settings with its sign-in and bypass
/// list, or None for a direct connection.
pub fn current_proxy() -> Option<ureq::Proxy> {
    let url = PROXY.lock().ok().and_then(|slot| slot.clone())?;
    let auth = PROXY_AUTH.lock().ok().and_then(|slot| slot.clone());
    let bypass = PROXY_BYPASS.lock().map(|slot| slot.clone()).unwrap_or_default();
    build_proxy(&url, auth.as_ref(), &bypass).ok()
}

/// Build a proxy from an `http(s)://host:port` address, an optional sign-in and a bypass list.
pub fn build_proxy(
    url: &str,
    auth: Option<&(String, String)>,
    bypass: &[String],
) -> Result<ureq::Proxy, String> {
    validate_proxy(url)?;
    let lower = url.trim().to_ascii_lowercase();
    let (protocol, rest) = if let Some(rest) = lower.strip_prefix("https://") {
        (ureq::ProxyProtocol::Https, rest)
    } else {
        (
            ureq::ProxyProtocol::Http,
            lower.strip_prefix("http://").unwrap_or(&lower),
        )
    };
    let host_port = rest.trim_end_matches('/');
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or("The proxy needs a host and a port")?;
    let port: u16 = port.parse().map_err(|_| "The proxy port is not a number")?;
    let mut builder = ureq::Proxy::builder(protocol).host(host).port(port);
    if let Some((user, pass)) = auth {
        builder = builder.username(user);
        if !pass.is_empty() {
            builder = builder.password(pass);
        }
    }
    for entry in bypass {
        builder = builder.no_proxy(entry);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Download routing chosen in Settings: a GameBanana file host to use in place of
/// `files.gamebanana.com`, and how many parts a large download is fetched in.
static DOWNLOAD_HOST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DOWNLOAD_PARTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Use `host` (a `*.gamebanana.com` name) for GameBanana file downloads. None is automatic.
pub fn set_download_host(host: Option<String>) {
    let host = host
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| h == "gamebanana.com" || h.ends_with(".gamebanana.com"));
    if let Ok(mut slot) = DOWNLOAD_HOST.lock() {
        *slot = host;
    }
}

pub fn download_host() -> Option<String> {
    DOWNLOAD_HOST.lock().ok().and_then(|slot| slot.clone())
}

/// Fetch large downloads in `parts` ranges at once (1 to 16; 1 is one connection).
pub fn set_download_parts(parts: u32) {
    DOWNLOAD_PARTS.store(parts.clamp(1, 16), Ordering::SeqCst);
}

pub fn download_parts() -> u32 {
    DOWNLOAD_PARTS.load(Ordering::SeqCst).clamp(1, 16)
}

/// A GameBanana file URL on the chosen host. Only `files.gamebanana.com` and the numbered
/// `filecacheNN` hosts are rewritten; every other URL comes back unchanged.
pub fn route_download(url: &str) -> String {
    match download_host() {
        Some(host) => rewrite_file_host(url, &host),
        None => url.to_string(),
    }
}

/// [`route_download`] with the host passed in.
pub fn rewrite_file_host(url: &str, host: &str) -> String {
    let Some(rest) = url.strip_prefix("https://") else {
        return url.to_string();
    };
    let (current, path) = rest.split_once('/').unwrap_or((rest, ""));
    let current_lower = current.to_ascii_lowercase();
    let is_file_host = current_lower == "files.gamebanana.com"
        || (current_lower.starts_with("filecache") && current_lower.ends_with(".gamebanana.com"));
    if !is_file_host || host.is_empty() {
        return url.to_string();
    }
    format!("https://{host}/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_urls_need_a_scheme_host_and_port() {
        assert!(validate_proxy("http://127.0.0.1:7890").is_ok());
        assert!(validate_proxy("https://proxy.lan:8443/").is_ok());
        assert!(validate_proxy("http://user:pw@proxy.lan:1080").is_err());
        assert!(validate_proxy("socks5://proxy.lan:1080").is_err());
        assert!(validate_proxy("127.0.0.1:7890").is_err());
        assert!(validate_proxy("http://proxy.lan").is_err());
        assert!(validate_proxy("ftp://proxy.lan:21").is_err());
    }

    #[test]
    fn proxies_carry_a_sign_in_and_a_bypass_list() {
        let auth = ("me".to_string(), "p@ss:word".to_string());
        let proxy = build_proxy("http://proxy.lan:3128", Some(&auth), &["localhost".into(), ".lan".into()]).unwrap();
        assert_eq!(proxy.host(), "proxy.lan");
        assert_eq!(proxy.port(), 3128);
        assert_eq!(proxy.username(), Some("me"));
        assert!(proxy.password().is_some());
        let lan: ureq::http::Uri = "https://nas.lan/x".parse().unwrap();
        let banana: ureq::http::Uri = "https://gamebanana.com/x".parse().unwrap();
        assert!(proxy.is_no_proxy(&lan));
        assert!(!proxy.is_no_proxy(&banana));
        assert!(build_proxy("http://proxy.lan", None, &[]).is_err());
    }

    #[test]
    fn bypass_entries_are_hosts_or_wildcards() {
        assert!(valid_bypass("localhost"));
        assert!(valid_bypass("*.lan"));
        assert!(valid_bypass(".example.com"));
        assert!(valid_bypass("*"));
        assert!(valid_bypass("127.0.0.1"));
        assert!(!valid_bypass("http://x"));
        assert!(!valid_bypass("a b"));
        assert_eq!(parse_bypass("localhost, *.lan\n10.0.0.1"), vec!["localhost", "*.lan", "10.0.0.1"]);
    }

    #[test]
    fn only_file_hosts_are_rewritten() {
        let host = "fs2.gamebanana.com";
        assert_eq!(
            rewrite_file_host("https://files.gamebanana.com/mods/a.zip", host),
            "https://fs2.gamebanana.com/mods/a.zip"
        );
        assert_eq!(
            rewrite_file_host("https://filecache12.gamebanana.com/mods/a.zip", host),
            "https://fs2.gamebanana.com/mods/a.zip"
        );
        assert_eq!(
            rewrite_file_host("https://gamebanana.com/dl/123", host),
            "https://gamebanana.com/dl/123"
        );
        assert_eq!(rewrite_file_host("http://files.gamebanana.com/a", host), "http://files.gamebanana.com/a");
    }

    #[test]
    fn env_values_that_mean_offline() {
        assert!(env_says_offline(Some("1")));
        assert!(env_says_offline(Some(" true ")));
        assert!(!env_says_offline(Some("0")));
        assert!(!env_says_offline(Some("")));
        assert!(!env_says_offline(None));
    }

    #[test]
    fn the_gate_blocks_before_any_connection() {
        // Port 9 on a reserved TEST-NET address: were the gate open, this would time out
        // or be refused, never fail with the offline message.
        // The middleware runs on the calling thread, so a thread-local switch is enough and
        // other tests running in parallel stay online.
        TEST_OFFLINE.with(|f| f.set(true));
        let result = agent(std::time::Duration::from_secs(2))
            .get("http://192.0.2.1:9/")
            .call();
        assert!(check().is_err());
        TEST_OFFLINE.with(|f| f.set(false));
        let error = result.expect_err("offline requests fail").to_string();
        assert!(error.contains("Offline mode"), "{error}");
    }
}
