//! Subscription sign-in for Codex (ChatGPT) and Grok.
//!
//! The flows are the first-party OAuth clients their own CLIs use, read from the
//! Apache-2.0 fx source. This module never scrapes a browser session, never
//! resells an account, and never prints a token. It stores one token set per
//! provider in the app data directory with owner-only permissions.
//!
//! The API endpoints are the subscription endpoints, not the public API:
//! Codex uses `chatgpt.com/backend-api/codex/responses`, Grok uses
//! `cli-chat-proxy.grok.com/v1/responses`. Both speak the Responses shape.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::util::{now_iso, write_atomic};

pub const TOKEN_SCHEMA: u32 = 1;
/// Refresh when the access token has less than this many seconds left.
const REFRESH_SKEW_SECONDS: i64 = 120;
const HTTP_TIMEOUT_SECONDS: u64 = 30;

pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const GROK_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// A subscription provider the workbench can sign in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Grok,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Codex, Provider::Grok];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" | "chatgpt" | "openai" => Some(Provider::Codex),
            "grok" | "xai" | "x" => Some(Provider::Grok),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::Codex => "codex",
            Provider::Grok => "grok",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Codex => "ChatGPT subscription (Codex)",
            Provider::Grok => "Grok subscription (SuperGrok or X Premium)",
        }
    }

    pub fn client_id(self) -> &'static str {
        match self {
            Provider::Codex => CODEX_CLIENT_ID,
            Provider::Grok => GROK_CLIENT_ID,
        }
    }

    pub fn issuer(self) -> &'static str {
        match self {
            Provider::Codex => "https://auth.openai.com",
            Provider::Grok => "https://auth.x.ai",
        }
    }

    pub fn authorize_path(self) -> &'static str {
        match self {
            Provider::Codex => "/oauth/authorize",
            Provider::Grok => "/oauth2/authorize",
        }
    }

    pub fn token_url(self) -> String {
        match self {
            Provider::Codex => "https://auth.openai.com/oauth/token".to_string(),
            Provider::Grok => "https://auth.x.ai/oauth2/token".to_string(),
        }
    }

    pub fn revoke_url(self) -> String {
        match self {
            Provider::Codex => "https://auth.openai.com/oauth/revoke".to_string(),
            Provider::Grok => "https://auth.x.ai/oauth2/revoke".to_string(),
        }
    }

    pub fn scopes(self) -> &'static str {
        match self {
            Provider::Codex => {
                "openid profile email offline_access api.connectors.read api.connectors.invoke"
            }
            Provider::Grok => "openid profile email offline_access grok-cli:access api:access",
        }
    }

    /// Loopback callback path. Codex uses `/auth/callback`, Grok `/callback`.
    pub fn callback_path(self) -> &'static str {
        match self {
            Provider::Codex => "/auth/callback",
            Provider::Grok => "/callback",
        }
    }

    /// Ports to try for the loopback listener. `0` means an ephemeral port.
    pub fn ports(self) -> &'static [u16] {
        match self {
            Provider::Codex => &[1455, 1457, 0],
            Provider::Grok => &[0],
        }
    }

    /// Host name in the redirect URI. Codex registers `localhost`; Grok uses
    /// the loopback address.
    pub fn redirect_host(self) -> &'static str {
        match self {
            Provider::Codex => "localhost",
            Provider::Grok => "127.0.0.1",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Provider::Codex => "https://chatgpt.com/backend-api/codex/responses",
            Provider::Grok => "https://cli-chat-proxy.grok.com/v1/responses",
        }
    }

    /// Extra query parameters on the authorize URL.
    pub fn authorize_extra(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Provider::Codex => &[("codex_cli_simplified_flow", "true")],
            Provider::Grok => &[],
        }
    }

    pub fn redirect_uri(self, port: u16) -> String {
        format!(
            "http://{}:{}{}",
            self.redirect_host(),
            port,
            self.callback_path()
        )
    }
}

/// PKCE verifier, challenge and CSRF state for one login.
#[derive(Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl fmt::Debug for Pkce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkce")
            .field("verifier", &"[redacted]")
            .field("challenge", &self.challenge)
            .field("state", &"[redacted]")
            .finish()
    }
}

/// A stored login. Debug never prints a token.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSet {
    pub schema: u32,
    pub provider: Provider,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub expires_at_unix: i64,
    pub saved_at: String,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenSet")
            .field("provider", &self.provider)
            .field("account_id", &self.account_id)
            .field("expires_at_unix", &self.expires_at_unix)
            .field("access_token", &"[redacted]")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

