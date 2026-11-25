//! `dah auth`: subscription sign-in for Codex and Grok.
//!
//! The login is the first-party OAuth flow: a loopback listener, PKCE S256, the
//! browser, and a token exchange. No browser cookie is read and no token is
//! printed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use deadlock_core::subscription::{self, Provider};

use crate::error::{CliError, Result};
use crate::util::print_json;

const LOGIN_TIMEOUT_SECONDS: u64 = 300;

pub struct LoginArgs {
    pub provider: String,
    pub no_open: bool,
    pub json: bool,
}

pub struct StatusArgs {
    pub provider: Option<String>,
    pub json: bool,
}

pub struct LogoutArgs {
    pub provider: String,
    pub json: bool,
}

fn provider(value: &str) -> Result<Provider> {
    Provider::parse(value).ok_or_else(|| {
        CliError::new(
            "auth_provider",
            format!(
                "Unknown subscription provider {value:?}. Use codex or grok."
            ),
        )
    })
}

pub fn login(args: &LoginArgs) -> Result<()> {
    let provider = provider(&args.provider)?;
    let pkce = subscription::pkce().map_err(|error| CliError::new("auth_pkce", error.to_string()))?;
    let listener = bind_listener(provider)?;
    let port = listener
        .local_addr()
        .map_err(|error| CliError::new("auth_callback", error.to_string()))?
        .port();
    let redirect_uri = provider.redirect_uri(port);
    let url = subscription::authorize_url(provider, &redirect_uri, &pkce);

    if !args.no_open && !open_browser(&url) {
        eprintln!("Could not open a browser. Open this URL by hand:");
        eprintln!("{url}");
    } else {
        println!("Open this URL if the browser did not:");
        println!("{url}");
    }
    println!();
    println!(
        "Waiting for the {} callback on {redirect_uri} (up to {LOGIN_TIMEOUT_SECONDS}s).",
        provider.label()
    );

    let code = wait_for_code(&listener, provider, &pkce.state)?;
    let tokens = subscription::exchange_code(provider, &code, &pkce.verifier, &redirect_uri)
        .map_err(|error| CliError::new("auth_exchange", error.to_string()))?;
    let path = subscription::save(&tokens)
        .map_err(|error| CliError::new("auth_store", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "provider": provider.id(),
            "accountId": tokens.account_id,
            "expiresAtUnix": tokens.expires_at_unix,
            "path": path.display().to_string(),
            "next": {
                "envProvider": provider.id(),
                "envModel": "the exact model id your subscription exposes",
            },
        }));
    } else {
        println!("Signed in to {}.", provider.label());
        if let Some(account) = &tokens.account_id {
            println!("  account: {account}");
        }
        println!(
            "  access expires in {} minutes",
            (tokens.expires_at_unix - now_unix()) / 60
        );
        println!("  stored:  {}", path.display());
        println!();
        println!("Next: set DAH_AGENT_PROVIDER={} and DAH_AGENT_MODEL=<model id>, then start the app.", provider.id());
        println!("The host reads this login. No key goes on a command line.");
    }
    Ok(())
}

