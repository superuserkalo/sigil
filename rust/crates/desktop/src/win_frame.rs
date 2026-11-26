//! A guard against Windows' own title bar showing above Sigil's.
//!
//! Rune removes the system caption with a window subclass (`WM_NCCALCSIZE`). On some Windows 10
//! machines players saw both bars stacked, so after the window opens Sigil checks its own
//! windows: a top-level window with a caption style whose content starts more than a border
//! below its outer edge still has the system title bar. The guard drops `WS_CAPTION` from it
//! (the sizing border, snapping and the window menu stay) and asks for a new frame, then notes
//! it in the log with the Windows build. Where Rune's frame works the gap is at most a border and
//! nothing changes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Seconds after start when the guard looks: after the first frame, and once more after the
/// GPU hand-over.
const CHECKS: [f64; 2] = [1.0, 5.0];

/// A window was made (a start with no window opens one later, and a window hidden to the tray is
/// made again) or came back in sight: the checks start over (`rearm`).
static REARM: AtomicBool = AtomicBool::new(false);

/// From `App::on_window` and `App::on_visibility`: check again from now. A minimized window is
/// not checked (its frame is the system's minimized frame), so a check that found none waits for
/// this.
pub fn rearm() {
    REARM.store(true, Ordering::Relaxed);
}

pub struct Guard {
    started: Instant,
    done: usize,
}

impl Default for Guard {
    fn default() -> Self {
        Self { started: Instant::now(), done: 0 }
    }
}

impl Guard {
    /// Call from the app's poll; cheap, and does nothing after its last check. It asks for a
    /// wake at its next check, so an idle window still gets checked.
    pub fn poll(&mut self, tx: &mut rune::Tx) {
        if rune::is_headless() || !cfg!(windows) {
            return;
        }
        if REARM.swap(false, Ordering::Relaxed) {
            *self = Self::default();
        }
        let Some(&due) = CHECKS.get(self.done) else { return };
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed < due {
            tx.wake_after(std::time::Duration::from_secs_f64(due - elapsed));
            return;
        }
        // No window in sight to check (none made yet, or minimized): wait for `rearm`.
        #[cfg(windows)]
        if !imp::check() {
            self.done = CHECKS.len();
            return;
        }
        self.done += 1;
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GWL_STYLE, GetClientRect, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
        SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_CHILD,
    };

    /// Whether a window in sight was there to check.
    pub fn check() -> bool {
        let mut seen = 0isize;
        // SAFETY: the callback only reads and restyles windows of this thread, and writes `seen`,
        // which outlives the call.
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(visit), (&raw mut seen) as LPARAM) };
        seen != 0
    }

    unsafe extern "system" fn visit(hwnd: HWND, seen: LPARAM) -> BOOL {
        // SAFETY: `hwnd` is a live window of this thread, handed to us by EnumThreadWindows, and
        // `seen` is `check`'s counter.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            // A minimized window's rectangles are the system's minimized frame, not Sigil's.
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 || style & WS_CHILD != 0 {
                return 1;
            }
            *(seen as *mut isize) += 1;
            if style & WS_CAPTION != WS_CAPTION {
                return 1;
            }
            let mut outer = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let mut inner = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if GetWindowRect(hwnd, &mut outer) == 0 || GetClientRect(hwnd, &mut inner) == 0 {
                return 1;
            }
            let mut origin = POINT { x: 0, y: 0 };
            ClientToScreen(hwnd, &mut origin);
            let gap = origin.y - outer.top;
            // A border is a few pixels at most; a system title bar is about 23 px at 96 DPI.
            let limit = (12 * GetDpiForWindow(hwnd).max(96) / 96) as i32;
            if gap <= limit {
                return 1;
            }
            SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !WS_CAPTION) as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
            crate::screens::settings::log::info(format!(
                "Windows showed its own title bar above Sigil's ({gap} px); removed it. Windows build {}.",
                build()
            ));
        }
        1
    }

    /// The Windows build number (19045 is Windows 10 22H2), from the registry.
    fn build() -> String {
        use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (sub, name) = (wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), wide("CurrentBuild"));
        let mut buf = [0u16; 32];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: NUL-terminated strings; `buf` holds `bytes` bytes.
        let rc = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        if rc != 0 {
            return "unknown".to_string();
        }
        String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)])
    }
}