impl TokenSet {
    pub fn expired(&self, now: i64) -> bool {
        self.expires_at_unix - now <= REFRESH_SKEW_SECONDS
    }

    pub fn expires_in_seconds(&self, now: i64) -> i64 {
        (self.expires_at_unix - now).max(0)
    }
}

/// One request target and the headers a subscription endpoint needs.
pub struct ProviderRequest {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
}

impl fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRequest")
            .field("endpoint", &self.endpoint)
            .field("header_names", &self.headers.iter().map(|(name, _)| name).collect::<Vec<_>>())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// PKCE and authorize URL
// ---------------------------------------------------------------------------

pub fn pkce() -> Result<Pkce> {
    let mut verifier_bytes = [0_u8; 64];
    getrandom::fill(&mut verifier_bytes)
        .map_err(|error| Error::msg(format!("Could not read OS randomness: {error}")))?;
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut state_bytes = [0_u8; 32];
    getrandom::fill(&mut state_bytes)
        .map_err(|error| Error::msg(format!("Could not read OS randomness: {error}")))?;
    let state = URL_SAFE_NO_PAD.encode(state_bytes);

    let challenge = challenge_for(&verifier);
    Ok(Pkce {
        verifier,
        challenge,
        state,
    })
}

/// S256 challenge for a verifier. Public so a test can use the RFC vector.
pub fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn authorize_url(provider: Provider, redirect_uri: &str, pkce: &Pkce) -> String {
    let mut params: Vec<(&str, String)> = vec![
        ("response_type", "code".to_string()),
        ("client_id", provider.client_id().to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("scope", provider.scopes().to_string()),
        ("code_challenge", pkce.challenge.clone()),
        ("code_challenge_method", "S256".to_string()),
        ("state", pkce.state.clone()),
    ];
    for (key, value) in provider.authorize_extra() {
        params.push((key, (*value).to_string()));
    }
    format!(
        "{}{}?{}",
        provider.issuer(),
        provider.authorize_path(),
        form_encode(&params)
    )
}

// ---------------------------------------------------------------------------
// Token exchange and refresh
// ---------------------------------------------------------------------------

pub fn exchange_code(
    provider: Provider,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenSet> {
    exchange_code_at(provider, &provider.token_url(), code, verifier, redirect_uri)
}

/// `exchange_code` against an explicit endpoint. Tests point this at a loopback
/// server; production uses the provider URL.
pub fn exchange_code_at(
    provider: Provider,
    token_url: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenSet> {
    let params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", provider.client_id().to_string()),
        ("code_verifier", verifier.to_string()),
    ];
    let (status, text) = post_form(token_url, &params)?;
    parse_token_response(provider, status, &text, None)
}

pub fn refresh(provider: Provider, tokens: &TokenSet) -> Result<TokenSet> {
    refresh_at(provider, &provider.token_url(), tokens)
}

pub fn refresh_at(provider: Provider, token_url: &str, tokens: &TokenSet) -> Result<TokenSet> {
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        return Err(Error::msg(
            "The stored login has no refresh token. Sign in again.",
        ));
    };
    let params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", provider.client_id().to_string()),
    ];
    let (status, text) = post_form(token_url, &params)?;
    parse_token_response(provider, status, &text, Some(tokens))
}

/// Load a login and refresh it when it is close to expiry.
pub fn access_token(provider: Provider) -> Result<TokenSet> {
    let tokens = load(provider)?.ok_or_else(|| {
        Error::msg(format!(
            "Not signed in to {}. Run `dah auth subscription login {}`, or use the Settings screen.",
            provider.label(),
            provider.id()
        ))
    })?;
    if tokens.expired(now_unix()) {
        let refreshed = refresh(provider, &tokens)?;
        save(&refreshed)?;
        Ok(refreshed)
    } else {
        Ok(tokens)
    }
}

pub fn revoke(provider: Provider, tokens: &TokenSet) -> Result<()> {
    let params = vec![
        ("token", tokens.access_token.clone()),
        ("client_id", provider.client_id().to_string()),
    ];
    let (status, text) = post_form(&provider.revoke_url(), &params)?;
    if (200..300).contains(&status) || status == 400 || status == 401 {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "The provider did not confirm sign-out (HTTP {status}): {}",
            text.chars().take(200).collect::<String>()
        )))
    }
}

