//! Game sounds: the hero's own sound events from the game files, played through the shared
//! preview player (game sound browser), and swapping them for the player's MP3s (swap a hero's
//! sounds).
//!
//! - Grimoire: a hero's gameplay sounds by ability (slot chip, ULT for the fourth), weapon,
//!   movement, melee and other, plus voice lines loaded on demand and capped; each row plays the
//!   first clip at the global preview volume and has Swap, which takes one MP3, a name and a loop
//!   choice, then builds and installs a mod at once.
//! - Deadlock Mod Manager: per-skin sound groups with play, a "Replaced" mark, a Yours / Original
//!   compare, and replacing takes MP3, WAV, OGG or FLAC; changes leave through its export sheet.
//! - Sigil: a Game sounds section on every hero page with the groups above, search over label and
//!   event, Play on every row (one sample at a time app-wide, the status bar's volume). Replace
//!   stages an MP3 per event; staged swaps can be played back, set to loop or not, or dropped.
//!   Build mod packs all of them into one VPK in app data and opens the import review, so the mod
//!   lands switched off like every install. Only MP3 is accepted, as the core writes MP3 clips.

use std::path::{Path, PathBuf};

use rune::{El, FileDialogOptions, Tx, col, row, text};

use super::assets::{self, Load, SoundGroup, SoundRow, Swap, play_id};
use super::{HeroCard, View};
use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, SMALL_NUM, Ui};

/// Rows a group shows before it asks for a search.
pub const GROUP_CAP: usize = 200;

/// The rows the section shows, grouped in order, after the search.
pub fn grouped(v: &View) -> Vec<(SoundGroup, Vec<&SoundRow>)> {
    let Some(list) = v.assets.sounds.ready() else { return Vec::new() };
    let query = v.assets.sound_query.text.trim().to_ascii_lowercase();
    let mut groups: Vec<(SoundGroup, Vec<&SoundRow>)> = Vec::new();
    for row in list {
        if !query.is_empty()
            && !row.label.to_ascii_lowercase().contains(&query)
            && !row.event.to_ascii_lowercase().contains(&query)
        {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == row.group) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((row.group.clone(), vec![row])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
}

/// Voice lines start folded (a long list); every other group starts open. `folded` holds the
/// groups the player flipped.
fn is_open(v: &View, group: &SoundGroup) -> bool {
    let flipped = v.assets.folded.contains(&group.title());
    let searching = !v.assets.sound_query.text.trim().is_empty();
    if searching {
        return true;
    }
    (*group == SoundGroup::Voice) == flipped
}

/// An MP3 by its first bytes: an ID3 tag or an MPEG audio frame sync.
pub fn is_mp3(head: &[u8]) -> bool {
    head.starts_with(b"ID3") || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0)
}

/// A WAV by its RIFF header. The core takes 16-bit PCM WAV only for clips the game stores
/// uncompressed and says so when it refuses one.
pub fn is_wav(head: &[u8]) -> bool {
    head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WAVE"
}

fn check_mp3(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut head = [0u8; 12];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..read];
    if (ext == "mp3" && is_mp3(head)) || (ext == "wav" && is_wav(head)) {
        Ok(())
    } else {
        Err(tr("Swaps take an MP3 (or a 16-bit WAV for sounds the game stores uncompressed). Convert OGG, FLAC and other audio to MP3 first.").to_string())
    }
}

/// Stage (or restage) a swap for one event.
pub fn stage(st: &mut State, row: &SoundRow, file: PathBuf) {
    if let Err(error) = check_mp3(&file) {
        st.locker.assets.swap_error = Some(error);
        return;
    }
    put(st, row, Some(file));
}

/// Stage a volume change of the game's own sound (no file), DMM's per-sound volume.
pub fn stage_volume(st: &mut State, row: &SoundRow) {
    if st.locker.assets.swaps.iter().any(|s| s.event == row.event) {
        return;
    }
    put(st, row, None);
    if let Some(swap) = st.locker.assets.swaps.iter_mut().find(|s| s.event == row.event) {
        swap.gain_db = -3.0;
    }
}

