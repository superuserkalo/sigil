//! Console variables on the Autoexec page: every convar the game has, searchable, with Valve's
//! description, default and limits, the value the player has now, and an edit that lands in the
//! command list above the Save button. Game presets set several at once after a preview.
//!
//! The data is `deadlock_core::autoexec::convars`: Valve's convar list from
//! GameTracking-Deadlock (the cached copy, from a Performance refresh or the Download the list
//! button here), the autoexec and `machine_convars.vcfg` for current values. Editing writes no
//! file: an edit changes the list, and the page's Save writes the autoexec with its backup and
//! check.
//! Saved presets live in `game-presets.json` under the app data directory.
//!
//! Behaviour after Deadlock Mod Manager's convar editor and game presets branches (GPL-3.0,
//! behaviour only) and Grimoire's command library (MIT).

use std::collections::BTreeMap;

use deadlock_core::autoexec::convars::{self as core, Catalog, Change, Convar, Current, GamePreset, Problem};
use rune::{El, Key, KeyEvent, TextEdit, col, row, spacer, text};

use crate::app::State;
use crate::jobs::Job;
use crate::notify::feedback;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

/// Rows the list shows before Show more.
const PAGE: usize = 30;

/// What demo mode and headless scenes list instead of the player's cache.
const SAMPLE_DUMP: &str = "citadel_hud_visible true (clientdll archive)\n\tShow the HUD.\nengine_low_latency_sleep_after_client_tick false (clientdll archive)\n\tSleep after the client tick to lower latency.\nfps_max 400 (min: 0, max: 1000 clientdll archive)\n\tFrame rate limit. 0 means no limit.\nfps_max_ui 120 (min: 0, max: 1000 clientdll archive)\n\tFrame rate limit while a menu is open.\nr_fullscreen_gamma 2.2 (min: 1, max: 3 clientdll archive)\n\tDisplay gamma.\nrate 1000000 (clientdll archive)\n\tNetwork rate in bytes per second.\nsv_cheats false (notify replicated)\n\tAllow cheat convars.\n";

#[path = "autoexec_convars_de.rs"]
mod de;

fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some((_, de)) = de::DE.iter().find(|(en, _)| *en == text)
    {
        return de;
    }
    i18n::tr(text)
}

pub struct Convars {
    pub catalog: Option<Catalog>,
    /// `machine_convars.vcfg`, read when the section opens.
    pub game: BTreeMap<String, String>,
    pub query: TextEdit,
    pub group: Option<&'static str>,
    pub cheats: bool,
    pub shown: usize,
    /// The convar whose value field is open.
    pub editing: Option<String>,
    pub value: TextEdit,
    pub error: Option<String>,
    /// Presets the player saved.
    pub saved: Vec<GamePreset>,
    /// The preset whose preview is open.
    pub preview: Option<GamePreset>,
    pub preset_name: TextEdit,
    pub note: Option<String>,
    /// The player's Download the list.
    pub download: Job<Catalog>,
    pub download_error: Option<String>,
}

impl Default for Convars {
    fn default() -> Self {
        Convars {
            catalog: None,
            game: BTreeMap::new(),
            query: TextEdit::new(""),
            group: None,
            cheats: false,
            shown: PAGE,
            editing: None,
            value: TextEdit::new(""),
            error: None,
            saved: Vec::new(),
            preview: None,
            preset_name: TextEdit::new(""),
            note: None,
            download: Job::default(),
            download_error: None,
        }
    }
}

// --- actions -----------------------------------------------------------------------------------

/// Read the catalogue, the game's saved values and the saved presets. Local files only.
pub fn open(st: &mut State) {
    let data = crate::data::data_dir();
    let catalog = if simulated(st) { sample_catalog() } else { core::load_catalog(&data) };
    let c = &mut st.autoexec.convars;
    c.catalog = Some(catalog);
    c.saved = core::load_presets(&data);
    c.game = st
        .game_root
        .as_ref()
        .and_then(|root| std::fs::read_to_string(core::machine_convars_path(root)).ok())
        .map(|text| core::parse_vcfg(&text))
        .unwrap_or_default();
}

