//! The archive picker for an archive that holds several `pakNN_dir.vpk` files (a model and its
//! voice pack, a HUD in parts), Grimoire's multi-file install (MIT).
//!
//! Every file is checked by default; the player unchecks what they do not want and sets the
//! order with the arrows. The press on Install is the approval: core's `install_vpks` plans and
//! writes each checked file in that order, the first into the lowest free slot, so it wins where
//! two of them change the same file. It stops at the first failure and keeps what went in. Each
//! installed slot gets a library entry, as a single-file install does, so Installed groups them
//! into one card whose files can be on together.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rune::{Color, El, col, icon, row, spacer, text};

use super::*;
use crate::screens::settings::i18n::{fill, tr};

/// What the multi-file worker reports.
pub struct Done {
    pub slots: Vec<u16>,
    pub error: Option<String>,
}

/// The paths to install, in order: the archive's list without the unchecked ones.
pub fn chosen(vpks: &[VpkChoice], unchecked: &BTreeSet<String>) -> Vec<String> {
    vpks.iter().filter(|c| !unchecked.contains(&c.path)).map(|c| c.path.clone()).collect()
}

/// `vpks` with the file at `index` one place up or down. Unchanged at either end.
pub fn nudge(vpks: &mut [VpkChoice], index: usize, up: bool) {
    let to = if up { index.checked_sub(1) } else { Some(index + 1) };
    if let Some(to) = to
        && to < vpks.len()
        && index < vpks.len()
    {
        vpks.swap(index, to);
    }
}

/// The picker inside the install dialog. `None` unless the archive holds several files.
pub fn picker(s: &State, u: &Ui) -> Option<El> {
    let v = &s.install;
    if v.vpks.len() < 2 {
        return None;
    }
    let busy = v.multi.is_running();
    let mut c = col().w_full().gap(4.0).child(
        row()
            .w_full()
            .center_y()
            .mb(4.0)
            .child(label(u, tr("Files in this archive")))
            .child(spacer())
            .child(text(tr("Higher in the list wins")).font(SMALL).color(u.faint)),
    );
    let count = v.vpks.len();
    for (index, choice) in v.vpks.iter().enumerate() {
        let path = choice.path.clone();
        let on = !v.unchecked.contains(&path);
        let toggle = path.clone();
        let mut line = row()
            .key(format!("vpk-choice-{}", choice.name()))
            .w_full()
            .min_h(44.0)
            .px(10.0)
            .py(6.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .fill(if on { u.accent.alpha(0.06) } else { Color::WHITE.alpha(0.02) })
            .border(1.0, if on { u.accent.alpha(0.4) } else { u.hair_strong })
            .child(
                row()
                    .key(format!("vpk-check-{}", choice.name()))
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(10.0)
                    .center_y()
                    .focusable(true)
                    .role("checkbox")
                    .checked(on)
                    .label(fill(tr("Install {}"), &[choice.name()]))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let unchecked = &mut st.install.unchecked;
                        if !unchecked.remove(&toggle) {
                            unchecked.insert(toggle.clone());
                        }
                    })
                    .child(crate::screens::installed::check_box_el(u, on))
                    .child(icon(&FILE_ARCHIVE, 16.0).color(u.muted).a11y_hidden())
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(2.0)
                            .child(text(choice.path.clone()).font(BODY).color(if on { u.ink } else { u.muted }).truncate())
                            .child(
                                text(format!(
                                    "{} · {}",
                                    crate::model::format_size(choice.bytes),
                                    plural(choice.chunks, "chunk", "chunks")
                                ))
                                .font(SMALL)
                                .color(u.muted),
                            ),
                    ),
            );
        for (up, glyph, words) in [(true, &icons::ARROW_UP, tr("Load earlier")), (false, &ARROW_DOWN, tr("Load later"))] {
            let end = if up { index == 0 } else { index + 1 == count };
            let mut b = u
                .icon_button(glyph, words)
                .key(format!("vpk-{}-{}", if up { "up" } else { "down" }, choice.name()));
            b = if end {
                b.disabled(true).opacity(0.35)
            } else {
                b.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| nudge(&mut st.install.vpks, index, up))
            };
            line = line.child(b);
        }
        c = c.child(line);
    }
    let picked = chosen(&v.vpks, &v.unchecked).len();
    let ready = picked > 0 && !busy && s.game_detected;
    let words = if busy {
        tr("Installing").to_string()
    } else if picked == 1 {
        tr("Install 1 file").to_string()
    } else {
        fill(tr("Install {} files"), &[&picked.to_string()])
    };
    c = c.child(
        row().w_full().pt(8.0).child(spacer()).child(
            u.icon_action(&icons::DOWNLOAD, &words, ActionKind::Primary)
                .key("vpk-install")
                .disabled(!ready)
                .opacity(if ready { 1.0 } else { 0.45 })
                .cursor(rune::Cursor::Pointer)
                .on_press(start),
        ),
    );
    Some(c)
}