fn parse_token_response(
    provider: Provider,
    status: u16,
    text: &str,
    previous: Option<&TokenSet>,
) -> Result<TokenSet> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        Error::msg(format!(
            "The token endpoint returned invalid JSON ({error}): {}",
            text.chars().take(200).collect::<String>()
        ))
    })?;

    if !(200..300).contains(&status) {
        let message = value
            .get("error_description")
            .or_else(|| value.get("error_message"))
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("no message");
        return Err(Error::msg(format!(
            "The token endpoint returned HTTP {status}: {message}"
        )));
    }

    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("The token response has no access_token."))?
        .to_string();
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| previous.and_then(|tokens| tokens.refresh_token.clone()));
    let id_token = value
        .get("id_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| previous.and_then(|tokens| tokens.id_token.clone()));
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600) as i64;

    let account_id = if provider == Provider::Codex {
        let account = account_id_from_access_token(&access_token)
            .or_else(|| id_token.as_deref().and_then(account_id_from_access_token))
            .or_else(|| previous.and_then(|tokens| tokens.account_id.clone()));
        Some(account.ok_or_else(|| {
            Error::msg(
                "The Codex login returned no chatgpt_account_id claim. Sign in again from the official client list.",
            )
        })?)
    } else {
        None
    };

    Ok(TokenSet {
        schema: TOKEN_SCHEMA,
        provider,
        access_token,
        refresh_token,
        id_token,
        account_id,
        expires_at_unix: now_unix() + expires_in,
        saved_at: now_iso(),
    })
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

pub fn request_config(
    provider: Provider,
    tokens: &TokenSet,
    model: &str,
) -> Result<ProviderRequest> {
    if model.trim().is_empty() {
        return Err(Error::msg("A model id is required for the subscription."));
    }
    if !header_safe(model) {
        return Err(Error::msg(
            "The model id contains characters a request header cannot carry.",
        ));
    }

    let mut headers = vec![(
        "Authorization".to_string(),
        format!("Bearer {}", tokens.access_token),
    )];

    if provider == Provider::Codex {
        let account = tokens
            .account_id
            .as_deref()
            .ok_or_else(|| Error::msg("The Codex login has no account id. Sign in again."))?;
        if !valid_account_id(account) {
            return Err(Error::msg(
                "The Codex account id is not safe for a request header. Sign in again.",
            ));
        }
        headers.push(("chatgpt-account-id".to_string(), account.to_string()));
        headers.push((
            "OpenAI-Beta".to_string(),
            "responses=experimental".to_string(),
        ));
        headers.push(("originator".to_string(), "dah".to_string()));
        headers.push(("accept".to_string(), "text/event-stream".to_string()));
    } else {
        // The Grok CLI proxy checks this token marker and the client id.
        headers.push((
            "X-XAI-Token-Auth".to_string(),
            "xai-grok-cli".to_string(),
        ));
        headers.push(("x-grok-client-identifier".to_string(), "dah".to_string()));
        headers.push(("x-grok-model-override".to_string(), model.to_string()));
        headers.push(("accept".to_string(), "text/event-stream".to_string()));
    }

    Ok(ProviderRequest {
        endpoint: provider.endpoint().to_string(),
        headers,
    })
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub fn data_dir() -> Result<PathBuf> {
    if let Some(configured) = std::env::var_os("DAH_DATA_DIR")
        && !configured.is_empty()
    {
        return Ok(PathBuf::from(configured));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        Ok(home
            .map(|home| home.join("Library/Application Support/DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah")))
    } else if cfg!(target_os = "windows") {
        Ok(std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("DeadlockAgentHarness"))
            .unwrap_or_else(|| PathBuf::from(".dah")))
    } else {
        Ok(std::env::var_os("XDG_DATA_HOME")
            .map(|xdg| PathBuf::from(xdg).join("DeadlockAgentHarness"))
            .or_else(|| home.map(|home| home.join(".local/share/deadlock-harness")))
            .unwrap_or_else(|| PathBuf::from(".dah")))
    }
}

pub fn auth_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("auth"))
}

/// The agent host configuration written by `dah auth use` or the Settings
/// screen. It names a subscription provider and a model, never a token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    pub schema: u32,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub saved_at: String,
}

pub fn host_config_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("host.json"))
}

