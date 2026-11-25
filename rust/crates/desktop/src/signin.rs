//! Subscription sign-in from inside the app, in place of the old helper scripts.
//!
//! The OAuth flow itself (loopback listener, PKCE, token exchange, the chmod 600 store) stays in
//! one place: `dah auth login`, which ships beside the app. This module starts it as a child
//! process with the browser opened by the CLI, reads the authorize URL it prints so Settings can
//! offer it again, and settles when the child exits. No token passes through this process: the
//! CLI writes the login to the store and Settings re-reads only the signed-in state.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::app::State;

/// A sign-in the user started and has not finished in the browser yet.
#[derive(Clone)]
pub struct Pending {
    pub provider: String,
    pub url: String,
    pub code: Option<String>,
}

enum Msg {
    Url(String),
    Done(Result<(), String>),
}

/// The running sign-in, if any. One at a time.
#[derive(Default)]
pub struct Flow {
    pending: Option<Pending>,
    rx: Option<Receiver<Msg>>,
    child: Option<Arc<Mutex<Option<Child>>>>,
    /// The last outcome, for one quiet line under the provider row.
    pub message: Option<(String, bool)>,
}

pub fn pending(st: &State) -> Option<Pending> {
    st.signin.pending.clone()
}

/// Start the browser sign-in for `provider` ("codex" or "grok").
pub fn start(st: &mut State, provider: &str) {
    cancel(st);
    if rune::is_headless() {
        st.signin.pending = Some(Pending {
            provider: provider.to_string(),
            url: String::new(),
            code: None,
        });
        return;
    }
    let Some(binary) = dah_binary() else {
        st.signin.message = Some((
            "Sigil is missing a part it needs (its command line). Reinstall the app.".to_string(),
            true,
        ));
        return;
    };
    let spawned = binary
        .command()
        .args(["auth", "login", provider, "--json"])
        .env(deadlock_core::app_data::ENV, &st.paths.data)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            st.signin.message = Some((format!("Sign-in could not start: {error}"), true));
            return;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = mpsc::channel();
    let wake = st.wake.clone();
    let waiter = child.clone();
    let _ = std::thread::Builder::new()
        .name("sigil-signin".to_string())
        .spawn(move || {
            let mut ok_line = false;
            if let Some(stdout) = stdout {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.starts_with("https://") {
                        let _ = tx.send(Msg::Url(line));
                        if let Some(wake) = &wake {
                            wake();
                        }
                    } else if line.contains("\"ok\": true") || line.contains("\"ok\":true") {
                        ok_line = true;
                    }
                }
            }
            let mut err_text = String::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_string(&mut err_text);
            }
            let status = waiter
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .and_then(|mut child| child.wait().ok());
            let result = match status {
                Some(status) if status.success() || ok_line => Ok(()),
                Some(_) => Err(friendly_error(&err_text)),
                // Taken by `cancel`: the user stopped it.
                None => Err(String::new()),
            };
            let _ = tx.send(Msg::Done(result));
            if let Some(wake) = &wake {
                wake();
            }
        });
    st.signin.pending = Some(Pending {
        provider: provider.to_string(),
        url: String::new(),
        code: None,
    });
    st.signin.rx = Some(rx);
    st.signin.child = Some(child);
    st.signin.message = None;
}

/// Stop a sign-in that is waiting on the browser.
pub fn cancel(st: &mut State) {
    if let Some(child) = st.signin.child.take()
        && let Ok(mut slot) = child.lock()
        && let Some(mut child) = slot.take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
    st.signin.pending = None;
    st.signin.rx = None;
}

/// Remove the stored login for `provider`, revoking it when the provider allows.
pub fn sign_out(st: &mut State, provider: &str) {
    let Some(provider) = deadlock_core::subscription::Provider::parse(provider) else {
        return;
    };
    if let Ok(Some(tokens)) = deadlock_core::subscription::load(provider) {
        let _ = deadlock_core::subscription::revoke(provider, &tokens);
    }
    match deadlock_core::subscription::delete(provider) {
        Ok(_) => st.signin.message = None,
        Err(error) => st.signin.message = Some((error.to_string(), true)),
    }
    crate::data::refresh_subscriptions(st);
}

/// Drain the sign-in channel. Returns true when Settings should redraw.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    let mut done = None;
    if let Some(rx) = &st.signin.rx {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Url(url) => {
                    if let Some(pending) = st.signin.pending.as_mut() {
                        pending.url = url;
                    }
                }
                Msg::Done(result) => done = Some(result),
            }
            changed = true;
        }
    }
    if let Some(result) = done {
        let provider = st
            .signin
            .pending
            .as_ref()
            .map(|pending| pending.provider.clone())
            .unwrap_or_default();
        st.signin.pending = None;
        st.signin.rx = None;
        st.signin.child = None;
        crate::data::refresh_subscriptions(st);
        match result {
            Ok(()) => {
                st.signin.message = None;
                // With a model already chosen, the agent switches to this login at once.
                if !st.host_model.text.trim().is_empty() {
                    crate::data::use_subscription_for_agent(st, &provider);
                }
            }
            Err(error) if error.is_empty() => {}
            Err(error) => st.signin.message = Some((error, true)),
        }
    }
    changed
}

fn friendly_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("five minutes") || lower.contains("timeout") {
        "Sign-in timed out. Try again.".to_string()
    } else if lower.contains("loopback") || lower.contains("address in use") {
        "Another sign-in is already open. Close it and try again.".to_string()
    } else if lower.contains("denied") || lower.contains("access_denied") {
        "Sign-in was cancelled in the browser.".to_string()
    } else {
        "Sign-in did not finish. Try again.".to_string()
    }
}

/// The dah CLI: `DAH_RS_BIN`, else this executable as `cli` (`crate::helpers`).
fn dah_binary() -> Option<crate::helpers::Helper> {
    crate::helpers::cli().ok()
}