fn simulated(st: &State) -> bool {
    st.demo || rune::is_headless()
}

fn sample_catalog() -> Catalog {
    Catalog { convars: core::parse_dump(SAMPLE_DUMP) }
}

/// Fetch Valve's list on a worker, from the player's press. Offline mode and the proxy apply as
/// they do to every Performance fetch.
fn start_download(st: &mut State) {
    if simulated(st) || st.autoexec.convars.download.is_running() {
        return;
    }
    st.autoexec.convars.download_error = None;
    let wake = st.wake.clone();
    st.autoexec.convars.download.start(wake, || {
        let data = crate::data::data_dir();
        deadlock_core::performance::refresh_convar_list(&data).map_err(|error| error.to_string())?;
        let catalog = core::load_catalog(&data);
        if catalog.convars.is_empty() {
            return Err(tr("The download did not hold a usable list.").to_string());
        }
        Ok(catalog)
    });
}

/// Take a finished download.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.autoexec.convars.download.try_take() else { return false };
    let c = &mut st.autoexec.convars;
    match result {
        Ok(catalog) => {
            c.catalog = Some(catalog);
            c.download_error = None;
            c.shown = PAGE;
        }
        Err(error) => c.download_error = Some(error),
    }
    true
}

/// A Performance refresh may have just cached the list: pick it up if this section is waiting.
pub fn reload_if_empty(st: &mut State) {
    if simulated(st) || !st.autoexec.convars.catalog.as_ref().is_some_and(|catalog| catalog.convars.is_empty()) {
        return;
    }
    st.autoexec.convars.catalog = Some(core::load_catalog(&crate::data::data_dir()));
}

fn autoexec_values(st: &State) -> BTreeMap<String, String> {
    core::autoexec_values(&st.autoexec.text())
}

fn start_edit(st: &mut State, name: String) {
    let values = autoexec_values(st);
    let c = &st.autoexec.convars;
    let shown = match c.catalog.as_ref().and_then(|catalog| catalog.get(&name)) {
        Some(convar) => match core::current(&name, &values, &c.game) {
            Current::Autoexec(value) | Current::Game(value) => value,
            Current::Default => convar.default.clone(),
        },
        None => String::new(),
    };
    let c = &mut st.autoexec.convars;
    c.value = TextEdit::new(&shown);
    c.error = None;
    c.editing = Some(name);
}

/// Put `name value` into the list, over the line that sets it now.
fn set_value(st: &mut State) {
    let Some(name) = st.autoexec.convars.editing.clone() else { return };
    let value = st.autoexec.convars.value.text.trim().to_string();
    // Only a name the game's list holds is written, and only as one checked command.
    let Some(convar) = st.autoexec.convars.catalog.as_ref().and_then(|catalog| catalog.get(&name)) else {
        st.autoexec.convars.error = Some(tr("This setting is not in the game's list.").to_string());
        return;
    };
    if let Some(problem) = convar.check(&value) {
        st.autoexec.convars.error = Some(problem_text(&problem));
        return;
    }
    let Some(command) = core::safe_command(&name, &value) else {
        st.autoexec.convars.error = Some(problem_text(&Problem::Forbidden));
        return;
    };
    st.autoexec.add_preset(&command);
    st.autoexec.convars.editing = None;
    st.autoexec.convars.note = Some(fill(tr("{} is in the list. Save to write it."), &[&name]));
}

fn problem_text(problem: &Problem) -> String {
    match problem {
        Problem::Empty => tr("Type a value.").to_string(),
        Problem::Forbidden => tr("A value cannot hold quotes, semicolons or line breaks.").to_string(),
        Problem::NotBool => tr("Use true or false.").to_string(),
        Problem::NotWhole => tr("Use a whole number.").to_string(),
        Problem::NotNumber => tr("Use a number.").to_string(),
        Problem::BelowMin(min) => fill(tr("The game's lowest value is {}."), &[min]),
        Problem::AboveMax(max) => fill(tr("The game's highest value is {}."), &[max]),
    }
}

