//! Settings > Appearance > Transparent sidebar: real glass behind the sidebar.
//!
//! The operating system blurs what is behind the window and Sigil paints a translucent tint over
//! it in the sidebar only; the pages stay opaque. Nothing here blurs per frame: the system
//! composites the backdrop, so an idle window costs what it did before.
//!
//! - Windows 11 22H2 and later (build 22523+): `DWMWA_SYSTEMBACKDROP_TYPE` = Mica, the frame
//!   extended into the client area. Windows 11 21H2: the older `DWMWA_MICA_EFFECT`. Windows
//!   draws Mica solid while the window is inactive; the tint stays, so nothing flashes.
//! - Windows 10: `SetWindowCompositionAttribute` with a blur-behind accent. Not the acrylic
//!   accent: that one stalls dragging and resizing from 1903 on.
//! - macOS: the popover vibrancy material under the sidebar (`El::system_material`).
//! - Elsewhere, and whenever something is missing, the sidebar is the solid one.
//!
//! The window has to be created see-through (`WindowOptions::transparent`), which costs the
//! compositor a blend and skips Rune's CPU-drawn first frame. So it is created that way only when
//! the setting is on at launch and the system can show glass; turning it on later applies after a
//! restart, turning it off applies at once. Every call is best effort: a failure leaves the
//! solid sidebar and one log line, never a window that does not open.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The backdrop the system draws behind the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// Windows 11 22H2 and later: `DWMWA_SYSTEMBACKDROP_TYPE` = `DWMSBT_MAINWINDOW`.
    Mica,
    /// Windows 11 21H2: the undocumented `DWMWA_MICA_EFFECT` (1029).
    MicaLegacy,
    /// Windows 10: `ACCENT_ENABLE_BLURBEHIND` through `SetWindowCompositionAttribute`. A plain
    /// blur with no system tint, so Sigil's own tint is denser over it.
    Blur,
    /// macOS: `NSVisualEffectView`'s popover material, behind window.
    Vibrancy,
}

impl Backdrop {
    /// The material carries its own light or dark tint (Mica, vibrancy); a plain blur does not.
    pub fn tinted(self) -> bool {
        !matches!(self, Backdrop::Blur)
    }

    pub fn name(self) -> &'static str {
        match self {
            Backdrop::Mica => "Mica",
            Backdrop::MicaLegacy => "Mica (21H2)",
            Backdrop::Blur => "blur-behind accent",
            Backdrop::Vibrancy => "vibrancy",
        }
    }
}

/// Why the sidebar stays solid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// No backdrop API on this system (Linux and the rest).
    Platform,
    /// Windows before 10, or a Windows 10 without `SetWindowCompositionAttribute`.
    OldWindows,
    /// Windows Settings > Personalization > Colors > Transparency effects is off.
    TransparencyOff,
    /// A Windows contrast theme or macOS Increase Contrast.
    HighContrast,
    /// A remote desktop session.
    Remote,
    /// The window's GPU surface cannot composite see-through pixels (software rendering, an old
    /// driver, a non-DX12 backend on Windows).
    Surface,
    /// The system call for the backdrop failed.
    Refused,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::Platform => "no backdrop on this system",
            Why::OldWindows => "this Windows has no backdrop API",
            Why::TransparencyOff => "transparency effects are off in Windows",
            Why::HighContrast => "a high contrast setting is on",
            Why::Remote => "remote desktop session",
            Why::Surface => "the GPU surface is opaque",
            Why::Refused => "the system refused the backdrop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    #[cfg_attr(not(windows), allow(dead_code))]
    Windows,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MacOs,
    Other,
}

/// What the system says, read once at launch (and the transient parts again later).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    pub os: Os,
    /// The Windows build (22631 is 11 23H2, 19045 is 10 22H2); 0 elsewhere or unknown.
    pub build: u32,
    /// `user32!SetWindowCompositionAttribute` exists.
    pub accent_api: bool,
    /// Windows' transparency effects (on when the value is missing).
    pub transparency: bool,
    pub high_contrast: bool,
    pub remote: bool,
}

