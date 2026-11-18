//! View imprint (Grimoire's card menu item): who a mod says it is, read from the file itself.
//!
//! - Grimoire: the card menu shows the embedded identity: ids, hashes, merge sources and the raw
//!   `addoninfo.txt`.
//! - Deadlock Mod Manager: none.
//! - Sigil: More actions > View imprint reads, on a worker, the slot file's sha256 and size, the
//!   root `addoninfo.txt` and `modinfo.json` it carries (Settings > Imprint writes them), what
//!   Sigil's library and install journal know (GameBanana id, download link), and the sources
//!   when the mod is a merge. Nothing is written.

use std::path::{Path, PathBuf};

use rune::{El, Tx, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

/// What the file and Sigil's records say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    pub file: String,
    pub sha256: String,
    pub size: u64,
    pub addoninfo: Option<String>,
    pub modinfo: Option<String>,
    pub gamebanana_id: Option<u64>,
    pub download_url: Option<String>,
    /// "name (GameBanana id)" per merge source.
    pub merged_from: Vec<String>,
}

#[derive(Default)]
pub struct Imprint {
    pub slot: Option<u16>,
    pub name: String,
    pub job: Job<Read>,
    pub read: Option<Result<Read, String>>,
}

/// The two root files a VPK may carry about itself.
fn root_texts(path: &Path) -> (Option<String>, Option<String>) {
    use deadlock_core::vpk::{ReadVpkOptions, read_vpk_directory_with, read_vpk_entry_data};
    let Ok(directory) = read_vpk_directory_with(path, &ReadVpkOptions { keep_preload: false }) else {
        return (None, None);
    };
    let text_of = |want: &str| {
        let entry = directory.entries.iter().find(|e| e.path.eq_ignore_ascii_case(want))?;
        if entry.total_length > 64 * 1024 {
            return None;
        }
        read_vpk_entry_data(&directory, entry).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    (text_of("addoninfo.txt"), text_of("modinfo.json"))
}

pub fn open(st: &mut State, slot: u16) {
    st.installed.menu = None;
    let Some(addon) = st.addons.addons.iter().find(|a| a.slot == slot).cloned() else { return };
    let name = super::display_name(st, &addon);
    let gamebanana_id = super::meta_for(st, &addon).and_then(|m| m.gamebanana_id);
    st.installed.imprint = Imprint { slot: Some(slot), name, job: Job::default(), read: None };
    if st.demo || rune::is_headless() {
        return;
    }
    let path: PathBuf = addon.path(&super::addons_folder(st));
    let workspace = crate::data::workspace_dir();
    let data = crate::data::data_dir();
    let (file, size) = (addon.file.clone(), addon.size_bytes);
    st.installed.imprint.job.start(st.wake.clone(), move || {
        let sha256 = deadlock_core::identify::sha256_file(&path).map_err(|e| e.to_string())?;
        let (addoninfo, modinfo) = root_texts(&path);
        let download_url = deadlock_core::install::read_journal(&workspace)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|entry| entry.slot == slot && entry.vpk_bytes == size)
            .and_then(|entry| entry.download_url);
        let merged_from = deadlock_core::mod_merge::read_merged(&deadlock_core::mod_merge::merged_path(&data))
            .merged
            .into_iter()
            .filter(|m| m.slot == slot)
            .flat_map(|m| m.sources)
            .map(|source| match source.gamebanana_id {
                Some(id) => format!("{} (GameBanana {id})", source.name),
                None => source.name,
            })
            .collect();
        Ok(Read { file, sha256, size, addoninfo, modinfo, gamebanana_id, download_url, merged_from })
    });
}

pub fn poll(st: &mut State) -> bool {
    match st.installed.imprint.job.try_take() {
        Some(result) => {
            st.installed.imprint.read = Some(result);
            true
        }
        None => false,
    }
}

fn close(st: &mut State) {
    st.installed.imprint.slot = None;
}

fn pair(u: &Ui, key: &str, value: String) -> El {
    row()
        .w_full()
        .gap(12.0)
        .child(text(key.to_string()).font(SMALL).color(u.muted).w(150.0))
        .child(text(value).font(MONO).color(u.prose).wrap_anywhere().selectable(true).grow(1.0).min_w(0.0))
}

