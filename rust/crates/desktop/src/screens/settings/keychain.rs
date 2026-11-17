//! Secrets in the OS credential store: the macOS keychain (`security`), the Windows
//! Credential Manager (`CredWriteW`) and the freedesktop Secret Service (`secret-tool`). A
//! secret never reaches a process argument list, a file Sigil writes, or the log: macOS and
//! Linux get it on stdin, Windows through the API.
//!
//! Sigil keeps one service name and an account per secret (`proxy` for the proxy password).

use std::io::Write;
use std::process::{Command, Stdio};

pub const SERVICE: &str = "app.sigil.desktop";

/// Quote a value for `security -i`, which reads shell-like words.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Whether a value can be stored at all: no line breaks or NULs, which every store's text
/// path would cut.
pub fn storable(value: &str) -> bool {
    !value.is_empty() && !value.contains(['\n', '\r', '\0']) && value.len() <= 1024
}

fn run_with_stdin(mut command: Command, input: &str) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        // stderr from these tools names the item, never the secret.
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Save a secret, replacing an older one.
pub fn set(account: &str, secret: &str) -> Result<(), String> {
    if !storable(secret) {
        return Err("That password cannot be stored.".into());
    }
    if rune::is_headless() || cfg!(test) {
        return Ok(());
    }
    platform::set(account, secret)
}

/// Read a secret. None when none is stored or the store is unavailable.
pub fn get(account: &str) -> Option<String> {
    if rune::is_headless() || cfg!(test) {
        return None;
    }
    platform::get(account).filter(|s| !s.is_empty())
}

/// Remove a secret. Removing one that does not exist is fine.
pub fn delete(account: &str) -> Result<(), String> {
    if rune::is_headless() || cfg!(test) {
        return Ok(());
    }
    platform::delete(account)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let script = format!(
            "add-generic-password -U -s {} -a {} -l {} -w {}\n",
            quote(SERVICE),
            quote(account),
            quote("Sigil"),
            quote(secret)
        );
        let mut command = Command::new("/usr/bin/security");
        command.arg("-i");
        run_with_stdin(command, &script).map(|_| ())
    }

    pub fn get(account: &str) -> Option<String> {
        let out = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let _ = Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", SERVICE, "-a", account])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let mut command = Command::new("secret-tool");
        command.args(["store", "--label=Sigil", "service", SERVICE, "account", account]);
        run_with_stdin(command, secret)
            .map(|_| ())
            .map_err(|e| if e.is_empty() { "No keyring is running.".into() } else { e })
    }

    pub fn get(account: &str) -> Option<String> {
        let out = Command::new("secret-tool")
            .args(["lookup", "service", SERVICE, "account", account])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let _ = Command::new("secret-tool")
            .args(["clear", "service", SERVICE, "account", account])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    //! Credential Manager through advapi32, declared by hand to avoid a new crate.
    use std::ffi::c_void;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct Credential {
        flags: u32,
        kind: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FileTime,
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredWriteW(credential: *const Credential, flags: u32) -> i32;
        fn CredReadW(target: *const u16, kind: u32, flags: u32, credential: *mut *mut Credential) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut c_void);
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn target(account: &str) -> Vec<u16> {
        wide(&format!("{}/{account}", super::SERVICE))
    }

    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        let mut name = target(account);
        let mut user = wide(account);
        let mut blob: Vec<u8> = secret.as_bytes().to_vec();
        let credential = Credential {
            flags: 0,
            kind: CRED_TYPE_GENERIC,
            target_name: name.as_mut_ptr(),
            comment: std::ptr::null_mut(),
            last_written: FileTime { low: 0, high: 0 },
            blob_size: blob.len() as u32,
            blob: blob.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: std::ptr::null_mut(),
            target_alias: std::ptr::null_mut(),
            user_name: user.as_mut_ptr(),
        };
        // SAFETY: every pointer points into a buffer that outlives the call.
        let ok = unsafe { CredWriteW(&credential, 0) };
        blob.iter_mut().for_each(|b| *b = 0);
        if ok != 0 {
            Ok(())
        } else {
            Err(format!("Credential Manager refused the password ({})", std::io::Error::last_os_error()))
        }
    }

    pub fn get(account: &str) -> Option<String> {
        let name = target(account);
        let mut out: *mut Credential = std::ptr::null_mut();
        // SAFETY: `out` receives a buffer the API allocates; CredFree releases it.
        unsafe {
            if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out) == 0 || out.is_null() {
                return None;
            }
            let cred = &*out;
            let bytes = std::slice::from_raw_parts(cred.blob, cred.blob_size as usize).to_vec();
            CredFree(out as *mut c_void);
            String::from_utf8(bytes).ok()
        }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let name = target(account);
        // SAFETY: a valid NUL-terminated wide string.
        unsafe {
            CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_escapes_quotes_and_backslashes() {
        assert_eq!(quote(r#"a b"c\d"#), r#""a b\"c\\d""#);
        assert_eq!(quote(""), r#""""#);
    }

    #[test]
    fn line_breaks_are_not_storable() {
        assert!(storable("hunter2"));
        assert!(!storable("two\nlines"));
        assert!(!storable(""));
    }
}