/// First build with `DWMWA_SYSTEMBACKDROP_TYPE` (Windows 11 22H2 shipped as 22621).
pub const BUILD_BACKDROP_TYPE: u32 = 22523;
/// Windows 11 21H2.
pub const BUILD_WINDOWS_11: u32 = 22000;
/// Windows 10 1507.
pub const BUILD_WINDOWS_10: u32 = 10240;

/// The backdrop for this system, or why there is none. Pure, so every branch is tested.
pub fn choose(p: &Probe) -> Result<Backdrop, Why> {
    match p.os {
        Os::Other => Err(Why::Platform),
        Os::MacOs => {
            if p.high_contrast {
                Err(Why::HighContrast)
            } else {
                Ok(Backdrop::Vibrancy)
            }
        }
        Os::Windows => {
            // The lasting reasons first, so the row says the one the player cannot change.
            let kind = if p.build >= BUILD_BACKDROP_TYPE {
                Backdrop::Mica
            } else if p.build >= BUILD_WINDOWS_11 {
                Backdrop::MicaLegacy
            } else if p.build >= BUILD_WINDOWS_10 && p.accent_api {
                Backdrop::Blur
            } else {
                return Err(Why::OldWindows);
            };
            if p.remote {
                Err(Why::Remote)
            } else if p.high_contrast {
                Err(Why::HighContrast)
            } else if !p.transparency {
                Err(Why::TransparencyOff)
            } else {
                Ok(kind)
            }
        }
    }
}

/// How the Settings row shows the switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// A switch that works now.
    Ready,
    /// On, but the window was made opaque at launch: it applies after a restart.
    Restart,
    /// No switch: the reason instead.
    Unavailable(Why),
}

/// The row's state from what the launch decided and what holds now. Pure.
pub fn row(headless: bool, pref: bool, window_glass: bool, now: Result<Backdrop, Why>, surface: Option<bool>, refused: bool) -> Row {
    if headless {
        return Row::Ready;
    }
    if let Err(why) = now {
        return Row::Unavailable(why);
    }
    if surface == Some(false) {
        return Row::Unavailable(Why::Surface);
    }
    if refused {
        return Row::Unavailable(Why::Refused);
    }
    if pref && !window_glass {
        return Row::Restart;
    }
    Row::Ready
}

// --- process state -------------------------------------------------------------------

struct Launch {
    /// The setting at launch.
    pref: bool,
    probe: Probe,
    /// The window was created see-through.
    window: bool,
}

static LAUNCH: OnceLock<Launch> = OnceLock::new();
/// The GPU surface composites alpha: 0 unknown, 1 yes, 2 no.
static SURFACE: AtomicU8 = AtomicU8::new(0);
static REFUSED: AtomicBool = AtomicBool::new(false);
/// A contrast setting is on now (the view writes it each frame from Rune's own reading).
static CONTRAST: AtomicBool = AtomicBool::new(false);
/// Windows' transparency effects now, re-read when the window comes back to the front.
static TRANSPARENCY: AtomicBool = AtomicBool::new(true);
/// Something here changed outside a handler; the next poll redraws.
static DIRTY: AtomicBool = AtomicBool::new(false);
/// The setting as the last frame drew it, for the feedback report.
static PREF: AtomicBool = AtomicBool::new(false);

thread_local! {
    static WINDOW: RefCell<Option<rune::WindowHandle>> = const { RefCell::new(None) };
    /// The backdrop and darkness last handed to the system: `None` before the first.
    static APPLIED: Cell<Option<(Option<Backdrop>, bool)>> = const { Cell::new(None) };
    static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// A scene previews this backdrop over a painted wallpaper (headless only). Per thread, so
    /// one test's scene never leaks into another's.
    static PREVIEW: Cell<Option<Backdrop>> = const { Cell::new(None) };
}

fn probe_now() -> Probe {
    #[cfg(windows)]
    {
        win::probe()
    }
    #[cfg(target_os = "macos")]
    {
        Probe { os: Os::MacOs, build: 0, accent_api: false, transparency: true, high_contrast: false, remote: false }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Probe { os: Os::Other, build: 0, accent_api: false, transparency: true, high_contrast: false, remote: false }
    }
}