fn well(u: &Ui, key: &str, body: &str) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .max_h(160.0)
        .padding(10.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(text(body.to_string()).font(MONO).color(u.prose).wrap_anywhere().selectable(true).w_full())
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.installed.imprint;
    v.slot?;
    let mut body = col().w_full().gap(8.0);
    match &v.read {
        None => body = body.child(text(tr("Reading the file")).font(BODY).color(u.muted)),
        Some(Err(error)) => body = body.child(crate::notify::feedback::error_box(u, "imprint-error", error)),
        Some(Ok(r)) => {
            body = body
                .child(pair(u, tr("File"), r.file.clone()))
                .child(pair(u, tr("Size"), crate::model::format_size(r.size)))
                .child(pair(u, "SHA-256", r.sha256.clone()));
            if let Some(id) = r.gamebanana_id {
                body = body.child(pair(u, tr("GameBanana mod"), id.to_string()));
            }
            if let Some(url) = &r.download_url {
                body = body.child(pair(u, tr("Downloaded from"), url.clone()));
            }
            if !r.merged_from.is_empty() {
                body = body.child(pair(u, tr("Merged from"), r.merged_from.join(", ")));
            }
            match (&r.addoninfo, &r.modinfo) {
                (None, None) => {
                    body = body.child(
                        text(tr("The file carries no imprint. Settings > Tools > Imprint installed mods writes one."))
                            .font(SMALL)
                            .color(u.muted)
                            .wrap(),
                    )
                }
                (addoninfo, modinfo) => {
                    if let Some(t) = addoninfo {
                        body = body.child(text("addoninfo.txt").font(BODY_MED).color(u.ink).pt(6.0)).child(well(u, "imprint-addoninfo", t));
                    }
                    if let Some(t) = modinfo {
                        body = body.child(text("modinfo.json").font(BODY_MED).color(u.ink).pt(6.0)).child(well(u, "imprint-modinfo", t));
                    }
                }
            }
        }
    }
    let copy = match &v.read {
        Some(Ok(r)) => format!("{}\n{}\n{}", r.file, r.sha256, r.addoninfo.clone().unwrap_or_default()),
        _ => String::new(),
    };
    Some(
        u.dialog_shell()
            .key("installed-imprint")
            .role("dialog")
            .label(tr("Imprint"))
            .w(620.0)
            .max_h(680.0)
            .padding(24.0)
            .gap(14.0)
            .modal(true, close)
            .child(text(fill(tr("Imprint of {}"), &[&v.name])).font(H1).color(u.ink).wrap())
            .child(col().w_full().shrink().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(
                        u.action(tr("Copy"), ActionKind::Ghost)
                            .key("imprint-copy")
                            .disabled(copy.is_empty())
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |_st: &mut State, tx: &mut Tx| tx.copy(copy.clone())),
                    )
                    .child(spacer())
                    .child(u.action(tr("Close"), ActionKind::Primary).key("imprint-close").cursor(rune::Cursor::Pointer).on_press(close)),
            ),
    )
}

/// `w4a-imprint`: slot 03's imprint with both root files.
pub fn stage_scene(st: &mut State) {
    st.installed.imprint = Imprint {
        slot: Some(3),
        name: "Random Respawn Dances".to_string(),
        job: Job::default(),
        read: Some(Ok(Read {
            file: "pak03_dir.vpk".to_string(),
            sha256: "9f2c1e7a6b0d44c3a8e1f5b2c7d9e0a1b3c5d7e9f1a2b4c6d8e0f1a3b5c7d9e1".to_string(),
            size: 48_400_000,
            addoninfo: Some("\"AddonInfo\"\n{\n    addontitle \"Random Respawn Dances\"\n    addonauthor \"Bailey31\"\n    gamebananaId \"512345\"\n}".to_string()),
            modinfo: None,
            gamebanana_id: Some(512345),
            download_url: Some("https://gamebanana.com/dl/1300001".to_string()),
            merged_from: Vec::new(),
        })),
    };
}