fn put(st: &mut State, row: &SoundRow, file: Option<PathBuf>) {
    let a = &mut st.locker.assets;
    a.swap_error = None;
    let old = a.swaps.iter().position(|s| s.event == row.event).map(|at| a.swaps.remove(at));
    a.swaps_open = true;
    a.swaps.push(Swap {
        event: row.event.clone(),
        label: row.label.clone(),
        soundevents: row.soundevents.clone(),
        clips: row.clips.clone(),
        file,
        looped: old.as_ref().and_then(|s| s.looped),
        gain_db: old.map_or(0.0, |s| s.gain_db),
    });
}

/// The volume steps a swap offers, in decibels (the core works in steps of about 1.5 dB).
pub const GAIN_STEP: f64 = 1.5;
pub const GAIN_MIN: f64 = -18.0;
pub const GAIN_MAX: f64 = 12.0;

fn nudge(st: &mut State, event: &str, steps: f64) {
    if let Some(swap) = st.locker.assets.swaps.iter_mut().find(|s| s.event == event) {
        swap.gain_db = (swap.gain_db + steps * GAIN_STEP).clamp(GAIN_MIN, GAIN_MAX);
    }
}

fn gain_label(db: f64) -> String {
    if db == 0.0 { "0 dB".to_string() } else { format!("{db:+.1} dB") }
}

fn pick(row: SoundRow) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |_st: &mut State, tx: &mut Tx| {
        let options = FileDialogOptions {
            title: fill(tr("Replace {}"), &[&row.label]),
            ..FileDialogOptions::default()
        }
        .filter("MP3 or WAV", &["mp3", "wav"]);
        let row = row.clone();
        tx.open_file(options, move |st: &mut State, result, _tx| match result {
            Ok(Some(path)) => stage(st, &row, path),
            Ok(None) => {}
            Err(error) => st.locker.assets.swap_error = Some(format!("{}: {error}", tr("The file picker could not open"))),
        });
    }
}

fn hero_name(st: &State) -> Option<String> {
    let super::Page::Hero(name) = &st.locker.page else { return None };
    Some(name.clone())
}

/// What the shared-clip check ran for: each swap's event and file.
pub fn signature(swaps: &[Swap]) -> Vec<String> {
    let mut sig: Vec<String> = swaps.iter().map(|w| format!("{}|{:?}", w.event, w.file)).collect();
    sig.sort();
    sig
}

/// The shared-clip result for the swaps staged now, if the check ran for exactly them.
fn shared_for(v: &View) -> Option<&Vec<(String, Vec<String>)>> {
    let (sig, list) = v.assets.shared.as_ref()?;
    (*sig == signature(&v.assets.swaps)).then_some(list)
}

/// "Also plays for Abrams, Bebop and 2 more."
pub fn also_plays(heroes: &[String]) -> String {
    match heroes.len() {
        0 => tr("Other heroes play these clips too.").to_string(),
        1..=3 => fill(tr("Also plays for {}."), &[&heroes.join(", ")]),
        n => fill(tr("Also plays for {} and {} more."), &[&heroes[..3].join(", "), &(n - 3).to_string()]),
    }
}

/// Build: first check whether another hero plays the same clips (warn, then Build anyway),
/// then pack.
fn build(st: &mut State) {
    if st.locker.assets.swaps.is_empty() || st.locker.assets.swap_job.is_running() || st.locker.assets.share_job.is_running() {
        return;
    }
    let sig = signature(&st.locker.assets.swaps);
    let checked = st.locker.assets.shared.as_ref().is_some_and(|(s, _)| *s == sig);
    if !checked && !st.demo && !rune::is_headless() {
        let Some(game_root) = st.game_root.clone() else {
            st.locker.assets.swap_error = Some(tr("Set the game folder in Settings first.").to_string());
            return;
        };
        let Some(display) = hero_name(st) else { return };
        let swaps = st.locker.assets.swaps.clone();
        st.locker.assets.swap_error = None;
        st.locker.assets.share_job.start(st.wake.clone(), move || {
            assets::api::shared_heroes(&game_root, &display, &swaps).map(|list| (signature(&swaps), list))
        });
        return;
    }
    build_now(st);
}