/// At launch, before the window exists: whether to create it see-through. Reads the system once.
pub fn launch(pref: bool) -> bool {
    let headless = rune::is_headless();
    let probe = if headless {
        Probe { os: Os::Other, build: 0, accent_api: false, transparency: true, high_contrast: false, remote: false }
    } else {
        probe_now()
    };
    TRANSPARENCY.store(probe.transparency, Ordering::Relaxed);
    let window = pref && !headless && choose(&probe).is_ok();
    let _ = LAUNCH.set(Launch { pref, probe, window });
    window
}

fn launched() -> Option<&'static Launch> {
    LAUNCH.get()
}

/// The system's answer now: the launch probe with the settings that can change since.
fn now() -> Result<Backdrop, Why> {
    let Some(l) = launched() else { return Err(Why::Platform) };
    let mut p = l.probe;
    p.transparency = TRANSPARENCY.load(Ordering::Relaxed);
    p.high_contrast = CONTRAST.load(Ordering::Relaxed);
    choose(&p)
}

/// The view's reading of the system contrast setting, every frame (a relaxed store).
pub fn set_contrast(on: bool) {
    CONTRAST.store(on, Ordering::Relaxed);
}

/// The backdrop showing behind the sidebar this frame, or None for the solid sidebar.
pub fn live(pref: bool) -> Option<Backdrop> {
    if let Some(kind) = preview() {
        return pref.then_some(kind);
    }
    let l = launched()?;
    if !pref || !l.window || SURFACE.load(Ordering::Relaxed) == 2 || REFUSED.load(Ordering::Relaxed) {
        return None;
    }
    now().ok()
}

/// Whether the window was made see-through at launch (the scene preview counts).
pub fn window_glass() -> bool {
    preview().is_some() || launched().is_some_and(|l| l.window)
}

/// The Settings row.
pub fn row_state(pref: bool) -> Row {
    let surface = match SURFACE.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    };
    row(rune::is_headless(), pref, window_glass(), now(), surface, REFUSED.load(Ordering::Relaxed))
}

/// A scene shows the glass over a painted wallpaper (headless snapshots only).
pub fn set_preview(kind: Option<Backdrop>) {
    PREVIEW.with(|p| p.set(kind));
}

pub fn preview() -> Option<Backdrop> {
    PREVIEW.with(|p| p.get())
}

/// `main.rs`: how to wake the loop when the first frame shows the surface went opaque.
pub fn set_waker(wake: impl Fn() + 'static) {
    WAKE.with(|w| *w.borrow_mut() = Some(Box::new(wake)));
}

/// The app's poll: true once after something here changed outside a handler.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

fn mark_dirty() {
    DIRTY.store(true, Ordering::Relaxed);
    WAKE.with(|w| {
        if let Some(wake) = w.borrow().as_ref() {
            wake();
        }
    });
}

/// `App::on_window`: the native window exists. Puts the backdrop on before the first frame and
/// logs what it did.
pub fn on_window(handle: &rune::WindowHandle) {
    WINDOW.with(|w| *w.borrow_mut() = Some(handle.clone()));
    APPLIED.with(|a| a.set(None));
    let Some(l) = launched() else { return };
    if !l.window {
        log_once(if l.pref {
            match choose(&l.probe) {
                Err(why) => format!("Sidebar glass: solid, {}. {}", why.name(), system(&l.probe)),
                Ok(_) => format!("Sidebar glass: solid. {}", system(&l.probe)),
            }
        } else {
            format!("Sidebar glass: off. {}", system(&l.probe))
        });
        return;
    }
    let dark = !crate::ui::Ui::new().light;
    let kind = live(l.pref);
    apply(kind, dark);
    let line = match (kind, REFUSED.load(Ordering::Relaxed)) {
        (Some(k), false) => format!("Sidebar glass: {} applied. {}", k.name(), system(&l.probe)),
        (_, true) => format!("Sidebar glass: solid, {}. {}", Why::Refused.name(), system(&l.probe)),
        (None, false) => format!("Sidebar glass: solid. {}", system(&l.probe)),
    };
    log_once(line);
}