/// Take every line that sets `name` out of the list.
fn remove_value(st: &mut State, name: &str) {
    let ids: Vec<u64> = st
        .autoexec
        .blocks
        .iter()
        .filter(|block| deadlock_core::autoexec::command_name(&block.raw()).as_deref() == Some(name))
        .map(|block| block.id)
        .collect();
    for id in ids {
        st.autoexec.remove(id);
    }
    st.autoexec.convars.editing = None;
    st.autoexec.convars.note = Some(fill(tr("{} is out of the list. Save to write it."), &[name]));
}

fn apply_preset(st: &mut State) {
    let Some(preset) = st.autoexec.convars.preview.take() else { return };
    for command in core::preset_commands(&preset) {
        st.autoexec.add_preset(&command);
    }
    st.autoexec.convars.note = Some(fill(tr("{} is in the list. Save to write it."), &[&preset.name]));
}

fn save_preset(st: &mut State) {
    let name = st.autoexec.convars.preset_name.text.trim().to_string();
    if name.is_empty() {
        st.autoexec.convars.note = Some(tr("Name the preset first.").to_string());
        return;
    }
    let values = autoexec_values(st);
    let Some(catalog) = st.autoexec.convars.catalog.as_ref() else { return };
    let Some(preset) = core::preset_from_autoexec(&name, &values, catalog) else {
        st.autoexec.convars.note = Some(tr("The list sets no known convar yet.").to_string());
        return;
    };
    let c = &mut st.autoexec.convars;
    c.saved.retain(|seen| !seen.name.eq_ignore_ascii_case(&name));
    c.saved.push(preset);
    c.preset_name = TextEdit::new("");
    st.autoexec.convars.note = match store(st) {
        Ok(()) => Some(fill(tr("Saved the preset {}."), &[&name])),
        Err(error) => Some(error),
    };
}

fn delete_preset(st: &mut State, name: &str) {
    st.autoexec.convars.saved.retain(|seen| seen.name != name);
    st.autoexec.convars.note = match store(st) {
        Ok(()) => Some(fill(tr("Deleted the preset {}."), &[name])),
        Err(error) => Some(error),
    };
}

fn store(st: &State) -> Result<(), String> {
    if st.demo || rune::is_headless() {
        return Ok(());
    }
    core::store_presets(&crate::data::data_dir(), &st.autoexec.convars.saved).map_err(|error| error.to_string())
}

// --- view --------------------------------------------------------------------------------------

fn pill(u: &Ui, key: String, label: &str, on: bool) -> El {
    row()
        .key(key)
        .h(26.0)
        .px(9.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        // A wrapping filter chip, not a segmented selector: the pick is its own fill.
        .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(label.to_string()).font(SMALL).color(if on { u.ink } else { u.muted }).a11y_hidden())
}

