//! Start Sigil when the user signs in to the computer.
//!
//! - macOS: a LaunchAgent plist in `~/Library/LaunchAgents` with `RunAtLoad`.
//! - Windows: a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written and
//!   removed with `reg.exe` (no registry crate).
//! - Linux: an XDG autostart entry in `~/.config/autostart`.
//!
//! The entry starts the running executable with `--minimized` when "Start minimized" is on.
//! Whether it is on is read back from the system, never assumed from settings.json.

use std::path::{Path, PathBuf};

pub const LABEL: &str = "app.sigil.desktop";
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Sigil";
/// The argument the login entry passes, read by [`started_minimized`].
pub const MINIMIZED_ARG: &str = "--minimized";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The LaunchAgent property list.
pub fn plist(exe: &Path, minimized: bool) -> String {
    let mut args = format!(
        "\t\t<string>{}</string>\n",
        xml_escape(&exe.display().to_string())
    );
    if minimized {
        args.push_str(&format!("\t\t<string>{MINIMIZED_ARG}</string>\n"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>{LABEL}</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
{args}\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
\t<key>ProcessType</key>\n\
\t<string>Interactive</string>\n\
</dict>\n\
</plist>\n"
    )
}

/// `reg.exe` arguments that set the Run value.
pub fn reg_add_args(exe: &Path, minimized: bool) -> Vec<String> {
    let mut command = format!("\"{}\"", exe.display());
    if minimized {
        command.push(' ');
        command.push_str(MINIMIZED_ARG);
    }
    vec![
        "add".into(),
        RUN_KEY.into(),
        "/v".into(),
        RUN_VALUE.into(),
        "/t".into(),
        "REG_SZ".into(),
        "/d".into(),
        command,
        "/f".into(),
    ]
}

#[allow(dead_code)]
pub fn reg_delete_args() -> Vec<String> {
    vec![
        "delete".into(),
        RUN_KEY.into(),
        "/v".into(),
        RUN_VALUE.into(),
        "/f".into(),
    ]
}

#[allow(dead_code)]
pub fn reg_query_args() -> Vec<String> {
    vec!["query".into(), RUN_KEY.into(), "/v".into(), RUN_VALUE.into()]
}

/// The XDG autostart desktop entry.
pub fn desktop_entry(exe: &Path, minimized: bool) -> String {
    let mut exec = format!("\"{}\"", exe.display().to_string().replace('"', "\\\""));
    if minimized {
        exec.push(' ');
        exec.push_str(MINIMIZED_ARG);
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=Sigil\nComment=Deadlock mod manager\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Where the macOS or Linux entry lives.
pub fn entry_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    } else if cfg!(windows) {
        None
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))
            .map(|c| c.join("autostart").join("sigil.desktop"))
    }
}

/// Whether a login entry for Sigil exists.
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        // A direct registry read: starting `reg.exe` here stalled the Settings sheet's opening
        // frame (process start plus antivirus scan) on the UI thread.
        return run_value_present();
    }
    #[cfg(not(windows))]
    entry_path().is_some_and(|p| p.exists())
}

/// Write (`Some(command)`) or remove (`None`) the `Sigil` value under the Run key.
/// Removing a value that is not there is fine.
#[cfg(windows)]
fn set_run_value(command: Option<String>) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let sub = wide(RUN_KEY.trim_start_matches(r"HKCU\"));
    let name = wide(RUN_VALUE);
    let rc = match command {
        Some(command) => {
            let data = wide(&command);
            // SAFETY: NUL-terminated strings; `data` holds the byte count passed.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    sub.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
        }
        // SAFETY: NUL-terminated strings.
        None => match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            rc => rc,
        },
    };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("Windows refused the startup entry (error {rc})."))
    }
}

/// True when HKCU\...\Run holds a `Sigil` value.
#[cfg(windows)]
fn run_value_present() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_ANY, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let sub = wide(RUN_KEY.trim_start_matches(r"HKCU\"));
    let name = wide(RUN_VALUE);
    // SAFETY: NUL-terminated strings; no data buffer is passed, so only presence is read.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ptr(),
            RRF_RT_ANY,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    rc == ERROR_SUCCESS
}

/// Add or remove the login entry for the running executable.
pub fn set(enabled: bool, minimized: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Direct registry calls on Windows: `reg.exe` stalled the switch for a process start.
    #[cfg(windows)]
    return set_run_value(enabled.then(|| reg_add_args(&exe, minimized).swap_remove(7)));
    #[cfg(not(windows))]
    set_entry(&exe, enabled, minimized)
}

/// The macOS LaunchAgent or the XDG autostart entry.
#[cfg(not(windows))]
fn set_entry(exe: &Path, enabled: bool, minimized: bool) -> Result<(), String> {
    let path = entry_path().ok_or("No home folder")?;
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let text = if cfg!(target_os = "macos") {
        plist(exe, minimized)
    } else {
        desktop_entry(exe, minimized)
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let partial = path.with_extension("tmp");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())
}

/// True when this process was started by the login entry with `--minimized`.
pub fn started_minimized() -> bool {
    std::env::args().any(|a| a == MINIMIZED_ARG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_runs_the_executable_at_load() {
        let p = plist(Path::new("/Applications/Sigil & Co.app/Contents/MacOS/sigil"), true);
        assert!(p.contains("<string>app.sigil.desktop</string>"));
        assert!(p.contains("<string>/Applications/Sigil &amp; Co.app/Contents/MacOS/sigil</string>"));
        assert!(p.contains("<string>--minimized</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(!plist(Path::new("/x"), false).contains("--minimized"));
    }

    #[test]
    fn reg_commands_target_the_user_run_key() {
        let add = reg_add_args(Path::new(r"C:\Program Files\Sigil\sigil.exe"), true);
        assert_eq!(add[0], "add");
        assert_eq!(add[1], RUN_KEY);
        assert_eq!(add[3], "Sigil");
        assert_eq!(add[7], r#""C:\Program Files\Sigil\sigil.exe" --minimized"#);
        assert_eq!(add.last().unwrap(), "/f");
        assert_eq!(reg_delete_args(), ["delete", RUN_KEY, "/v", "Sigil", "/f"]);
        assert!(RUN_KEY.starts_with("HKCU"));
    }

    #[test]
    fn the_desktop_entry_quotes_the_path() {
        let e = desktop_entry(Path::new("/opt/sigil/sigil"), false);
        assert!(e.contains("Exec=\"/opt/sigil/sigil\"\n"));
        assert!(e.starts_with("[Desktop Entry]\n"));
    }
}