pub fn status(args: &StatusArgs) -> Result<()> {
    let providers: Vec<Provider> = match &args.provider {
        Some(value) => vec![provider(value)?],
        None => Provider::ALL.to_vec(),
    };
    let mut rows = Vec::new();
    for provider in providers {
        let tokens = subscription::load(provider)
            .map_err(|error| CliError::new("auth_store", error.to_string()))?;
        rows.push((provider, tokens));
    }

    if args.json {
        print_json(&json!({
            "ok": true,
            "providers": rows.iter().map(|(provider, tokens)| json!({
                "provider": provider.id(),
                "label": provider.label(),
                "signedIn": tokens.is_some(),
                "accountId": tokens.as_ref().and_then(|tokens| tokens.account_id.clone()),
                "expiresAtUnix": tokens.as_ref().map(|tokens| tokens.expires_at_unix),
                "expiresInSeconds": tokens.as_ref().map(|tokens| tokens.expires_in_seconds(now_unix())),
                "hasRefreshToken": tokens.as_ref().and_then(|tokens| tokens.refresh_token.as_ref()).is_some(),
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    for (provider, tokens) in rows {
        match tokens {
            Some(tokens) => {
                println!("{}: signed in", provider.id());
                if let Some(account) = &tokens.account_id {
                    println!("  account: {account}");
                }
                println!(
                    "  expires in {} minutes · refresh token {}",
                    tokens.expires_in_seconds(now_unix()) / 60,
                    if tokens.refresh_token.is_some() {
                        "present"
                    } else {
                        "absent"
                    }
                );
            }
            None => println!(
                "{}: signed out. Run `Sigil cli auth login {}`.",
                provider.id(),
                provider.id()
            ),
        }
    }
    Ok(())
}

pub fn logout(args: &LogoutArgs) -> Result<()> {
    let provider = provider(&args.provider)?;
    let tokens = subscription::load(provider)
        .map_err(|error| CliError::new("auth_store", error.to_string()))?;
    let Some(tokens) = tokens else {
        println!("{} is already signed out.", provider.id());
        return Ok(());
    };

    let revoke = subscription::revoke(provider, &tokens);
    subscription::delete(provider)
        .map_err(|error| CliError::new("auth_store", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": revoke.is_ok(),
            "provider": provider.id(),
            "revoked": revoke.is_ok(),
            "error": revoke.err().map(|error| error.to_string()),
        }));
    } else {
        println!("Signed out of {}. The local login was removed.", provider.id());
        if let Err(error) = revoke {
            println!("  the provider did not confirm revocation: {error}");
        }
    }
    Ok(())
}

fn bind_listener(provider: Provider) -> Result<TcpListener> {
    let mut errors = Vec::new();
    for port in provider.ports() {
        match TcpListener::bind(("127.0.0.1", *port)) {
            Ok(listener) => return Ok(listener),
            Err(error) => errors.push(format!("127.0.0.1:{port}: {error}")),
        }
    }
    Err(CliError::new(
        "auth_callback",
        format!(
            "Could not open a loopback callback for {}. {}",
            provider.id(),
            errors.join("; ")
        ),
    ))
}

fn wait_for_code(listener: &TcpListener, provider: Provider, expected_state: &str) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| CliError::new("auth_callback", error.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(LOGIN_TIMEOUT_SECONDS);

    loop {
        if Instant::now() > deadline {
            return Err(CliError::new(
                "auth_timeout",
                "The sign-in did not finish within five minutes. Run login again.".to_string(),
            ));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_request(&mut stream);
                let target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                let path = target.split('?').next().unwrap_or("");
                if path != provider.callback_path() {
                    let _ = respond(&mut stream, 404, "Not found", "This is not the sign-in callback.");
                    continue;
                }
                let query = target.split_once('?').map(|(_, query)| query).unwrap_or("");
                let params = parse_query(query);
                if let Some(error) = params.get("error") {
                    let description = params
                        .get("error_description")
                        .cloned()
                        .unwrap_or_else(|| error.clone());
                    let _ = respond(&mut stream, 400, "Sign-in failed", &description);
                    return Err(CliError::new("auth_denied", description));
                }
                let state = params.get("state").cloned().unwrap_or_default();
                if state != expected_state {
                    let _ = respond(
                        &mut stream,
                        400,
                        "Sign-in failed",
                        "The state did not match. Start the login again.",
                    );
                    return Err(CliError::new(
                        "auth_state",
                        "The callback state did not match the login. Start again.".to_string(),
                    ));
                }
                let Some(code) = params.get("code").cloned() else {
                    let _ = respond(&mut stream, 400, "Sign-in failed", "The callback had no code.");
                    return Err(CliError::new(
                        "auth_code",
                        "The callback had no authorization code.".to_string(),
                    ));
                };
                let _ = respond(
                    &mut stream,
                    200,
                    "Signed in",
                    "You can close this tab and return to the terminal.",
                );
                return Ok(code);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(CliError::new(
                    "auth_callback",
                    format!("The callback listener failed: {error}"),
                ));
            }
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 16_384 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn respond(stream: &mut TcpStream, status: u16, title: &str, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"font:15px system-ui;background:#121212;color:#eee;padding:40px\">\
         <h1 style=\"font-weight:600\">{title}</h1><p>{body}</p></body>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn parse_query(query: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        map.insert(percent_decode(key), percent_decode(value));
    }
    map
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).to_string()
}

fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    result.is_ok()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub struct UseArgs {
    pub provider: String,
    pub model: String,
    pub json: bool,
}

/// `Sigil cli auth use <provider> --model <id>`: point the host at the login.
pub fn use_for_agent(args: &UseArgs) -> Result<()> {
    let provider = provider(&args.provider)?;
    let tokens = subscription::load(provider)
        .map_err(|error| CliError::new("auth_store", error.to_string()))?;
    if tokens.is_none() {
        return Err(CliError::new(
            "auth_missing",
            format!(
                "No {} login yet. Run `Sigil cli auth login {}` first.",
                provider.id(),
                provider.id()
            ),
        ));
    }
    let path = subscription::save_host_config(provider, &args.model)
        .map_err(|error| CliError::new("auth_host", error.to_string()))?;
    if args.json {
        print_json(&json!({
            "ok": true,
            "provider": provider.id(),
            "model": args.model,
            "path": path.display().to_string(),
            "note": "Start or restart the desktop app; it reads this file before the agent host starts.",
        }));
    } else {
        println!("The agent host will use {} with model {}.", provider.id(), args.model);
        println!("  {}", path.display());
        println!("Start or restart the app. The token stays in the auth store.");
    }
    Ok(())
}