fn field(u: &Ui, edit: &TextEdit, key: &str, label: &str, width: f32) -> El {
    rune::text_input(edit, BODY)
        .key(key.to_string())
        .label(label.to_string())
        .single_line()
        .w(width)
        .color(u.ink)
        .placeholder(label.to_string(), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
}

fn field_frame(u: &Ui) -> El {
    row().h(32.0).px(10.0).center_y().radius(crate::ui::r(8.0)).fill(u.well).border(1.0, u.hair)
}

fn presets(s: &State, u: &Ui) -> El {
    let c = &s.autoexec.convars;
    let mut chips = row().key("convars-presets").w_full().gap(8.0).flex_wrap();
    let builtin = core::builtin_presets();
    for preset in builtin.iter().chain(c.saved.iter()) {
        let saved = c.saved.iter().any(|seen| seen.name == preset.name);
        let open = preset.clone();
        let mut chip = row()
            .key(format!("convars-preset-{}", preset.name))
            .h(32.0)
            .pl(12.0)
            .pr(if saved { 4.0 } else { 12.0 })
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(8.0))
            .border(1.0, u.hair)
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(fill(tr("Preview the preset {}"), &[&preset.name]))
            .help(if preset.description.is_empty() {
                fill(tr("{} convars"), &[&preset.values.len().to_string()])
            } else {
                tr(&preset.description).to_string()
            })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.autoexec.convars.preview = Some(open.clone()))
            .child(text(tr(&preset.name).to_string()).font(BODY_MED).color(u.ink).a11y_hidden())
            .child(text(preset.values.len().to_string()).font(SMALL).color(u.faint).a11y_hidden());
        if saved {
            let name = preset.name.clone();
            chip = chip.child(
                u.icon_button(&crate::icons::X, tr("Delete preset"))
                    .key(format!("convars-preset-delete-{}", preset.name))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| delete_preset(st, &name)),
            );
        }
        chips = chips.child(chip);
    }
    let save = row()
        .gap(8.0)
        .center_y()
        .child(
            field_frame(u).child(
                field(u, &c.preset_name, "convars-preset-name", tr("New preset name"), 180.0)
                    .on_edit(|st: &mut State| &mut st.autoexec.convars.preset_name)
                    .on_key(|st: &mut State, ev: &KeyEvent, _| {
                        if ev.key == Key::Enter {
                            save_preset(st);
                            true
                        } else {
                            false
                        }
                    }),
            ),
        )
        .child(
            u.action(tr("Save the list as a preset"), ActionKind::Secondary)
                .key("convars-preset-save")
                .cursor(rune::Cursor::Pointer)
                .on_press(save_preset),
        );
    col()
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(text(tr("Game presets")).font(BODY_MED).color(u.ink))
                .child(text(tr("Several convars at once. A preview shows each line first.")).font(SMALL).color(u.faint))
                .child(spacer())
                .child(save),
        )
        .child(chips)
}

fn current_text(current: &Current, default: &str) -> (String, &'static str) {
    match current {
        Current::Autoexec(value) => (value.clone(), tr("in your autoexec")),
        Current::Game(value) => (value.clone(), tr("saved by the game")),
        Current::Default => (default.to_string(), tr("default")),
    }
}

