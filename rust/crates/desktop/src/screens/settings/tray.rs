//! Minimize to tray and close to tray (Deadlock Mod Manager's tray, reimplemented on Rune's
//! `Tray`). The icon exists only while one of the two is on. Its menu: Open Sigil, Quit.
//!
//! Rune's tray runs on Windows and Linux; macOS has no tray backend yet, so the rows say so.
//! The window hooks live in `main.rs` (owned elsewhere):
//!
//! ```ignore
//! let app = rune::App::new(spec).titlebar(..).keep_running().on_close(screens::settings::tray::on_close);
//! screens::settings::tray::install(app.handle());
//! app.run();
//! ```
//!
//! Until those lines exist, [`installed`] is false and the rows explain that a restart is
//! needed; nothing else changes.

use std::sync::{Mutex, OnceLock};

use rune::{AppHandle, El, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec, Tx};

use super::i18n::tr;
use super::options;
use super::{disabled, section, toggle_row};
use crate::app::State;
use crate::ui::Ui;

static HANDLE: OnceLock<AppHandle<State>> = OnceLock::new();
static TRAY: Mutex<Option<Tray>> = Mutex::new(None);

/// Whether this platform has a tray.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// Whether `main.rs` handed over the window handle (the hook is in place).
pub fn installed() -> bool {
    HANDLE.get().is_some()
}

/// From `main.rs`, before `run`.
pub fn install(handle: AppHandle<State>) {
    let _ = HANDLE.set(handle);
    sync();
}

/// The window's close: hide to the tray when asked and possible, else quit.
pub fn on_close(st: &mut State, tx: &mut Tx) {
    if options::get().tray_close && active() {
        return;
    }
    st.agent.shutdown();
    tx.quit();
}

fn active() -> bool {
    TRAY.lock().map(|t| t.is_some()).unwrap_or(false)
}

/// Minimize Sigil: into the tray when "Minimize to tray" is on and the tray is up.
pub fn minimize(tx: &mut Tx) {
    if options::get().tray_minimize
        && active()
        && let Some(handle) = HANDLE.get()
    {
        handle.hide();
        return;
    }
    tx.minimize();
}

/// A 32 by 32 icon: an ember disc with a dark ring, drawn here so no file is needed.
pub fn icon_pixels() -> TrayIcon {
    let size = 32u32;
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let (r, g, b, a) = if d <= 9.0 {
                (0xF2, 0x6B, 0x3A, 0xFF)
            } else if d <= 14.5 {
                (0x1C, 0x1A, 0x19, 0xFF)
            } else if d <= 15.5 {
                (0x1C, 0x1A, 0x19, ((15.5 - d) * 255.0) as u8)
            } else {
                (0, 0, 0, 0)
            };
            px.extend_from_slice(&[r, g, b, a]);
        }
    }
    TrayIcon::rgba(size, size, px)
}

/// Create or remove the icon to match the options.
pub fn sync() {
    let o = options::get();
    let want = (o.tray_minimize || o.tray_close) && supported();
    let Some(handle) = HANDLE.get().cloned() else { return };
    let Ok(mut slot) = TRAY.lock() else { return };
    if !want {
        *slot = None;
        return;
    }
    if slot.is_some() {
        return;
    }
    let spec = TraySpec::new("app.sigil.desktop", "Sigil", icon_pixels())
        .tooltip("Sigil")
        .menu(vec![
            TrayItem::action("open", tr("Open Sigil")),
            TrayItem::Separator,
            TrayItem::action("quit", tr("Quit")),
        ]);
    match Tray::new(spec, move |event| match event {
        TrayEvent::Activate { token } => match token {
            Some(token) => handle.activate(token),
            None => handle.open(),
        },
        TrayEvent::Menu(id) if id == "quit" => handle.quit(),
        TrayEvent::Menu(_) => handle.open(),
        _ => {}
    }) {
        Ok(tray) => *slot = Some(tray),
        Err(error) => super::log::error(format!("Tray: {error}")),
    }
}

pub fn rows(_s: &State, u: &Ui) -> El {
    let o = options::get();
    let line_min = if !supported() {
        "Not available on macOS yet."
    } else if !installed() {
        "Takes effect after Sigil restarts."
    } else {
        "Minimizing Sigil hides it in the system tray."
    };
    let line_close = if !supported() {
        "Not available on macOS yet."
    } else if !installed() {
        "Takes effect after Sigil restarts."
    } else {
        "The close button hides Sigil in the tray; Quit from the tray menu ends it."
    };
    section(
        u,
        "Window",
        vec![
            disabled(
                toggle_row(u, "settings-tray-minimize", "Minimize to tray", line_min, o.tray_minimize, |st| {
                    options::edit(st, |o| o.tray_minimize = !o.tray_minimize);
                    sync();
                }),
                !supported(),
            ),
            disabled(
                toggle_row(u, "settings-tray-close", "Close to tray", line_close, o.tray_close, |st| {
                    options::edit(st, |o| o.tray_close = !o.tray_close);
                    sync();
                }),
                !supported(),
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_one_rgba_pixel_per_point() {
        assert_eq!(icon_pixels(), icon_pixels());
        if !supported() {
            // Where there is no tray, a well-formed icon fails as unsupported, not as a bad size.
            let spec = TraySpec::new("t", "T", icon_pixels());
            assert!(matches!(Tray::new(spec, |_| {}), Err(rune::TrayError::Unsupported)));
        }
        assert!(!installed());
    }
}