/// `App::on_frame_drawn`: whether the surface really composites alpha. Read once; an opaque
/// surface turns the glass off and redraws.
pub fn frame_drawn(transparent: bool) {
    if SURFACE.load(Ordering::Relaxed) != 0 {
        return;
    }
    let Some(l) = launched() else { return };
    if !l.window {
        SURFACE.store(1, Ordering::Relaxed);
        return;
    }
    SURFACE.store(if transparent { 1 } else { 2 }, Ordering::Relaxed);
    if !transparent {
        crate::screens::settings::log::info(format!(
            "Sidebar glass: solid, {}. {}",
            Why::Surface.name(),
            system(&l.probe)
        ));
        apply(None, true);
        mark_dirty();
    }
}

/// The window came to the front: Windows' transparency effects may have changed meanwhile.
pub fn recheck() {
    #[cfg(windows)]
    {
        let on = win::transparency_effects();
        if TRANSPARENCY.swap(on, Ordering::Relaxed) != on {
            mark_dirty();
        }
    }
}

/// The view, each frame: hand the system the backdrop the sidebar shows now. Calls the system
/// only when it changed (the setting, the theme, contrast or transparency), so a frame costs a
/// compare.
pub fn sync(pref: bool, kind: Option<Backdrop>, dark: bool) {
    PREF.store(pref, Ordering::Relaxed);
    if rune::is_headless() || !launched().is_some_and(|l| l.window) {
        return;
    }
    if APPLIED.with(|a| a.get()) == Some((kind, dark)) {
        return;
    }
    apply(kind, dark);
}

fn apply(kind: Option<Backdrop>, dark: bool) {
    APPLIED.with(|a| a.set(Some((kind, dark))));
    let ok = WINDOW.with(|w| match w.borrow().as_ref() {
        Some(handle) => apply_to(handle, kind, dark),
        None => true,
    });
    if !ok && kind.is_some() {
        REFUSED.store(true, Ordering::Relaxed);
        mark_dirty();
    }
}

#[allow(unused_variables)]
fn apply_to(handle: &rune::WindowHandle, kind: Option<Backdrop>, dark: bool) -> bool {
    use rune::raw_window_handle::RawWindowHandle;
    let raw = handle.with_raw_handle(|h| h.as_raw());
    match raw {
        #[cfg(windows)]
        Ok(RawWindowHandle::Win32(h)) => win::apply(h.hwnd.get() as _, kind, dark),
        #[cfg(target_os = "macos")]
        Ok(RawWindowHandle::AppKit(h)) => {
            // The material follows the window's appearance: match it to Sigil's theme while the
            // glass shows, so a dark sidebar never sits on light vibrancy. Back to the system's
            // appearance otherwise.
            mac::appearance(h.ns_view.as_ptr(), kind.map(|_| dark));
            true
        }
        _ => true,
    }
}

fn system(p: &Probe) -> String {
    match p.os {
        Os::Windows => format!("Windows build {}.", p.build),
        Os::MacOs => "macOS.".to_string(),
        Os::Other => format!("{}.", std::env::consts::OS),
    }
}

fn log_once(line: String) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        crate::screens::settings::log::info(line);
    }
}

/// One line for Send feedback and Send diagnostics: what the sidebar shows and why.
pub fn report() -> String {
    let Some(l) = launched() else { return "Sidebar glass: unknown".to_string() };
    let pref = PREF.load(Ordering::Relaxed);
    let sys = system(&l.probe);
    let state = if !pref {
        "off".to_string()
    } else if let Some(kind) = live(pref) {
        format!("on, {}", kind.name())
    } else {
        match row_state(pref) {
            Row::Unavailable(why) => format!("solid, {}", why.name()),
            Row::Restart => "solid until restart".to_string(),
            Row::Ready => "solid".to_string(),
        }
    };
    format!("Sidebar glass: {state} ({sys})")
}

