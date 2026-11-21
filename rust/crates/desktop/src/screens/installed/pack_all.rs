//! Pack all enabled into N (DMM's unreleased mod compression, behaviour only): every enabled
//! mod, past slot 99 too, is merged into two to eight shared VPKs, neighbouring slots together so
//! the load order holds, and each pack is switched on. A failure puts every mod back. The sources stay on disk switched off;
//! each pack appears in the merge list and Unmerge takes it apart again. Core:
//! `deadlock_core::mod_merge::pack_all`. Game install write, only from the dialog's button.

use deadlock_core::mod_merge::{self, MAX_PACKS, MergeInput, MergedMod, pack_groups};
use rune::{El, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, H1, SMALL, Ui};

pub struct PackAll {
    pub open: bool,
    pub packs: usize,
    pub error: Option<String>,
    pub job: Job<(Vec<MergedMod>, Option<String>)>,
}

impl Default for PackAll {
    fn default() -> Self {
        Self { open: false, packs: 4, error: None, job: Job::default() }
    }
}

fn enabled(s: &State) -> Vec<MergeInput> {
    let mut mods: Vec<MergeInput> = s
        .addons
        .addons
        .iter()
        .filter(|addon| addon.enabled)
        .map(|addon| MergeInput { slot: addon.slot, name: super::display_name(s, addon) })
        .collect();
    mods.sort_by_key(|m| m.slot);
    mods
}

pub fn open(st: &mut State) {
    st.installed.menu = None;
    let ui = &mut st.installed.tools.pack_all;
    ui.open = true;
    ui.error = None;
}

fn run(st: &mut State) {
    if crate::screens::settings::watch::refuse_while_running(st) {
        return;
    }
    let Some(game_root) = super::writable_game(st) else { return };
    let mods = enabled(st);
    let packs = st.installed.tools.pack_all.packs;
    let workspace = crate::data::workspace_dir();
    let store = mod_merge::merged_path(&crate::data::data_dir());
    let trash = deadlock_core::trash::trash_root(&crate::data::data_dir());
    let wake = st.wake.clone();
    st.installed.tools.pack_all.error = None;
    st.installed.tools.pack_all.job.start(wake, move || {
        let (made, error) = mod_merge::pack_all(&game_root, &workspace, &trash, &store, &mods, packs, true);
        Ok((made, error.map(|error| error.to_string())))
    });
}

pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.installed.tools.pack_all.job.try_take() else {
        return false;
    };
    match result {
        Ok((made, error)) => {
            if !made.is_empty() {
                let sources: usize = made.iter().map(|merged| merged.sources.len()).sum();
                st.status = fill(tr("Packed {} mods into {}."), &[&sources.to_string(), &made.len().to_string()]);
                st.installed.tools.merged = mod_merge::read_merged(&mod_merge::merged_path(&crate::data::data_dir()));
            }
            match error {
                Some(error) => st.installed.tools.pack_all.error = Some(error),
                None => st.installed.tools.pack_all.open = false,
            }
        }
        Err(error) => st.installed.tools.pack_all.error = Some(error),
    }
    super::rescan(st);
    true
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let ui = &s.installed.tools.pack_all;
    if !ui.open {
        return None;
    }
    let close = |st: &mut State| st.installed.tools.pack_all.open = false;
    let mods = enabled(s);
    let groups = pack_groups(&mods, ui.packs);
    let packed: usize = groups.iter().map(Vec::len).sum();
    let running = ui.job.is_running();

    let mut stepper = row().gap(8.0).center_y().child(text(tr("Packs")).font(BODY).color(u.prose)).child(spacer());
    for packs in 2..=MAX_PACKS {
        let on = ui.packs == packs;
        stepper = stepper.child(
            u.action(&packs.to_string(), if on { ActionKind::Primary } else { ActionKind::Ghost })
                .key(format!("pack-all-{packs}"))
                .label(fill(tr("{} packs"), &[&packs.to_string()]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.installed.tools.pack_all.packs = packs),
        );
    }

    let mut plan = col().w_full().gap(2.0);
    for (index, group) in groups.iter().enumerate() {
        let first = group.first().map(|m| m.slot).unwrap_or_default();
        let last = group.last().map(|m| m.slot).unwrap_or_default();
        plan = plan.child(
            row()
                .w_full()
                .h(28.0)
                .gap(8.0)
                .center_y()
                .child(text(fill(tr("Pack {}"), &[&(index + 1).to_string()])).font(BODY).color(u.ink).w(64.0))
                .child(
                    text(fill(tr("{} mods, slots {} to {}"), &[&group.len().to_string(), &deadlock_core::addons::slot_label(first), &deadlock_core::addons::slot_label(last)]))
                        .font(SMALL)
                        .color(u.muted)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if mods.len() > packed {
        plan = plan.child(
            text(fill(tr("{} stays as it is."), &[&(mods.len() - packed).to_string()]))
                .font(SMALL)
                .color(u.faint),
        );
    }

    let mut body = col()
        .w_full()
        .gap(12.0)
        .child(
            text(tr("Every enabled mod goes into a few shared VPKs, neighbouring slots together, so the load order stays the same. The mods stay on disk switched off, and Unmerge in the merge list takes a pack apart."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(stepper)
        .child(plan);
    if let Some(error) = &ui.error {
        body = body.child(crate::notify::feedback::error_line(u, "pack-all-error", error));
    }
    let ready = !groups.is_empty() && !running;
    let actions = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(spacer())
        .child(u.action(tr("Cancel"), ActionKind::Ghost).cursor(rune::Cursor::Pointer).on_press(close))
        .child(
            u.action(&if running { tr("Packing").to_string() } else { fill(tr("Pack {} mods"), &[&packed.to_string()]) }, ActionKind::Primary)
                .key("pack-all-go")
                .disabled(!ready)
                .cursor(rune::Cursor::Pointer)
                .on_press(run),
        );
    Some(
        u.dialog_shell()
            .key("installed-pack-all")
            .role("dialog")
            .label(tr("Pack all enabled mods"))
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .child(text(tr("Pack all enabled mods")).font(H1).color(u.ink))
            .child(body)
            .child(actions),
    )
}

/// Headless scene: the dialog over the demo library.
pub fn scene(st: &mut State) {
    crate::app::scene_demo_installed(st);
    st.installed.tools.pack_all.open = true;
    st.installed.tools.pack_all.packs = 3;
}