fn convar_row(s: &State, u: &Ui, convar: &Convar, values: &BTreeMap<String, String>) -> El {
    let c = &s.autoexec.convars;
    let current = core::current(&convar.name, values, &c.game);
    let (value, source) = current_text(&current, &convar.default);
    let in_list = matches!(current, Current::Autoexec(_));
    let editing = c.editing.as_deref() == Some(convar.name.as_str());
    let mut notes: Vec<String> = vec![tr(convar.group()).to_string()];
    if !convar.default.is_empty() {
        notes.push(fill(tr("default {}"), &[&convar.default]));
    }
    match (convar.min, convar.max) {
        (Some(min), Some(max)) => notes.push(fill(tr("{} to {}"), &[&format!("{min}"), &format!("{max}")])),
        (Some(min), None) => notes.push(fill(tr("at least {}"), &[&format!("{min}")])),
        (None, Some(max)) => notes.push(fill(tr("at most {}"), &[&format!("{max}")])),
        (None, None) => {}
    }
    if convar.cheat() {
        notes.push(tr("needs sv_cheats").to_string());
    }
    if convar.server_only() {
        notes.push(tr("your own lobbies only").to_string());
    }
    let name = convar.name.clone();
    let mut body = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(text(convar.name.clone()).font(MONO).color(u.ink).truncate())
                .child(spacer())
                .child(text(value).font(MONO).color(if in_list { u.accent } else { u.prose }).max_w(220.0).truncate())
                .child(text(source).font(SMALL).color(u.faint)),
        )
        .children((!convar.description.is_empty()).then(|| {
            text(convar.description.clone()).font(SMALL).color(u.muted).wrap().max_lines(2)
        }))
        .child(text(notes.join(" · ")).font(SMALL).color(u.faint));
    if editing {
        let mut edit = row()
            .w_full()
            .gap(8.0)
            .pt(6.0)
            .center_y()
            .child(
                field_frame(u).child(
                    field(u, &c.value, "convars-value", tr("Value"), 200.0)
                        .on_edit(|st: &mut State| &mut st.autoexec.convars.value)
                        .on_key(|st: &mut State, ev: &KeyEvent, _| match ev.key {
                            Key::Enter => {
                                set_value(st);
                                true
                            }
                            Key::Escape => {
                                st.autoexec.convars.editing = None;
                                true
                            }
                            _ => false,
                        }),
                ),
            )
            .child(
                u.action(tr("Set in the list"), ActionKind::Secondary)
                    .key("convars-set")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(set_value),
            );
        let default = convar.default.clone();
        if !default.is_empty() {
            edit = edit.child(
                u.action(tr("Use the default"), ActionKind::Ghost)
                    .key("convars-default")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.autoexec.convars.value = TextEdit::new(&default)),
            );
        }
        if in_list {
            let name = convar.name.clone();
            edit = edit.child(
                u.action(tr("Take out of the list"), ActionKind::Ghost)
                    .key("convars-remove")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| remove_value(st, &name)),
            );
        }
        body = body.child(edit);
        if let Some(error) = &c.error {
            body = body.child(text(error.clone()).font(SMALL).color(u.rose));
        }
    }
    let mut frame = row()
        .key(format!("convar-{}", convar.name))
        .w_full()
        .py(8.0)
        .gap(12.0)
        .items_start()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(body);
    if !editing {
        frame = frame
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(fill(tr("Edit {}"), &[&convar.name]))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| start_edit(st, name.clone()));
    }
    frame
}