#[cfg(windows)]
mod win {
    //! The Win32 side: what the system offers, and the backdrop on and off.
    use super::{Backdrop, Os, Probe};
    use windows_sys::Win32::Foundation::{BOOL, HWND};
    use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::Controls::MARGINS;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION, SPI_GETHIGHCONTRAST, SystemParametersInfoW};

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    const DWMWA_MICA_EFFECT: u32 = 1029;
    const DWMSBT_AUTO: i32 = 0;
    const DWMSBT_MAINWINDOW: i32 = 2;
    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_BLURBEHIND: u32 = 3;

    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }

    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut core::ffi::c_void,
        size: usize,
    }

    type SetComposition = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn set_composition() -> Option<SetComposition> {
        let user32 = wide("user32.dll");
        // SAFETY: user32 is loaded in every GUI process; the export, when present, has this
        // signature on every Windows 10 and 11 build.
        unsafe {
            let module = GetModuleHandleW(user32.as_ptr());
            if module.is_null() {
                return None;
            }
            let f = GetProcAddress(module, c"SetWindowCompositionAttribute".as_ptr().cast())?;
            Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, SetComposition>(f))
        }
    }

    /// The Windows build from the registry (not shimmed by compatibility modes, unlike
    /// GetVersionEx).
    fn build() -> u32 {
        let (sub, name) = (wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), wide("CurrentBuildNumber"));
        let mut buf = [0u16; 32];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: NUL-terminated strings; `buf` holds `bytes` bytes.
        let rc = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        if rc != 0 {
            return 0;
        }
        String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]).trim().parse().unwrap_or(0)
    }

    /// Settings > Personalization > Colors > Transparency effects. On when the value is missing.
    pub fn transparency_effects() -> bool {
        let (sub, name) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), wide("EnableTransparency"));
        let mut value = 1u32;
        let mut bytes = 4u32;
        // SAFETY: NUL-terminated strings; `value` holds 4 bytes.
        let rc = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&raw mut value).cast(), &mut bytes)
        };
        rc != 0 || value != 0
    }

    fn high_contrast() -> bool {
        let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, dwFlags: 0, lpszDefaultScheme: std::ptr::null_mut() };
        // SAFETY: SPI_GETHIGHCONTRAST fills a HIGHCONTRASTW whose cbSize is set.
        let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&raw mut hc).cast(), 0) };
        ok != 0 && hc.dwFlags & HCF_HIGHCONTRASTON != 0
    }

    pub fn probe() -> Probe {
        Probe {
            os: Os::Windows,
            build: build(),
            accent_api: set_composition().is_some(),
            transparency: transparency_effects(),
            high_contrast: high_contrast(),
            // SAFETY: a plain metric query.
            remote: unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0,
        }
    }

    fn attr<T>(hwnd: HWND, attribute: u32, value: &T) -> bool {
        // SAFETY: `value` is the attribute's documented type and size.
        unsafe { DwmSetWindowAttribute(hwnd, attribute, (value as *const T).cast(), std::mem::size_of::<T>() as u32) >= 0 }
    }

    fn extend(hwnd: HWND, all: bool) -> bool {
        let m = if all { -1 } else { 0 };
        let margins = MARGINS { cxLeftWidth: m, cxRightWidth: m, cyTopHeight: m, cyBottomHeight: m };
        // SAFETY: a live window of this thread and a MARGINS on the stack.
        unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) >= 0 }
    }

    fn accent(hwnd: HWND, state: u32) -> bool {
        let Some(set) = set_composition() else { return false };
        let mut policy = AccentPolicy { state, flags: 0, gradient: 0, animation: 0 };
        let mut data = CompositionData { attribute: WCA_ACCENT_POLICY, data: (&raw mut policy).cast(), size: std::mem::size_of::<AccentPolicy>() };
        // SAFETY: the documented-by-use layout of WINDOWCOMPOSITIONATTRIBDATA and ACCENT_POLICY.
        unsafe { set(hwnd, &mut data) != 0 }
    }

    /// Puts `kind` behind the window, or takes every backdrop off for None. Dark mode first,
    /// so Mica's own tint matches Sigil's theme. True when the backdrop went on.
    pub fn apply(hwnd: HWND, kind: Option<Backdrop>, dark: bool) -> bool {
        let dark_mode: BOOL = dark.into();
        attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_mode);
        let build = build();
        match kind {
            Some(Backdrop::Mica) => extend(hwnd, true) && attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_MAINWINDOW),
            Some(Backdrop::MicaLegacy) => extend(hwnd, true) && attr(hwnd, DWMWA_MICA_EFFECT, &BOOL::from(true)),
            Some(Backdrop::Blur) => accent(hwnd, ACCENT_ENABLE_BLURBEHIND),
            Some(Backdrop::Vibrancy) => false,
            None => {
                if build >= super::BUILD_BACKDROP_TYPE {
                    attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_AUTO);
                } else if build >= super::BUILD_WINDOWS_11 {
                    attr(hwnd, DWMWA_MICA_EFFECT, &BOOL::from(false));
                } else {
                    accent(hwnd, ACCENT_DISABLED);
                }
                extend(hwnd, false);
                true
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! The window's appearance through the Objective-C runtime (the crate links no objc
    //! bindings of its own).
    use std::ffi::{c_char, c_void};

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSAppearanceNameDarkAqua: *mut c_void;
        static NSAppearanceNameAqua: *mut c_void;
    }

    /// Dark or light aqua for the view's window, or None for the system's.
    pub fn appearance(ns_view: *mut c_void, dark: Option<bool>) {
        if ns_view.is_null() {
            return;
        }
        type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
        type Send1 = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
        // SAFETY: objc_msgSend called through the exact signature of each method: -[NSView
        // window], +[NSAppearance appearanceNamed:], -[NSWindow setAppearance:]. On the main
        // thread, where Rune calls the app's window hooks and its view.
        unsafe {
            let send0: Send0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send1: Send1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let window = send0(ns_view, sel_registerName(c"window".as_ptr()));
            if window.is_null() {
                return;
            }
            let look = match dark {
                Some(dark) => {
                    let class = objc_getClass(c"NSAppearance".as_ptr());
                    let name = if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua };
                    if class.is_null() || name.is_null() {
                        return;
                    }
                    send1(class, sel_registerName(c"appearanceNamed:".as_ptr()), name)
                }
                None => std::ptr::null_mut(),
            };
            send1(window, sel_registerName(c"setAppearance:".as_ptr()), look);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(build: u32) -> Probe {
        Probe { os: Os::Windows, build, accent_api: true, transparency: true, high_contrast: false, remote: false }
    }

    #[test]
    fn windows_builds_pick_their_backdrop() {
        assert_eq!(choose(&windows(22631)), Ok(Backdrop::Mica), "11 23H2");
        assert_eq!(choose(&windows(26100)), Ok(Backdrop::Mica), "11 24H2");
        assert_eq!(choose(&windows(22621)), Ok(Backdrop::Mica), "11 22H2");
        assert_eq!(choose(&windows(22523)), Ok(Backdrop::Mica), "first build with the backdrop type");
        assert_eq!(choose(&windows(22522)), Ok(Backdrop::MicaLegacy));
        assert_eq!(choose(&windows(22000)), Ok(Backdrop::MicaLegacy), "11 21H2");
        assert_eq!(choose(&windows(19045)), Ok(Backdrop::Blur), "10 22H2");
        assert_eq!(choose(&windows(10240)), Ok(Backdrop::Blur), "10 1507");
        assert_eq!(choose(&windows(9600)), Err(Why::OldWindows), "8.1");
        assert_eq!(choose(&windows(0)), Err(Why::OldWindows), "unknown build");
        let no_api = Probe { accent_api: false, ..windows(19045) };
        assert_eq!(choose(&no_api), Err(Why::OldWindows), "10 without the accent API");
        // Windows 11 does not need the accent API.
        assert_eq!(choose(&Probe { accent_api: false, ..windows(22631) }), Ok(Backdrop::Mica));
    }

    #[test]
    fn windows_settings_fall_back_to_solid() {
        let off = Probe { transparency: false, ..windows(22631) };
        assert_eq!(choose(&off), Err(Why::TransparencyOff));
        let hc = Probe { high_contrast: true, ..windows(22631) };
        assert_eq!(choose(&hc), Err(Why::HighContrast));
        let rdp = Probe { remote: true, transparency: false, ..windows(22631) };
        assert_eq!(choose(&rdp), Err(Why::Remote), "remote wins over a setting the player can change");
        // An old Windows says so even with every setting off.
        let old = Probe { remote: true, high_contrast: true, transparency: false, ..windows(9600) };
        assert_eq!(choose(&old), Err(Why::OldWindows));
    }

    #[test]
    fn other_systems() {
        let mac = Probe { os: Os::MacOs, build: 0, accent_api: false, transparency: true, high_contrast: false, remote: false };
        assert_eq!(choose(&mac), Ok(Backdrop::Vibrancy));
        assert_eq!(choose(&Probe { high_contrast: true, ..mac }), Err(Why::HighContrast));
        assert_eq!(choose(&Probe { os: Os::Other, ..mac }), Err(Why::Platform));
    }

    #[test]
    fn the_row_says_what_holds() {
        let ok = Ok(Backdrop::Mica);
        assert_eq!(row(false, false, false, ok, None, false), Row::Ready, "off, window opaque: a plain switch");
        assert_eq!(row(false, true, false, ok, None, false), Row::Restart, "on since launch: after restart");
        assert_eq!(row(false, true, true, ok, Some(true), false), Row::Ready);
        assert_eq!(row(false, false, true, ok, Some(true), false), Row::Ready, "off at runtime applies at once");
        assert_eq!(row(false, true, true, ok, Some(false), false), Row::Unavailable(Why::Surface));
        assert_eq!(row(false, true, true, ok, Some(true), true), Row::Unavailable(Why::Refused));
        assert_eq!(row(false, true, true, Err(Why::TransparencyOff), Some(true), false), Row::Unavailable(Why::TransparencyOff));
        assert_eq!(row(false, false, false, Err(Why::OldWindows), None, false), Row::Unavailable(Why::OldWindows));
        assert_eq!(row(true, true, false, Err(Why::Platform), None, false), Row::Ready, "headless scenes show the switch");
    }

    /// Text in the sidebar clears AA on any backdrop each material can show. Compositors blend
    /// in gamma space: the window's premultiplied pixel (its linear colour times alpha, then
    /// encoded) plus the backdrop times one less alpha.
    #[test]
    fn glass_text_clears_aa_on_any_backdrop() {
        use crate::ui::{Theme, Ui};
        use rune::color::{srgb_decode, srgb_encode};
        let lum = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let ratio = |a: f32, b: f32| (a.max(b) + 0.05) / (a.min(b) + 0.05);
        for theme in Theme::ALL {
            let u = Ui::themed(theme, None);
            for kind in [Backdrop::Mica, Backdrop::Blur, Backdrop::Vibrancy] {
                let g = u.glass(kind.tinted());
                let t = g.tint;
                // The gamma range of what the material can put behind the window. Mica and
                // vibrancy follow Sigil's theme (dark mode and the window appearance are set to
                // match), a plain blur shows the wallpaper as it is.
                // Mica dark over a white wallpaper comes to about 0.23 and dark popover vibrancy
                // to about 0.31, so 0.4 leaves room; the light materials mirror that.
                let range: [f32; 2] = match (kind.tinted(), u.light) {
                    (true, false) => [0.0, 0.4],
                    (true, true) => [0.65, 1.0],
                    (false, _) => [0.0, 1.0],
                };
                for back in range {
                    let mix = |c: f32| srgb_encode(c * t.a) + (1.0 - t.a) * back;
                    let ground = lum(srgb_decode(mix(t.r)), srgb_decode(mix(t.g)), srgb_decode(mix(t.b)));
                    for (name, ink, need) in [("ink", u.ink, 7.0), ("prose", u.prose, 4.5), ("muted", u.muted, 4.5)] {
                        let c = ratio(lum(ink.r, ink.g, ink.b), ground);
                        assert!(c >= need, "{theme:?} {kind:?} on {back}: {name} {c:.2} < {need}");
                    }
                }
            }
        }
    }
}