pub fn save_host_config(provider: Provider, model: &str) -> Result<PathBuf> {
    let model = model.trim();
    if model.is_empty() {
        return Err(Error::msg("A model id is required."));
    }
    if !header_safe(model) {
        return Err(Error::msg(
            "The model id contains characters a request header cannot carry.",
        ));
    }
    let path = host_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config = HostConfig {
        schema: 1,
        provider: provider.id().to_string(),
        model: model.to_string(),
        saved_at: now_iso(),
    };
    let mut bytes = serde_json::to_vec_pretty(&config)?;
    bytes.push(b'\n');
    write_atomic(&path, &bytes)?;
    Ok(path)
}

pub fn load_host_config() -> Result<Option<HostConfig>> {
    let path = host_config_path()?;
    match fs::read_to_string(&path) {
        Ok(text) => {
            let config: HostConfig = serde_json::from_str(&text).map_err(|error| {
                Error::msg(format!("Could not parse {}: {error}", path.display()))
            })?;
            if Provider::parse(&config.provider).is_none() {
                return Err(Error::msg(format!(
                    "{} names unknown provider {:?}. Use codex or grok.",
                    path.display(),
                    config.provider
                )));
            }
            Ok(Some(config))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

pub fn token_path(provider: Provider) -> Result<PathBuf> {
    Ok(auth_dir()?.join(format!("{}.json", provider.id())))
}

pub fn save(tokens: &TokenSet) -> Result<PathBuf> {
    save_at(&auth_dir()?, tokens)
}

pub fn save_at(dir: &Path, tokens: &TokenSet) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", tokens.provider.id()));
    let mut bytes = serde_json::to_vec_pretty(tokens)?;
    bytes.push(b'\n');
    write_atomic(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

pub fn load(provider: Provider) -> Result<Option<TokenSet>> {
    load_at(&auth_dir()?, provider)
}

pub fn load_at(dir: &Path, provider: Provider) -> Result<Option<TokenSet>> {
    let path = dir.join(format!("{}.json", provider.id()));
    match fs::read_to_string(&path) {
        Ok(text) => {
            let tokens: TokenSet = serde_json::from_str(&text).map_err(|error| {
                Error::msg(format!(
                    "Could not parse {}: {error}. Sign in again.",
                    path.display()
                ))
            })?;
            if tokens.provider != provider {
                return Err(Error::msg(format!(
                    "{} holds a {} login, not {}.",
                    path.display(),
                    tokens.provider.id(),
                    provider.id()
                )));
            }
            Ok(Some(tokens))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

pub fn delete(provider: Provider) -> Result<bool> {
    delete_at(&auth_dir()?, provider)
}

pub fn delete_at(dir: &Path, provider: Provider) -> Result<bool> {
    let path = dir.join(format!("{}.json", provider.id()));
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::msg(format!(
            "Could not delete {}: {error}",
            path.display()
        ))),
    }
}

// ---------------------------------------------------------------------------
// JWT and encoding helpers
// ---------------------------------------------------------------------------

/// Read `https://api.openai.com/auth.chatgpt_account_id` from an access token.
pub fn account_id_from_access_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let account = claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()?;
    if valid_account_id(account) {
        Some(account.to_string())
    } else {
        None
    }
}

fn valid_account_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn header_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.bytes().any(|byte| byte == b'\r' || byte == b'\n')
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn post_form(url: &str, params: &[(&str, String)]) -> Result<(u16, String)> {
    let body = form_encode(params);
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(HTTP_TIMEOUT_SECONDS)))
        .build()
        .new_agent();
    let mut response = agent
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .header("Content-Length", body.len().to_string())
        .send(body.as_bytes())
        .map_err(|error| Error::msg(format!("The auth request failed: {error}")))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::msg(format!("The auth response was unreadable: {error}")))?;
    Ok((status, text))
}

fn form_encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "dah-subscription-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn serve_once(body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header_end = buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buffer.len() < header_end + length {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&buffer).to_string();
            let payload = response.as_bytes();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                response
            );
            stream.write_all(reply.as_bytes()).unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    fn jwt(claims: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(claims.as_bytes());
        format!("{header}.{payload}.signature")
    }

    #[test]
    fn pkce_matches_the_rfc_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_carries_the_client_and_pkce() {
        let pkce = Pkce {
            verifier: "v".to_string(),
            challenge: "challenge-value".to_string(),
            state: "state-value".to_string(),
        };
        let url = authorize_url(
            Provider::Codex,
            "http://localhost:1455/auth/callback",
            &pkce,
        );
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("code_challenge=challenge-value"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        let grok = authorize_url(Provider::Grok, "http://127.0.0.1:9/callback", &pkce);
        assert!(grok.starts_with("https://auth.x.ai/oauth2/authorize?"));
        assert!(grok.contains("scope=openid%20profile%20email%20offline_access%20grok-cli%3Aaccess%20api%3Aaccess"));
    }

    #[test]
    fn reads_the_account_id_from_the_access_token() {
        let token = jwt(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_test"}}"#);
        assert_eq!(
            account_id_from_access_token(&token),
            Some("acct_test".to_string())
        );
        let bad = jwt(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"a\r\nb"}}"#);
        assert_eq!(account_id_from_access_token(&bad), None);
    }

    #[test]
    fn token_store_roundtrip_and_delete() {
        let dir = temp_dir("store");
        let tokens = TokenSet {
            schema: TOKEN_SCHEMA,
            provider: Provider::Codex,
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            id_token: None,
            account_id: Some("acct".to_string()),
            expires_at_unix: now_unix() + 3600,
            saved_at: now_iso(),
        };
        save_at(&dir, &tokens).unwrap();
        let loaded = load_at(&dir, Provider::Codex).unwrap().unwrap();
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.account_id, Some("acct".to_string()));
        assert!(delete_at(&dir, Provider::Codex).unwrap());
        assert!(load_at(&dir, Provider::Codex).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exchange_reads_a_token_response() {
        let access = jwt(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_exchange"}}"#);
        let body = format!(
            r#"{{"access_token":"{access}","refresh_token":"refresh","expires_in":3600}}"#
        );
        let (url, server) = serve_once(&body);
        let tokens = exchange_code_at(
            Provider::Codex,
            &format!("{url}/token"),
            "code",
            "verifier",
            "http://localhost:1455/auth/callback",
        )
        .unwrap();
        assert_eq!(tokens.account_id, Some("acct_exchange".to_string()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /token"));
        assert!(request.contains("grant_type=authorization_code"));
        assert!(request.contains("code_verifier=verifier"));
    }

    #[test]
    fn refresh_keeps_the_account_id() {
        let access = jwt(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_refresh"}}"#);
        let body = format!(r#"{{"access_token":"{access}","expires_in":3600}}"#);
        let (url, _server) = serve_once(&body);
        let previous = TokenSet {
            schema: TOKEN_SCHEMA,
            provider: Provider::Codex,
            access_token: "old".to_string(),
            refresh_token: Some("refresh".to_string()),
            id_token: None,
            account_id: Some("acct_refresh".to_string()),
            expires_at_unix: now_unix() - 10,
            saved_at: now_iso(),
        };
        let tokens = refresh_at(Provider::Codex, &format!("{url}/token"), &previous).unwrap();
        assert_eq!(tokens.account_id, Some("acct_refresh".to_string()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));
    }

    #[test]
    fn request_headers_are_provider_specific() {
        let tokens = TokenSet {
            schema: TOKEN_SCHEMA,
            provider: Provider::Codex,
            access_token: "access".to_string(),
            refresh_token: None,
            id_token: None,
            account_id: Some("acct".to_string()),
            expires_at_unix: now_unix() + 3600,
            saved_at: now_iso(),
        };
        let request = request_config(Provider::Codex, &tokens, "gpt-5-codex").unwrap();
        assert_eq!(
            request.endpoint,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert!(request.headers.iter().any(|(name, value)| name == "chatgpt-account-id" && value == "acct"));
        assert!(request.headers.iter().any(|(name, _)| name == "OpenAI-Beta"));

        let grok_tokens = TokenSet {
            provider: Provider::Grok,
            account_id: None,
            ..tokens
        };
        let grok = request_config(Provider::Grok, &grok_tokens, "grok-4").unwrap();
        assert_eq!(
            grok.endpoint,
            "https://cli-chat-proxy.grok.com/v1/responses"
        );
        assert!(grok.headers.iter().any(|(name, value)| name == "x-grok-model-override" && value == "grok-4"));
        assert!(grok.headers.iter().any(|(name, value)| name == "X-XAI-Token-Auth" && value == "xai-grok-cli"));
    }

    #[test]
    fn debug_redacts_tokens() {
        let tokens = TokenSet {
            schema: TOKEN_SCHEMA,
            provider: Provider::Codex,
            access_token: "secret-access".to_string(),
            refresh_token: Some("secret-refresh".to_string()),
            id_token: None,
            account_id: Some("acct".to_string()),
            expires_at_unix: 0,
            saved_at: now_iso(),
        };
        let text = format!("{tokens:?}");
        assert!(!text.contains("secret-access"));
        assert!(!text.contains("secret-refresh"));
    }
}