/// No cached list yet: where it comes from and the button that fetches it.
fn empty_state(s: &State, u: &Ui) -> El {
    let c = &s.autoexec.convars;
    let running = c.download.is_running();
    let label = if running { tr("Downloading the list...") } else { tr("Download the list") };
    let mut card = col()
        .key("convars-empty")
        .w_full()
        .gap(10.0)
        .padding(16.0)
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(text(tr("The convar list is not on this computer yet")).font(BODY_MED).color(u.ink))
        .child(
            text(tr("It comes from the public game tracking repository on GitHub. Sigil downloads it only when you press the button or refresh Performance."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row().child(
                u.action(label, ActionKind::Primary)
                    .key("convars-download")
                    .disabled(running)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(start_download),
            ),
        );
    if let Some(error) = &c.download_error {
        card = card.child(feedback::error_line(u, "convars-download-error", error));
    }
    card
}

/// The section on the Autoexec page.
pub fn section(s: &State, u: &Ui) -> El {
    let c = &s.autoexec.convars;
    let mut section = col().key("autoexec-convars").w_full().gap(12.0);
    let Some(catalog) = &c.catalog else {
        return section.child(
            row()
                .w_full()
                .h(32.0)
                .gap(16.0)
                .center_y()
                .child(u.section(tr("Console variables")))
                .child(text(tr("Every convar the game has, with Valve's descriptions")).font(SMALL).color(u.faint))
                .child(spacer())
                .child(
                    u.action(tr("Browse convars"), ActionKind::Secondary)
                        .key("convars-open")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(open),
                ),
        );
    };
    if catalog.convars.is_empty() {
        section = section
            .child(
                row()
                    .w_full()
                    .h(32.0)
                    .gap(16.0)
                    .center_y()
                    .child(u.section(tr("Console variables")))
                    .child(text(tr("Every convar the game has, with Valve's descriptions")).font(SMALL).color(u.faint)),
            )
            .child(empty_state(s, u))
            .child(presets(s, u));
        if let Some(note) = &c.note {
            section = section.child(text(note.clone()).font(SMALL).color(u.muted));
        }
        return section;
    }
    let source = tr("Valve's list from the game tracking repository");
    section = section.child(
        row()
            .w_full()
            .h(32.0)
            .gap(16.0)
            .center_y()
            .child(u.section(tr("Console variables")))
            .child(text(fill(tr("{} convars. {}"), &[&catalog.convars.len().to_string(), source])).font(SMALL).color(u.faint).truncate())
            .child(spacer())
            .child(
                row()
                    .key("convars-cheats")
                    .gap(8.0)
                    .center_y()
                    .focusable(true)
                    .role("switch")
                    .checked(c.cheats)
                    .label(tr("Show cheat convars"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| {
                        st.autoexec.convars.cheats = !st.autoexec.convars.cheats;
                        st.autoexec.convars.shown = PAGE;
                    })
                    .child(u.toggle_mark(c.cheats))
                    .child(text(tr("Show cheat convars")).font(SMALL).color(u.muted).a11y_hidden()),
            ),
    );
    section = section.child(presets(s, u));
    if let Some(note) = &c.note {
        section = section.child(text(note.clone()).font(SMALL).color(u.muted));
    }
    let mut groups = row().gap(2.0).flex_wrap();
    groups = groups.child(pill(u, "convars-group-all".to_string(), tr("All"), c.group.is_none()).on_press(|st: &mut State| {
        st.autoexec.convars.group = None;
        st.autoexec.convars.shown = PAGE;
    }));
    for group in core::groups() {
        groups = groups.child(
            pill(u, format!("convars-group-{group}"), tr(group), c.group == Some(group)).on_press(move |st: &mut State| {
                st.autoexec.convars.group = Some(group);
                st.autoexec.convars.shown = PAGE;
            }),
        );
    }
    section = section.child(
        row()
            .w_full()
            .gap(12.0)
            .items_start()
            .child(
                field_frame(u).w(280.0).child(
                    field(u, &c.query, "convars-search", tr("Search convars and descriptions"), 256.0)
                        .on_edit(|st: &mut State| {
                            st.autoexec.convars.shown = PAGE;
                            &mut st.autoexec.convars.query
                        }),
                ),
            )
            .child(groups.grow(1.0).min_w(0.0)),
    );
    let values = core::autoexec_values(&s.autoexec.text());
    let hits = catalog.search(&c.query.text, c.group, c.cheats);
    let mut list = col().key("convars-list").w_full();
    if hits.is_empty() {
        list = list.child(text(tr("No convar matches.")).font(BODY).color(u.muted));
    }
    for convar in hits.iter().take(c.shown) {
        list = list.child(convar_row(s, u, convar, &values));
    }
    section = section.child(list);
    if hits.len() > c.shown {
        section = section.child(
            u.action(&fill(tr("Show more ({} left)"), &[&(hits.len() - c.shown).to_string()]), ActionKind::Ghost)
                .key("convars-more")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.autoexec.convars.shown += PAGE),
        );
    }
    section
}

/// The preset preview, mounted with the page's other dialogs.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let preset = s.autoexec.convars.preview.as_ref()?;
    let close = |st: &mut State| st.autoexec.convars.preview = None;
    let changes = core::preview(preset, &core::autoexec_values(&s.autoexec.text()));
    let mut lines = col().w_full().gap(4.0);
    for change in &changes {
        let (mark, line, color) = match change {
            Change::Add { name, value } => ("+", format!("{name} {value}"), u.teal),
            Change::Change { name, from, to } => ("~", format!("{name} {from} → {to}"), u.amber),
            Change::Same { name, value } => ("=", format!("{name} {value}"), u.faint),
        };
        lines = lines.child(
            row()
                .w_full()
                .gap(8.0)
                .child(text(mark).font(MONO).color(color).w(12.0))
                .child(text(line).font(MONO).color(if color == u.faint { u.faint } else { u.prose }).grow(1.0).min_w(0.0).wrap_anywhere()),
        );
    }
    let adds = changes.iter().filter(|change| matches!(change, Change::Add { .. })).count();
    let edits = changes.iter().filter(|change| matches!(change, Change::Change { .. })).count();
    Some(
        u.dialog_shell()
            .key("convars-preset-dialog")
            .role("dialog")
            .label(preset.name.clone())
            .w(520.0)
            .padding(24.0)
            .gap(16.0)
            .modal(true, close)
            .on_key(move |st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    close(st);
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(text(tr(&preset.name).to_string()).font(H1).color(u.ink))
                    .children((!preset.description.is_empty()).then(|| text(tr(&preset.description).to_string()).font(BODY).color(u.muted).wrap()))
                    .child(
                        text(fill(tr("Adds {} lines and changes {}. Nothing is written until you save."), &[&adds.to_string(), &edits.to_string()]))
                            .font(SMALL)
                            .color(u.faint),
                    ),
            )
            .child(lines)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .pt(8.0)
                    .child(spacer())
                    .child(u.action(tr("Cancel"), ActionKind::Ghost).cursor(rune::Cursor::Pointer).on_press(close))
                    .child(
                        u.action(tr("Add to the list"), ActionKind::Primary)
                            .key("convars-preset-apply")
                            .disabled(adds + edits == 0)
                            .cursor(rune::Cursor::Pointer)
                            .on_press(apply_preset),
                    ),
            ),
    )
}