/// Pack the staged swaps on a worker.
fn build_now(st: &mut State) {
    if st.locker.assets.swaps.is_empty() || st.locker.assets.swap_job.is_running() {
        return;
    }
    let Some(display) = hero_name(st) else { return };
    let typed = st.locker.assets.swap_name.text.trim().to_string();
    let name = if typed.is_empty() { fill(tr("{} sounds"), &[&display]) } else { typed };
    let swaps = st.locker.assets.swaps.clone();
    st.locker.assets.swap_error = None;
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else {
        st.locker.assets.swap_error = Some(tr("Set the game folder in Settings first.").to_string());
        return;
    };
    st.locker.assets.swap_job.start(st.wake.clone(), move || assets::api::build_swaps(&game_root, &name, &swaps));
}

pub fn poll(st: &mut State) -> bool {
    if let Some(result) = st.locker.assets.share_job.try_take() {
        match result {
            Ok((sig, list)) => {
                let clean = list.is_empty();
                let current = sig == signature(&st.locker.assets.swaps);
                st.locker.assets.shared = Some((sig, list));
                if clean && current {
                    build_now(st);
                }
            }
            Err(error) => st.locker.assets.swap_error = Some(error),
        }
        return true;
    }
    let Some(result) = st.locker.assets.swap_job.try_take() else { return false };
    match result {
        Ok(paths) => {
            st.locker.assets.swaps.clear();
            st.locker.assets.shared = None;
            st.locker.assets.swap_name = rune::TextEdit::new("");
            crate::screens::import::open_local(st, paths);
        }
        Err(error) => st.locker.assets.swap_error = Some(error),
    }
    true
}

