//! Whether this copy of Sigil belongs to a package manager (Grimoire's "managed" install).
//!
//! A copy that winget, Scoop, Chocolatey, Homebrew or a Linux package installed is updated by
//! that manager: the in-app installer would fight it (or fail on a root-owned folder), so
//! Settings > Updates says where updates come from and hides the installer. The answer comes
//! from the executable's path only: nothing is run, read or asked of the manager.
//!
//! Grimoire treats `/opt`, `/usr`, `/nix/store`, `/snap`, `/var/lib/flatpak` and `/app` as
//! package-managed on Linux. Sigil keeps that list and adds the Windows managers' install
//! folders and Homebrew's Caskroom.

use std::path::Path;

/// Who updates this copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Winget,
    Scoop,
    Chocolatey,
    Homebrew,
    Nix,
    Snap,
    Flatpak,
    /// A system package (`/usr`, `/opt`): apt, dnf, pacman and the like.
    System,
}

impl Manager {
    /// The manager's own name, never translated. None for a system package.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Manager::Winget => Some("winget"),
            Manager::Scoop => Some("Scoop"),
            Manager::Chocolatey => Some("Chocolatey"),
            Manager::Homebrew => Some("Homebrew"),
            Manager::Nix => Some("Nix"),
            Manager::Snap => Some("Snap"),
            Manager::Flatpak => Some("Flatpak"),
            Manager::System => None,
        }
    }
}

/// The manager that owns an executable at `exe` on `os` (`std::env::consts::OS`), if any.
pub fn detect(os: &str, exe: &Path) -> Option<Manager> {
    let path = exe.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let has = |part: &str| path.contains(part);
    match os {
        "windows" => {
            if has("/winget/packages/") || has("/winget/links/") {
                Some(Manager::Winget)
            } else if has("/scoop/apps/") || has("/scoop/shims/") {
                Some(Manager::Scoop)
            } else if has("/chocolatey/lib/") || has("/chocolatey/bin/") {
                Some(Manager::Chocolatey)
            } else {
                None
            }
        }
        "macos" => {
            if has("/caskroom/") || path.starts_with("/opt/homebrew/") || has("/homebrew/cellar/") {
                Some(Manager::Homebrew)
            } else if path.starts_with("/nix/store/") {
                Some(Manager::Nix)
            } else {
                None
            }
        }
        "linux" => {
            if path.starts_with("/nix/store/") {
                Some(Manager::Nix)
            } else if path.starts_with("/snap/") {
                Some(Manager::Snap)
            } else if path.starts_with("/var/lib/flatpak/") || has("/.local/share/flatpak/") || path.starts_with("/app/") {
                Some(Manager::Flatpak)
            } else if path.starts_with("/home/linuxbrew/") {
                Some(Manager::Homebrew)
            } else if path.starts_with("/usr/") || path.starts_with("/opt/") {
                Some(Manager::System)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The manager that owns the running copy. A debug build never counts, so development runs
/// from `target/` keep the normal update rows.
pub fn current() -> Option<Manager> {
    if cfg!(debug_assertions) {
        // Debug snapshots: `SIGIL_PACKAGE_MANAGER_SCENE=scoop` and the like.
        return std::env::var("SIGIL_PACKAGE_MANAGER_SCENE").ok().and_then(|name| from_name(&name));
    }
    let exe = std::env::current_exe().ok()?;
    detect(std::env::consts::OS, &exe)
}

/// A manager by its lower-case name, for the debug scene switch.
fn from_name(name: &str) -> Option<Manager> {
    [
        Manager::Winget,
        Manager::Scoop,
        Manager::Chocolatey,
        Manager::Homebrew,
        Manager::Nix,
        Manager::Snap,
        Manager::Flatpak,
    ]
    .into_iter()
    .find(|m| m.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
    .or_else(|| (name == "system").then_some(Manager::System))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(path: &str) -> Option<Manager> {
        detect("windows", Path::new(path))
    }

    #[test]
    fn windows_managers_are_found_by_their_folders() {
        assert_eq!(
            win(r"C:\Users\ana\AppData\Local\Microsoft\WinGet\Packages\Sigil.Sigil_Microsoft.Winget.Source_8wekyb3d8bbwe\Sigil.exe"),
            Some(Manager::Winget)
        );
        assert_eq!(win(r"C:\Users\ana\AppData\Local\Microsoft\WinGet\Links\Sigil.exe"), Some(Manager::Winget));
        assert_eq!(win(r"C:\Program Files\WinGet\Packages\Sigil\Sigil.exe"), Some(Manager::Winget));
        assert_eq!(win(r"C:\Users\ana\scoop\apps\sigil\current\Sigil.exe"), Some(Manager::Scoop));
        assert_eq!(win(r"C:\ProgramData\scoop\apps\sigil\0.1.11\Sigil.exe"), Some(Manager::Scoop));
        assert_eq!(win(r"C:\ProgramData\chocolatey\lib\sigil\tools\Sigil.exe"), Some(Manager::Chocolatey));
    }

    #[test]
    fn a_release_zip_anywhere_else_updates_itself() {
        assert_eq!(win(r"C:\Users\ana\Desktop\Sigil\Sigil.exe"), None);
        assert_eq!(win(r"D:\Games\scoopy\Sigil.exe"), None);
        assert_eq!(detect("macos", Path::new("/Applications/Sigil.app/Contents/MacOS/Sigil")), None);
        assert_eq!(detect("linux", Path::new("/home/ana/Apps/Sigil/sigil")), None);
    }

    #[test]
    fn mac_and_linux_paths() {
        assert_eq!(
            detect("macos", Path::new("/opt/homebrew/Caskroom/sigil/0.1.11/Sigil.app/Contents/MacOS/Sigil")),
            Some(Manager::Homebrew)
        );
        assert_eq!(detect("linux", Path::new("/usr/bin/sigil")), Some(Manager::System));
        assert_eq!(detect("linux", Path::new("/opt/Sigil/sigil")), Some(Manager::System));
        assert_eq!(detect("linux", Path::new("/nix/store/abc-sigil/bin/sigil")), Some(Manager::Nix));
        assert_eq!(detect("linux", Path::new("/snap/sigil/12/sigil")), Some(Manager::Snap));
        assert_eq!(detect("linux", Path::new("/var/lib/flatpak/app/x/sigil")), Some(Manager::Flatpak));
        assert_eq!(detect("linux", Path::new("/app/bin/sigil")), Some(Manager::Flatpak));
        assert_eq!(Manager::System.name(), None);
        assert_eq!(Manager::Winget.name(), Some("winget"));
    }
}
