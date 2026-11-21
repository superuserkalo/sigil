//! The text editor Sigil opens files with (gameinfo.gi, autoexec.cfg, configs), and the folder
//! file dialogs start in. Grimoire's picker (MIT) lists the system default, editors found on
//! this computer and Browse; Sigil adds the macOS app bundles Grimoire does not look for.

use std::path::{Path, PathBuf};
use std::process::Command;

use rune::{FileDialogOptions, Tx};

use super::i18n::{fill, tr};
use super::options;
use crate::app::State;

/// An editor found on this computer: a display name and the program or app bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub path: PathBuf,
}

/// Editors on PATH that Grimoire looks for on macOS and Linux.
const PATH_EDITORS: [(&str, &str); 11] = [
    ("gnome-text-editor", "GNOME Text Editor"),
    ("gedit", "gedit"),
    ("kate", "Kate"),
    ("kwrite", "KWrite"),
    ("mousepad", "Mousepad"),
    ("pluma", "Pluma"),
    ("geany", "Geany"),
    ("code", "Visual Studio Code"),
    ("codium", "VSCodium"),
    ("cursor", "Cursor"),
    ("subl", "Sublime Text"),
];

/// macOS app bundles worth offering.
const MAC_APPS: [&str; 9] = [
    "TextEdit",
    "Visual Studio Code",
    "Cursor",
    "Zed",
    "Sublime Text",
    "BBEdit",
    "CotEditor",
    "Nova",
    "VSCodium",
];

/// Display name for a stored editor path.
pub fn name_of(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    PATH_EDITORS
        .iter()
        .find(|(bin, _)| bin.eq_ignore_ascii_case(&stem))
        .map(|(_, name)| (*name).to_string())
        .unwrap_or(stem)
}

/// Editors installed here, without duplicates by name.
pub fn detect() -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let mut push = |name: String, path: PathBuf| {
        if !out.iter().any(|f| f.name == name) {
            out.push(Found { name, path });
        }
    };
    if cfg!(target_os = "macos") {
        for app in MAC_APPS {
            for root in ["/Applications".to_string(), format!("{}/Applications", home())] {
                let path = PathBuf::from(&root).join(format!("{app}.app"));
                if path.is_dir() {
                    push(app.to_string(), path);
                }
            }
        }
        let text_edit = PathBuf::from("/System/Applications/TextEdit.app");
        if text_edit.is_dir() {
            push("TextEdit".into(), text_edit);
        }
    }
    if cfg!(windows) {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let candidates = [
            ("Notepad", format!("{}\\System32\\notepad.exe", env("WINDIR"))),
            ("Notepad++", format!("{}\\Notepad++\\notepad++.exe", env("ProgramFiles"))),
            ("Notepad++", format!("{}\\Notepad++\\notepad++.exe", env("ProgramFiles(x86)"))),
            (
                "Visual Studio Code",
                format!("{}\\Programs\\Microsoft VS Code\\Code.exe", env("LOCALAPPDATA")),
            ),
            ("Visual Studio Code", format!("{}\\Microsoft VS Code\\Code.exe", env("ProgramFiles"))),
            ("Sublime Text", format!("{}\\Sublime Text\\sublime_text.exe", env("ProgramFiles"))),
        ];
        for (name, path) in candidates {
            let path = PathBuf::from(path);
            if path.is_file() {
                push(name.to_string(), path);
            }
        }
    } else {
        let paths = std::env::var_os("PATH").unwrap_or_default();
        for (bin, name) in PATH_EDITORS {
            if let Some(found) = std::env::split_paths(&paths)
                .map(|dir| dir.join(bin))
                .find(|p| is_executable(p))
            {
                push(name.to_string(), found);
            }
        }
    }
    out
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The command that opens `file` with `editor` (None: the system default for text).
pub fn command_for(editor: Option<&Path>, file: &Path) -> Command {
    match editor {
        Some(app) if app.extension().is_some_and(|e| e == "app") => {
            let mut c = Command::new("open");
            c.arg("-a").arg(app).arg(file);
            c
        }
        Some(program) => {
            let mut c = Command::new(program);
            c.arg(file);
            c
        }
        None => {
            #[cfg(target_os = "macos")]
            {
                let mut c = Command::new("open");
                c.arg("-t").arg(file);
                c
            }
            #[cfg(windows)]
            {
                let mut c = Command::new("notepad");
                c.arg(file);
                c
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                let mut c = Command::new("xdg-open");
                c.arg(file);
                c
            }
        }
    }
}