pub fn section(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let a = &v.assets;
    let total = a.sounds.ready().map_or(0, Vec::len);
    // Staged swaps open in a dialog; the header keeps a way back to it.
    let swaps_button = (!a.swaps.is_empty() || a.swap_error.is_some() || a.swap_job.is_running()).then(|| {
        u.action(&fill(tr("Your swaps {}"), &[&a.swaps.len().to_string()]), ActionKind::Ghost)
            .h(24.0)
            .px(8.0)
            .key("locker-swaps-open")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.locker.assets.swaps_open = true)
    });
    let mut c = col().w_full().gap(12.0).child(super::pane_head(
        u,
        tr("Game sounds"),
        if a.sounds.ready().is_some() { fill(tr("{} sounds"), &[&total.to_string()]) } else { String::new() },
        swaps_button,
    ));
    match &a.sounds {
        Load::Idle | Load::Loading => {
            return c.child(text(tr("Reading sounds from the game files...")).font(BODY).color(u.muted).wrap());
        }
        Load::Failed(error) => {
            let retry = u
                .action(tr("Try again"), ActionKind::Secondary)
                .key("locker-sounds-retry")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.locker.assets.sounds = Load::Idle;
                    assets::ensure_loaded(st);
                });
            return c
                .child(text(tr("The game's sounds could not be read")).font(BODY_MED).color(u.ink).wrap())
                .child(text(error.clone()).font(SMALL).color(u.muted).wrap())
                .child(row().child(retry));
        }
        Load::Ready(_) => {}
    }
    if total == 0 {
        return c.child(
            text(fill(tr("The game files have no sounds for {}."), &[&hero.display_name]))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    let field = rune::text_input(&a.sound_query, BODY)
        .key("locker-sounds-search")
        .label(tr("Search sounds"))
        .single_line()
        .color(u.ink)
        .placeholder(tr("Search sounds"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(|st: &mut State| &mut st.locker.assets.sound_query)
        .on_input_key(|st: &mut State, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Escape && !st.locker.assets.sound_query.text.is_empty() {
                st.locker.assets.sound_query = rune::TextEdit::new("");
                true
            } else {
                false
            }
        });
    c = c.child(
        u.search_shell(field)
            .w_full()
            .help(tr("Replace takes an MP3. A 16-bit WAV works only for sounds the game stores uncompressed.")),
    );
    if let Some(error) = &s.browse_view.audio.error {
        c = c.child(crate::notify::feedback::error_line(u, "game-sounds-error", error));
    }
    let groups = grouped(v);
    if groups.is_empty() {
        return c.child(text(tr("No sounds match the search.")).font(BODY).color(u.muted).wrap());
    }
    for (group, rows) in groups {
        c = c.child(group_block(s, u, v, &group, &rows));
    }
    c
}

fn group_block(s: &State, u: &Ui, v: &View, group: &SoundGroup, rows: &[&SoundRow]) -> El {
    let open = is_open(v, group);
    let title = group.title();
    let toggle_title = title.clone();
    let mut headline = row()
        .key(format!("locker-sounds-group-{title}"))
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .focusable(true)
        .role("button")
        .expanded(open)
        .label(fill(tr("{}, {} sounds"), &[&title, &rows.len().to_string()]))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            let folded = &mut st.locker.assets.folded;
            if !folded.remove(&toggle_title) {
                folded.insert(toggle_title.clone());
            }
        })
        .child(
            rune::icon(if open { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 14.0)
                .color(u.faint)
                .a11y_hidden(),
        );
    if let Some(chip) = group.chip() {
        headline = headline.child(
            row()
                .h(20.0)
                .px(6.0)
                .center()
                .radius(crate::ui::r(4.0))
                .border(1.0, u.hair_strong)
                .child(text(chip).font(SMALL).color(u.muted))
                .a11y_hidden(),
        );
    }
    headline = headline
        .child(text(title.clone()).font(BODY_MED).color(u.ink).grow(1.0).min_w(0.0).truncate().a11y_hidden())
        .child(text(rows.len().to_string()).font(SMALL).color(u.faint).a11y_hidden());
    let mut block = col().w_full().child(headline);
    if !open {
        return block;
    }
    for row in rows.iter().take(GROUP_CAP) {
        block = block.child(sound_row(s, u, v, row));
    }
    if rows.len() > GROUP_CAP {
        block = block.child(
            text(fill(tr("Showing the first {}. Search to narrow the list."), &[&GROUP_CAP.to_string()]))
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    block
}

fn play_button(s: &State, u: &Ui, key: String, id: u64, label: &str) -> El {
    let audio = &s.browse_view.audio;
    let playing = audio.playing == Some(id);
    let loading = audio.loading == Some(id);
    u.icon_button(
        if playing { &icons::SQUARE_STOP } else if loading { &icons::LOADER } else { &icons::PLAY },
        &fill(if playing { tr("Stop {}") } else { tr("Play {}") }, &[label]),
    )
    .key(key)
    .cursor(rune::Cursor::Pointer)
}

fn sound_row(s: &State, u: &Ui, v: &View, row_data: &SoundRow) -> El {
    let staged = v.assets.swaps.iter().find(|w| w.event == row_data.event);
    let mut detail = row_data.event.clone();
    if row_data.clips.len() > 1 {
        detail.push_str(&format!(" · {}", fill(tr("{} clips"), &[&row_data.clips.len().to_string()])));
    }
    if let Some(seconds) = row_data.seconds {
        detail.push_str(&format!(" · {seconds:.1}s"));
    }
    let first = row_data.clips.first().cloned();
    let play = match first {
        Some(clip) => {
            let id = play_id(&clip);
            play_button(s, u, format!("locker-sound-play-{}", row_data.event), id, &row_data.label)
                .on_press(move |st: &mut State| assets::play_clip(st, &clip))
        }
        None => rune::el().size(32.0, 32.0).a11y_hidden(),
    };
    let mut r = row()
        .key(format!("locker-sound-{}", row_data.event))
        .w_full()
        .h(48.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(play)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(row_data.label.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(text(detail).font(SMALL_NUM).color(u.muted).truncate().selectable(true)),
        );
    // Fixed slots: tag 72, save 28, volume 28, Replace 72, so a staged row never shifts.
    r = r.child(row().w(72.0).justify_end().child(match staged {
        Some(swap) => row()
            .h(24.0)
            .px(8.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .fill(u.over.alpha(0.06))
            .child(text(if swap.file.is_some() { tr("Replaced") } else { tr("Volume") }).font(crate::ui::LABEL).color(u.prose)),
        None => rune::el(),
    }));
    if let Some(clip) = row_data.clips.first().cloned() {
        let label = row_data.label.clone();
        r = r.child(
            u.icon_button(&icons::DOWNLOAD, &fill(tr("Save {} as a file"), &[&row_data.label]))
                .size(28.0, 28.0)
                .key(format!("locker-sound-save-{}", row_data.event))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| assets::save_clip(st, tx, clip.clone(), label.clone())),
        );
    }
    {
        // The volume button keeps its box when the sound is staged.
        let volume_row = row_data.clone();
        let b = u
            .icon_button(&icons::VOLUME_2, &fill(tr("Change the volume of {}"), &[&row_data.label]))
            .size(28.0, 28.0)
            .key(format!("locker-sound-volume-{}", row_data.event))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| stage_volume(st, &volume_row));
        r = r.child(if staged.is_some() { b.disabled(true).opacity(0.0) } else { b });
    }
    r.child(
        u.action_m(if staged.is_some() { tr("Change") } else { tr("Replace") }, ActionKind::Ghost)
            .w(72.0)
            .key(format!("locker-sound-replace-{}", row_data.event))
            .label(fill(tr("Replace {} with your MP3"), &[&row_data.label]))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(pick(row_data.clone())),
    )
}

fn loop_label(looped: Option<bool>) -> &'static str {
    match looped {
        None => tr("Loop: as the game"),
        Some(true) => tr("Loop: on"),
        Some(false) => tr("Loop: off"),
    }
}

/// The swaps as a dialog over the page, so staging a sound never pushes the list.
pub fn swaps_dialog(s: &State, u: &Ui) -> Option<El> {
    let v = &s.locker;
    if !v.assets.swaps_open {
        return None;
    }
    let super::Page::Hero(name) = &v.page else { return None };
    let hero = v.heroes.iter().find(|h| &h.display_name == name)?;
    let close = |st: &mut State| st.locker.assets.swaps_open = false;
    Some(
        u.dialog_shell()
            .key("locker-swaps-dialog")
            .role("dialog")
            .label(tr("Your swaps"))
            .w(480.0)
            .max_w(480.0)
            .padding(24.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &rune::KeyEvent, _| {
                if ev.key == rune::Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(swap_panel(s, u, v, hero))
            .child(
                row().w_full().pt(16.0).child(rune::spacer()).child(
                    u.action(tr("Close"), ActionKind::Ghost)
                        .key("locker-swaps-close")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
            ),
    )
}

fn swap_panel(s: &State, u: &Ui, v: &View, hero: &HeroCard) -> El {
    let a = &v.assets;
    let busy = a.swap_job.is_running();
    let mut panel = col()
        .key("locker-swaps")
        .w_full()
        .gap(10.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(tr("Your swaps")).font(BODY_MED).color(u.ink))
                .child(text(a.swaps.len().to_string()).font(SMALL).color(u.faint)),
        );
    let shared = shared_for(v);
    for swap in &a.swaps {
        let file_name = match &swap.file {
            Some(file) => file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            None => tr("The game's sound, louder or quieter").to_string(),
        };
        let event = swap.event.clone();
        let (down, up) = (swap.event.clone(), swap.event.clone());
        let loop_event = swap.event.clone();
        let next = match swap.looped {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
        let first = row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(match swap.file.clone() {
                Some(file) => play_button(s, u, format!("locker-swap-play-{}", swap.event), play_id(&file.display().to_string()), &file_name)
                    .on_press(move |st: &mut State| assets::play_file(st, &file)),
                None => rune::el().size(32.0, 32.0).center().child(rune::icon(&icons::VOLUME_2, 16.0).color(u.faint)).a11y_hidden(),
            })
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(swap.label.clone()).font(BODY_MED).color(u.ink).truncate())
                    .child(text(file_name.clone()).font(SMALL).color(u.faint).truncate()),
            )
            .child(
                u.icon_button(&icons::X, &fill(tr("Remove the swap of {}"), &[&swap.label]))
                    .key(format!("locker-swap-remove-{}", swap.event))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.locker.assets.swaps.retain(|w| w.event != event)),
            );
        let second = row()
            .w_full()
            .pl(40.0)
            .gap(8.0)
            .center_y()
            .child(
                row()
                    .gap(0.0)
                    .center_y()
                    .role("group")
                    .label(fill(tr("Volume of {}"), &[&swap.label]))
                    .child(
                        u.icon_button(&icons::MINUS, tr("Quieter"))
                            .key(format!("locker-swap-quieter-{}", swap.event))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| nudge(st, &down, -1.0)),
                    )
                    .child(text(gain_label(swap.gain_db)).font(SMALL).color(if swap.gain_db == 0.0 { u.faint } else { u.ink }).w(56.0).align(rune::Align::Center))
                    .child(
                        u.icon_button(&icons::PLUS, tr("Louder"))
                            .key(format!("locker-swap-louder-{}", swap.event))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| nudge(st, &up, 1.0)),
                    ),
            )
            .child(
                u.action(loop_label(swap.looped), ActionKind::Ghost)
                    .key(format!("locker-swap-loop-{}", swap.event))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if let Some(w) = st.locker.assets.swaps.iter_mut().find(|w| w.event == loop_event) {
                            w.looped = next;
                        }
                    }),
            );
        let mut item = col().key(format!("locker-swap-{}", swap.event)).w_full().gap(2.0).child(first).child(second);
        if let Some(heroes) = shared.and_then(|list| list.iter().find(|(e, _)| e == &swap.event)).map(|(_, h)| h) {
            item = item.child(
                row()
                    .w_full()
                    .pl(40.0)
                    .gap(6.0)
                    .child(rune::icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                    .child(text(also_plays(heroes)).font(SMALL).color(u.amber).wrap().grow(1.0).min_w(0.0)),
            );
        }
        panel = panel.child(item);
    }
    let clips: usize = a.swaps.iter().map(|w| w.clips.len()).sum();
    if !a.swaps.is_empty() {
        panel = panel.child(
            text(fill(
                tr("Every clip of a swapped sound plays your audio ({} clips in all)."),
                &[&clips.to_string()],
            ))
            .font(SMALL)
            .color(u.muted)
            .wrap(),
        );
        let placeholder = fill(tr("{} sounds"), &[&hero.display_name]);
        let name = rune::text_input(&a.swap_name, BODY)
            .key("locker-swap-name")
            .label(tr("Mod name"))
            .single_line()
            .color(u.ink)
            .placeholder(placeholder, u.faint)
            .caret(u.accent, u.accent.alpha(0.20))
            .on_edit(|st: &mut State| &mut st.locker.assets.swap_name)
            .on_submit(|st: &mut State, _| build(st));
        panel = panel.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    row()
                        .h(32.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair)
                        .child(name.grow(1.0).min_w(0.0)),
                )
                .child({
                    let checking = a.share_job.is_running();
                    let warned = shared.is_some_and(|list| !list.is_empty());
                    let label = if busy {
                        tr("Building...")
                    } else if checking {
                        tr("Checking...")
                    } else if warned {
                        tr("Build anyway")
                    } else {
                        tr("Build mod")
                    };
                    u.icon_action(if busy || checking { &icons::LOADER } else { &icons::PACKAGE }, label, ActionKind::Primary)
                        .key("locker-swap-build")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(build)
                }),
        );
        if shared.is_some_and(|list| !list.is_empty()) {
            panel = panel.child(
                text(tr("A sound marked above is shared: other heroes play the same clips, so the swap changes it for them too. Build anyway, or remove that swap."))
                    .font(SMALL)
                    .color(u.amber)
                    .wrap(),
            );
        }
        panel = panel.child(
            text(if a.swaps.len() > 1 {
                tr("Each swap becomes its own mod, named after the sound. Sigil opens the import review; they land switched off.")
            } else {
                tr("Sigil packs your swap into a new mod and opens the import review. It lands switched off.")
            })
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        );
    }
    if let Some(error) = &a.swap_error {
        panel = panel.child(crate::notify::feedback::error_line(u, "game-sounds-swap-error", error));
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_is_known_by_its_first_bytes() {
        assert!(is_mp3(b"ID3\x04"));
        assert!(is_mp3(&[0xFF, 0xFB, 0x90, 0x00]));
        assert!(!is_mp3(b"RIFF"));
        assert!(!is_mp3(b"OggS"));
        assert!(!is_mp3(&[]));
    }

    fn sound(event: &str, group: SoundGroup) -> SoundRow {
        SoundRow {
            event: event.to_string(),
            label: event.replace('_', " "),
            group,
            soundevents: Some("soundevents/hero/haze.vsndevts_c".to_string()),
            clips: vec![format!("sounds/{event}.vsnd_c")],
            seconds: None,
        }
    }

    #[test]
    fn groups_follow_slot_order_and_the_search_narrows_them() {
        let mut v = View::default();
        v.assets.sounds = Load::Ready(vec![
            sound("haze_voice_hello", SoundGroup::Voice),
            sound("haze_dagger_cast", SoundGroup::Ability(1, "Sleep Dagger".into())),
            sound("haze_bullet_dance", SoundGroup::Ability(4, "Bullet Dance".into())),
            sound("haze_pistol_fire", SoundGroup::Weapon),
        ]);
        let groups = grouped(&v);
        let titles: Vec<String> = groups.iter().map(|(g, _)| g.title()).collect();
        assert_eq!(titles[0], "Sleep Dagger");
        assert_eq!(titles[1], "Bullet Dance");
        assert_eq!(groups[1].0.chip().as_deref(), Some("ULT"));
        assert!(!is_open(&v, &SoundGroup::Voice));
        assert!(is_open(&v, &SoundGroup::Weapon));
        v.assets.sound_query = rune::TextEdit::new("pistol");
        let groups = grouped(&v);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, SoundGroup::Weapon);
    }

    #[test]
    fn mp3_and_wav_pass_and_other_audio_is_refused() {
        let dir = std::env::temp_dir().join(format!("sigil-swap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ogg = dir.join("a.ogg");
        std::fs::write(&ogg, b"OggS\x00\x02\x00\x00\x00\x00\x00\x00").unwrap();
        assert!(check_mp3(&ogg).is_err());
        let mp3 = dir.join("a.mp3");
        std::fs::write(&mp3, b"ID3\x04\x00").unwrap();
        assert!(check_mp3(&mp3).is_ok());
        let wav_ok = dir.join("pcm.wav");
        std::fs::write(&wav_ok, b"RIFF\x24\x00\x00\x00WAVEfmt ").unwrap();
        assert!(check_mp3(&wav_ok).is_ok(), "a WAV goes to the core, which checks PCM");
        let fake = dir.join("b.mp3");
        std::fs::write(&fake, b"RIFF0000").unwrap();
        assert!(check_mp3(&fake).is_err(), "an mp3 name over a WAV is refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_clips_are_named_and_shared_folders_warn() {
        use crate::screens::locker::assets::api::{clip_key, shared_folder};
        assert_eq!(clip_key("Sounds/Player/Melee/Shared/swing_01.vsnd_c"), "sounds/player/melee/shared/swing_01.vsnd");
        assert!(shared_folder("sounds/player/melee/shared/swing_01.vsnd"));
        assert!(!shared_folder("sounds/abilities/haze/dagger_01.vsnd"));
        assert_eq!(also_plays(&["Wraith".into()]), "Also plays for Wraith.");
        let many: Vec<String> = ["A", "B", "C", "D", "E"].iter().map(|s| s.to_string()).collect();
        assert_eq!(also_plays(&many), "Also plays for A, B, C and 2 more.");
        assert_eq!(also_plays(&[]), "Other heroes play these clips too.");
    }

    #[test]
    fn a_changed_swap_list_needs_a_new_check() {
        let row = sound("haze_pistol_fire", SoundGroup::Weapon);
        let swap = |file: &str| Swap {
            event: row.event.clone(),
            label: row.label.clone(),
            soundevents: row.soundevents.clone(),
            clips: row.clips.clone(),
            file: Some(PathBuf::from(file)),
            looped: None,
            gain_db: 0.0,
        };
        assert_eq!(signature(&[swap("a.mp3")]), signature(&[swap("a.mp3")]));
        assert_ne!(signature(&[swap("a.mp3")]), signature(&[swap("b.mp3")]));
    }
}