/// `autoexec-convars`: the catalogue open on a search with one convar in edit.
pub fn stage_scene(s: &mut State) {
    // The catalogue lives in the right column's All convars tab.
    s.autoexec.all_convars = true;
    let c = &mut s.autoexec.convars;
    c.catalog = Some(sample_catalog());
    c.game = core::parse_vcfg("\"convars\"\n{\n\t\"fps_max\"\t\t\"240\"\n}\n");
    c.query = TextEdit::new("fps");
    c.saved = vec![GamePreset {
        name: "Ranked nights".to_string(),
        description: String::new(),
        values: vec![("fps_max".to_string(), "237".to_string()), ("rate".to_string(), "1000000".to_string())],
    }];
}

/// `autoexec-convars-preset`: a preset's preview over the page.
pub fn stage_preview(s: &mut State) {
    stage_scene(s);
    s.autoexec.convars.preview = core::builtin_presets().into_iter().next();
}

/// `next-convars-empty`: no cached list yet, with the button that fetches it.
pub fn stage_empty(s: &mut State) {
    s.autoexec.all_convars = true;
    s.autoexec.convars.catalog = Some(Catalog::default());
}

/// `next-convars-failed`: the download failed.
pub fn stage_failed(s: &mut State) {
    stage_empty(s);
    s.autoexec.convars.download_error = Some(tr("The download did not hold a usable list.").to_string());
}

#[cfg(test)]
mod tests {
    use super::de::DE;

    #[test]
    #[cfg_attr(not(feature = "languages"), ignore = "translations are parked: --features languages")]
    fn every_string_has_german() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in DE {
            assert!(seen.insert(*en), "duplicate {en:?}");
            assert_eq!(en.matches("{}").count(), de.matches("{}").count(), "holes in {en:?}");
            assert!(!en.contains('\u{2014}') && !de.contains('\u{2014}'));
        }
        let source = include_str!("autoexec_convars.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (at, _) in body.match_indices("tr(\"") {
            let rest = &body[at + 4..];
            let literal = &rest[..rest.find('"').unwrap()];
            assert!(DE.iter().any(|(en, _)| *en == literal), "untranslated {literal:?}");
        }
        for group in deadlock_core::autoexec::convars::groups() {
            assert!(DE.iter().any(|(en, _)| *en == group), "untranslated group {group:?}");
        }
        for preset in deadlock_core::autoexec::convars::builtin_presets() {
            assert!(DE.iter().any(|(en, _)| *en == preset.name), "untranslated preset {:?}", preset.name);
            assert!(DE.iter().any(|(en, _)| *en == preset.description), "untranslated {:?}", preset.description);
        }
    }
}