/// Open a text file in the chosen editor. Sigil only reads the file; the editor saves it.
pub fn open(st: &mut State, file: &Path) {
    if st.demo || rune::is_headless() {
        return;
    }
    let editor = options::get().editor.clone().map(PathBuf::from);
    if let Some(e) = &editor
        && !e.exists()
    {
        return super::say(
            st,
            fill(tr("{} is gone. Choose another editor in Settings."), &[&name_of(e)]),
            true,
        );
    }
    if let Err(error) = command_for(editor.as_deref(), file).spawn() {
        super::say(st, error.to_string(), true);
    }
}

/// Use an editor, or None for the system default.
pub fn choose(st: &mut State, path: Option<PathBuf>) {
    options::edit(st, |o| o.editor = path.map(|p| p.display().to_string()));
    st.settings.extra.dialog = None;
}

/// A file dialog for an editor program or app.
pub fn browse(_st: &mut State, tx: &mut Tx) {
    let mut opts = FileDialogOptions {
        title: tr("Choose an editor").into(),
        directory: Some(PathBuf::from(if cfg!(target_os = "macos") {
            "/Applications".to_string()
        } else {
            dialog_dir().map(|d| d.display().to_string()).unwrap_or_default()
        })),
        ..Default::default()
    };
    if cfg!(windows) {
        opts = opts.filter("Programs", &["exe"]);
    } else if cfg!(target_os = "macos") {
        opts = opts.filter("Apps", &["app"]);
    }
    tx.open_file(opts, |st: &mut State, result, _tx| match result {
        Ok(Some(path)) => choose(st, Some(path)),
        Ok(None) => {}
        Err(error) => super::say(st, error.to_string(), true),
    });
}

/// The label for the current choice.
pub fn current_label() -> String {
    match options::get().editor.clone() {
        Some(path) => name_of(Path::new(&path)),
        None => tr("System default").to_string(),
    }
}

// --- the folder file dialogs open in ----------------------------------------------------------

/// The folder the last file dialog picked from, when it still exists.
pub fn dialog_dir() -> Option<PathBuf> {
    options::get()
        .last_dialog_dir
        .clone()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Remember the folder of a picked file or folder.
pub fn remember(picked: &Path) {
    let dir = if picked.is_dir() {
        picked.to_path_buf()
    } else {
        match picked.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return,
        }
    };
    let dir = dir.display().to_string();
    if options::get().last_dialog_dir.as_deref() != Some(dir.as_str()) {
        options::edit_quiet(|o| o.last_dialog_dir = Some(dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_known_binaries() {
        assert_eq!(name_of(Path::new("/usr/bin/code")), "Visual Studio Code");
        assert_eq!(name_of(Path::new("/Applications/Zed.app")), "Zed");
        assert_eq!(name_of(Path::new("/opt/notepad++.exe")), "notepad++");
    }

    #[test]
    fn app_bundles_open_through_open_a() {
        let c = command_for(Some(Path::new("/Applications/Zed.app")), Path::new("/tmp/a.cfg"));
        assert_eq!(c.get_program(), "open");
        let args: Vec<_> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-a", "/Applications/Zed.app", "/tmp/a.cfg"]);
        let c = command_for(Some(Path::new("/usr/bin/kate")), Path::new("/tmp/a.cfg"));
        assert_eq!(c.get_program(), "/usr/bin/kate");
    }
}
