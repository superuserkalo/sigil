//! DeadlockForge installs: the app side of `deadlock_core::forge_bridge`.
//!
//! Off by default (Settings, "Installs from DeadlockForge"). On, a loopback listener accepts a
//! build the DeadlockForge site sends, and it opens in the same import review as a file dropped
//! on the window: that review, which names the mod and shows what it changes, is where the
//! player confirms or cancels. Only one build waits at a time; the listener takes the next once
//! the review closes, and the received file is deleted then.

// Backlogged by the owner (2026-11-18): built and tested, not wired into the app yet
// (`.scratch/parity-2026-11-18/plan.md`).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use deadlock_core::forge_bridge::{Bridge, Pending};

use crate::app::State;
use crate::notify::{self, Notice};

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);
static INBOX: Mutex<VecDeque<Pending>> = Mutex::new(VecDeque::new());
static WAKE: Mutex<Option<rune::Waker>> = Mutex::new(None);
/// The received file whose review is open, deleted once it closes.
static IN_REVIEW: Mutex<Option<PathBuf>> = Mutex::new(None);

fn folder() -> PathBuf {
    crate::data::data_dir().join("forge")
}

/// Start or stop the listener for the Settings switch. Starting twice keeps the running one.
pub fn set_enabled(on: bool) {
    let Ok(mut bridge) = BRIDGE.lock() else { return };
    if !on {
        *bridge = None;
        return;
    }
    if bridge.is_some() || rune::is_headless() {
        return;
    }
    match Bridge::start(&folder(), |pending| {
        if let Ok(mut inbox) = INBOX.lock() {
            inbox.push_back(pending);
        }
        if let Ok(wake) = WAKE.lock()
            && let Some(wake) = wake.as_ref()
        {
            wake();
        }
    }) {
        Ok(started) => {
            crate::screens::settings::log::info(format!("DeadlockForge installs listen on port {}.", started.port()));
            *bridge = Some(started);
        }
        Err(error) => crate::screens::settings::log::error(format!("DeadlockForge installs: {error}")),
    }
}

pub fn is_running() -> bool {
    BRIDGE.lock().is_ok_and(|b| b.is_some())
}

/// From the app's poll: hand a received build to the import review, and free the listener for
/// the next one when that review has closed.
pub fn poll(st: &mut State) -> bool {
    if let Ok(mut wake) = WAKE.lock()
        && wake.is_none()
    {
        *wake = st.wake.clone();
    }
    let mut changed = false;
    if let Ok(mut open) = IN_REVIEW.lock()
        && open.is_some()
        && st.installed.import.dialog.is_none()
    {
        if let Some(path) = open.take() {
            let _ = std::fs::remove_file(path);
        }
        if let Ok(bridge) = BRIDGE.lock()
            && let Some(bridge) = bridge.as_ref()
        {
            bridge.answered();
        }
    }
    let next = INBOX.lock().ok().and_then(|mut inbox| inbox.pop_front());
    if let Some(pending) = next {
        // The review names the file, so give it the mod's name rather than the temp name.
        let named = folder().join(format!("{}.vpk", file_stem(&pending.name)));
        let path = match std::fs::rename(&pending.path, &named) {
            Ok(()) => named,
            Err(_) => pending.path.clone(),
        };
        let by = if pending.author.is_empty() { String::new() } else { format!(" by {}", pending.author) };
        let title = if pending.name.is_empty() { "A build".to_string() } else { pending.name.clone() };
        notify::push(st, Notice::info(format!("DeadlockForge sent {title}{by}. Review it to install.")));
        crate::shell::dropped(st, &[path.clone()]);
        if let Ok(mut open) = IN_REVIEW.lock() {
            *open = Some(path);
        }
        changed = true;
    }
    changed
}

/// A file name from the mod's name: letters, digits, spaces, dashes and underscores; never empty.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(60)
        .collect::<String>()
        .trim()
        .to_string();
    if stem.is_empty() { "DeadlockForge build".to_string() } else { stem }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_received_name_makes_a_safe_file_name() {
        assert_eq!(file_stem("Crimson Haze"), "Crimson Haze");
        assert_eq!(file_stem("../../evil\\name"), "evilname");
        assert_eq!(file_stem("   "), "DeadlockForge build");
        assert_eq!(file_stem(&"a".repeat(100)).len(), 60);
    }
}