/// Install the checked files on a worker. The press is the approval.
pub fn start(st: &mut State) {
    if st.install.multi.is_running() {
        return;
    }
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let paths = chosen(&st.install.vpks, &st.install.unchecked);
    if paths.is_empty() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.install.log = Some(tr("Set the game folder in Settings to install.").to_string());
        st.install.tone = Tone::Bad;
        return;
    };
    let card = card_by_id(st, st.install.mod_id);
    let work = Work {
        target: st.install.target.trim().to_string(),
        one_click: st.install.one_click,
        mod_id: st.install.mod_id,
        name: st.install.label.trim().to_string(),
        author: card.map(|c| c.author.clone()).filter(|a| !a.is_empty()),
        thumbnail: card.and_then(|c| c.thumbnail_url.clone()),
        game_root,
        workspace: crate::data::workspace_dir(),
        library: deadlock_core::import::library_path(&crate::data::data_dir()),
        land_disabled: st.prefs.land_disabled,
        paths,
    };
    st.install.log = None;
    st.install.multi.start(st.wake.clone(), move || Ok(install(work)));
}

struct Work {
    target: String,
    one_click: bool,
    mod_id: u64,
    name: String,
    author: Option<String>,
    thumbnail: Option<String>,
    game_root: PathBuf,
    workspace: PathBuf,
    library: PathBuf,
    land_disabled: bool,
    paths: Vec<String>,
}

fn install(work: Work) -> Done {
    use deadlock_core::install::{ApplyOptions, InstallRequest, PlanOptions, install_vpks};
    let request = match InstallRequest::new(work.target.clone(), work.workspace.clone()) {
        Ok(request) => request.with_one_click(work.one_click),
        Err(error) => return Done { slots: Vec::new(), error: Some(error.to_string()) },
    };
    let options = PlanOptions { game_root: Some(work.game_root.clone()), ..Default::default() };
    let result = install_vpks(&request, &work.paths, &options, ApplyOptions { land_disabled: work.land_disabled }, true);
    let mut slots = Vec::new();
    for (plan, record) in &result.installed {
        slots.push(plan.slot);
        let meta = deadlock_core::import::ModMeta {
            slot: plan.slot,
            size_bytes: std::fs::metadata(&record.target).map(|m| m.len()).unwrap_or(0),
            name: work.name.clone(),
            author: work.author.clone(),
            thumbnail_url: work.thumbnail.clone(),
            gamebanana_id: (work.mod_id > 0).then_some(work.mod_id),
            source: "browse".to_string(),
            recorded_at: String::new(),
        };
        if !work.name.is_empty() {
            let _ = deadlock_core::import::record_meta(&work.library, meta.clone());
        }
        crate::screens::settings::tools::imprint_after_install(&work.game_root, plan.slot, &meta);
    }
    Done { slots, error: result.error.map(|e| e.to_string()) }
}

/// Take the worker's result: say what went in, rescan, and carry on with a collection.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.install.multi.try_take() else {
        return false;
    };
    let done = result.unwrap_or_else(|error| Done { slots: Vec::new(), error: Some(error) });
    let name = st.install.label.clone();
    if !done.slots.is_empty() {
        st.installed.library = deadlock_core::import::read_library(&deadlock_core::import::library_path(
            &crate::data::data_dir(),
        ))
        .unwrap_or_default();
        st.installed.library_sig = None;
        crate::data::scan_addons_now(st);
        super::quick::refresh_owned(st);
    }
    let slots = done.slots.iter().map(|s| format!("{s:02}")).collect::<Vec<_>>().join(", ");
    match done.error {
        None => {
            st.install.open = false;
            crate::notify::push(
                st,
                crate::notify::Notice::success(fill(tr("Installed {} files of {}"), &[&done.slots.len().to_string(), &name]))
                    .detail(fill(tr("Slots {}."), &[&slots]))
                    .action(tr("Open Installed"), |st: &mut State| st.go(Screen::Installed)),
            );
            after_install(st, false, true, false);
        }
        Some(error) => {
            st.install.log = Some(if done.slots.is_empty() {
                error.clone()
            } else {
                fill(tr("Installed {} files, then stopped: {}"), &[&done.slots.len().to_string(), &error])
            });
            st.install.tone = Tone::Bad;
            crate::notify::push(st, crate::notify::Notice::warning(fill(tr("{} was not installed"), &[&name])).detail(error));
        }
    }
    true
}

/// Lucide arrow-down (ISC), drawn like `icons.rs`.
static ARROW_DOWN: rune::Icon = rune::Icon {
    name: "sigil-vpk-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(path: &str) -> VpkChoice {
        VpkChoice { path: path.to_string(), bytes: 1, chunks: 0 }
    }

    #[test]
    fn every_file_is_checked_until_unchecked() {
        let vpks = vec![choice("a/pak01_dir.vpk"), choice("b/pak01_dir.vpk"), choice("pak02_dir.vpk")];
        assert_eq!(chosen(&vpks, &BTreeSet::new()).len(), 3);
        let unchecked: BTreeSet<String> = ["b/pak01_dir.vpk".to_string()].into_iter().collect();
        assert_eq!(chosen(&vpks, &unchecked), vec!["a/pak01_dir.vpk".to_string(), "pak02_dir.vpk".to_string()]);
    }

    #[test]
    fn the_order_is_the_install_order() {
        let mut vpks = vec![choice("one"), choice("two"), choice("three")];
        nudge(&mut vpks, 2, true);
        assert_eq!(chosen(&vpks, &BTreeSet::new()), vec!["one", "three", "two"]);
        nudge(&mut vpks, 0, true);
        assert_eq!(vpks[0].path, "one", "the first cannot go up");
        nudge(&mut vpks, 2, false);
        assert_eq!(vpks[2].path, "two", "the last cannot go down");
    }
}
