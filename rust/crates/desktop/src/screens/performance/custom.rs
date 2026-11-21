//! Custom presets from public GitHub repositories (Grimoire #572) and "Apply as written"
//! (Grimoire #590).
//!
//! - Add from GitHub: the player types a repository and presses Look up (`repos::inspect`,
//!   `api.github.com`), picks a gameinfo file and a video file, names the preset and presses
//!   Add (`repos::import`, `raw.githubusercontent.com`). Nothing is read before Look up. The
//!   preset joins the list after the built-ins; its page has Delete.
//! - Hide built-in presets: a list choice that keeps only the custom ones.
//! - Apply as written: a curated config that ships a whole gameinfo.gi can have its author's
//!   ConVars values, in their order, merged into the player's file. Only plain allowlisted
//!   values are taken and author comments are not (`as_written`); custom presets never get
//!   this mode. A confirm dialog starts it; Turn off puts the stock file back. GAME WRITE, player triggered.

use deadlock_core::performance::repos::{self, ImportChoice, Inspection};

use super::*;

/// Add from GitHub, the custom preset store and the as-written choice.
#[derive(Default)]
pub struct Custom {
    pub add: Option<Add>,
    pub inspect_job: Job<Inspection>,
    pub import_job: Job<String>,
    pub delete_job: Job<String>,
    /// The store, read with the catalog.
    pub store: Option<repos::Store>,
    /// "Apply as written?" for this config id.
    pub as_written: Option<String>,
}

/// The Add from GitHub dialog.
pub struct Add {
    pub repo: TextEdit,
    pub name: TextEdit,
    pub found: Option<Inspection>,
    /// The gameinfo file picked, `None` for video only.
    pub config: Option<String>,
    /// The video file picked, `None` for none.
    pub video: Option<String>,
    pub error: Option<String>,
}

fn open_add(st: &mut State) {
    st.performance.custom.add = Some(Add {
        repo: TextEdit::new(""),
        name: TextEdit::new(""),
        found: None,
        config: None,
        video: None,
        error: None,
    });
}

fn close_add(st: &mut State) {
    if !st.performance.custom.import_job.is_running() {
        st.performance.custom.add = None;
    }
}

fn read_store(st: &mut State) {
    if simulated(st) {
        return;
    }
    st.performance.custom.store = repos::read_store(&data_dir()).ok();
}

/// Is this config one the player added?
pub fn is_custom(id: &str) -> bool {
    id.starts_with(repos::ID_PREFIX)
}

/// The list hides the built-ins.
pub fn hides_builtins(s: &State) -> bool {
    s.performance.custom.store.as_ref().is_some_and(|store| store.hide_builtins && !store.presets.is_empty())
}

/// Network, from the player's Look up only.
fn start_inspect(st: &mut State) {
    let Some(add) = st.performance.custom.add.as_mut() else { return };
    if st.performance.custom.inspect_job.is_running() {
        return;
    }
    let repo = add.repo.text.trim().to_string();
    if let Err(error) = repos::parse_repository(&repo) {
        add.error = Some(error.to_string());
        return;
    }
    add.error = None;
    add.found = None;
    let wake = st.wake.clone();
    st.performance.custom.inspect_job.start(wake, move || {
        repos::inspect(&performance::fetch::Http::default(), &repo, None).map_err(|e| e.to_string())
    });
}

/// Network, from the player's Add. Writes the app data directory only.
fn start_import(st: &mut State) {
    let Some(add) = st.performance.custom.add.as_mut() else { return };
    let Some(found) = add.found.clone() else { return };
    if st.performance.custom.import_job.is_running() {
        return;
    }
    if add.config.is_none() && add.video.is_none() {
        add.error = Some(tr("Pick a gameinfo file or a video file.").to_string());
        return;
    }
    let choice = ImportChoice {
        name: add.name.text.trim().to_string(),
        config_path: add.config.clone(),
        video_path: Some(add.video.clone()),
    };
    add.error = None;
    let wake = st.wake.clone();
    st.performance.custom.import_job.start(wake, move || {
        repos::import(&performance::fetch::Http::default(), &data_dir(), &found, &choice, &repos::now())
            .map_err(|e| e.to_string())
    });
}

fn start_delete(st: &mut State, id: String) {
    if st.performance.custom.delete_job.is_running() {
        return;
    }
    if simulated(st) {
        st.performance.open = None;
        return;
    }
    let wake = st.wake.clone();
    st.performance.custom.delete_job.start(wake, move || {
        repos::delete(&data_dir(), &id).map_err(|e| e.to_string())?;
        Ok(id)
    });
}

fn set_hide(st: &mut State, hide: bool) {
    if let Some(store) = st.performance.custom.store.as_mut() {
        store.hide_builtins = hide;
    }
    if !simulated(st)
        && let Err(error) = repos::set_hide_builtins(&data_dir(), hide)
    {
        st.performance.message = Some((error.to_string(), true));
    }
}

/// GAME WRITE from the "Apply as written" confirm.
fn start_as_written(st: &mut State, id: String) {
    st.performance.custom.as_written = None;
    let Some(config) = config(st, &id).cloned() else { return };
    let Some(root) = st.game_root.clone() else { return };
    if simulated(st) {
        st.performance.message = Some((applied_line(config.name(), &[]), false));
        return;
    }
    let wake = st.wake.clone();
    st.performance.write_job.start(wake, move || {
        performance::as_written::apply_to_install(&root, &data_dir(), &config).map_err(|e| e.to_string())?;
        Ok(fill(tr("{} is on, as its author wrote it. It takes effect the next time you start the game."), &[config.name()]))
    });
}

/// After the catalog loads, and to take finished reads.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if st.performance.custom.store.is_none() && st.performance.catalog.is_some() {
        read_store(st);
        if st.performance.custom.store.is_none() {
            st.performance.custom.store = Some(repos::Store::default());
        }
    }
    if let Some(result) = st.performance.custom.inspect_job.try_take() {
        if let Some(add) = st.performance.custom.add.as_mut() {
            match result {
                Ok(found) => {
                    add.config = found.config_paths.first().cloned();
                    add.video = repos::suggested_video_path(add.config.as_deref(), &found.video_paths)
                        .ok()
                        .flatten()
                        .or_else(|| add.config.is_none().then(|| found.video_paths.first().cloned()).flatten());
                    if add.name.text.trim().is_empty() {
                        let short = found.repository.rsplit('/').next().unwrap_or_default().to_string();
                        add.name = TextEdit::new(&short);
                    }
                    if found.config_paths.is_empty() && found.video_paths.is_empty() {
                        add.error = Some(tr("The repository has no gameinfo or video file Sigil can use.").to_string());
                    }
                    add.found = Some(found);
                }
                Err(error) => add.error = Some(error),
            }
        }
        changed = true;
    }
    if let Some(result) = st.performance.custom.import_job.try_take() {
        match result {
            Ok(id) => {
                st.performance.custom.add = None;
                st.performance.custom.store = None;
                st.performance.catalog = None;
                st.performance.open = Some(id);
                start_read(st);
            }
            Err(error) => {
                if let Some(add) = st.performance.custom.add.as_mut() {
                    add.error = Some(error);
                }
            }
        }
        changed = true;
    }
    if let Some(result) = st.performance.custom.delete_job.try_take() {
        match result {
            Ok(_) => {
                st.performance.open = None;
                st.performance.custom.store = None;
                st.performance.catalog = None;
                start_read(st);
            }
            Err(error) => st.performance.message = Some((error, true)),
        }
        changed = true;
    }
    changed
}

// --- views ---------------------------------------------------------------------------------

/// The list page's Add from GitHub button.
pub fn add_button(u: &Ui) -> El {
    u.icon_action(&icons::PLUS, tr("Add from GitHub"), ActionKind::Secondary)
        .key("perf-custom-add")
        .cursor(rune::Cursor::Pointer)
        .on_press(open_add)
}

/// "Hide built-in presets" over the list, once the player has a custom one.
pub fn hide_choice(s: &State, u: &Ui) -> Option<El> {
    let store = s.performance.custom.store.as_ref().filter(|store| !store.presets.is_empty())?;
    let on = store.hide_builtins;
    Some(
        row()
            .key("perf-custom-hide")
            .gap(8.0)
            .center_y()
            .role("checkbox")
            .checked(on)
            .focusable(true)
            .label(tr("Hide built-in presets").to_string())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| set_hide(st, !on))
            .child(
                icon(if on { &icons::SQUARE_CHECK } else { &icons::SQUARE }, 14.0)
                    .color(if on { u.accent } else { u.muted })
                    .a11y_hidden(),
            )
            .child(text(tr("Hide built-in presets")).font(SMALL).color(u.muted).a11y_hidden()),
    )
}

thread_local! {
    /// The last config asked about, by id and commit: the view must not read the disk each frame.
    static WHOLE: std::cell::RefCell<Option<(String, String, bool)>> = const { std::cell::RefCell::new(None) };
}

fn ships_whole_file(config: &Config) -> bool {
    WHOLE.with(|cell| {
        let mut cell = cell.borrow_mut();
        match cell.as_ref() {
            Some((id, commit, whole)) if id == config.id() && *commit == config.commit => *whole,
            _ => {
                let whole = performance::as_written::author_file(&data_dir(), config).is_some();
                *cell = Some((config.id().to_string(), config.commit.clone(), whole));
                whole
            }
        }
    })
}

/// On a config's page: Apply as written when it ships a whole file, and Delete for a custom
/// preset. `None` when neither applies.
pub fn detail_section(s: &State, u: &Ui, config: &Config) -> Option<El> {
    let whole = !simulated(s) && ships_whole_file(config);
    let custom = is_custom(config.id());
    if !whole && !custom {
        return None;
    }
    let mut section = col().key("perf-custom").w_full().gap(10.0);
    if whole {
        let id = config.id().to_string();
        section = section.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(tr("Apply as written uses the author's own ConVars values, in their order. Only plain values are taken and the rest of your file stays; your switches and edits do not apply then."))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    u.action(tr("Apply as written"), ActionKind::Secondary)
                        .key("perf-as-written")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if !st.performance.write_job.is_running() {
                                st.performance.custom.as_written = Some(id.clone());
                            }
                        }),
                ),
        );
    }
    if custom {
        let preset = s.performance.custom.store.as_ref().and_then(|store| store.get(config.id()));
        let line = match preset {
            Some(preset) => fill(
                tr("Added from {} at commit {}. A custom preset sets plain values only: no engine edits or commands."),
                &[&preset.repo, &preset.commit.chars().take(7).collect::<String>()],
            ),
            None => tr("A custom preset sets plain values only: no engine edits or commands.").to_string(),
        };
        let id = config.id().to_string();
        let is_on = applied_id(s).as_deref() == Some(config.id());
        section = section.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(text(line).font(SMALL).color(u.muted).wrap().grow(1.0).min_w(0.0))
                .child(
                    u.icon_action(&icons::TRASH_2, tr("Delete preset"), ActionKind::Secondary)
                        .key("perf-custom-delete")
                        .opacity(if is_on { 0.4 } else { 1.0 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if is_on {
                                st.performance.message =
                                    Some((tr("Turn the preset off before you delete it.").to_string(), true));
                            } else {
                                start_delete(st, id.clone());
                            }
                        }),
                ),
        );
    }
    Some(section)
}

fn field(u: &Ui, edit: &TextEdit, key: &str, placeholder: &str, pick: fn(&mut State) -> &mut TextEdit) -> El {
    row()
        .w_full()
        .h(32.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            rune::text_input(edit, BODY)
                .key(key.to_string())
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .color(u.ink)
                .placeholder(placeholder.to_string(), u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(rune::edit_access(pick)),
        )
}

fn repo_field(st: &mut State) -> &mut TextEdit {
    match st.performance.custom.add.as_mut() {
        Some(add) => &mut add.repo,
        None => &mut st.performance.scratch,
    }
}

fn name_field(st: &mut State) -> &mut TextEdit {
    match st.performance.custom.add.as_mut() {
        Some(add) => &mut add.name,
        None => &mut st.performance.scratch,
    }
}

/// One choice among file paths, `None` labelled `none`.
fn picks(u: &Ui, key: &str, paths: &[String], current: &Option<String>, none: &str, set: fn(&mut Add, Option<String>)) -> El {
    let mut list = col().w_full().gap(2.0);
    let mut options: Vec<Option<String>> = paths.iter().cloned().map(Some).collect();
    options.push(None);
    for (index, option) in options.into_iter().enumerate() {
        let on = &option == current;
        let label = option.clone().unwrap_or_else(|| tr(none).to_string());
        let mono = option.is_some();
        list = list.child(
            row()
                .key(format!("{key}-{index}"))
                .w_full()
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .hover(u.hover)
                .role("radio")
                .checked(on)
                .focusable(true)
                .label(label.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(add) = st.performance.custom.add.as_mut() {
                        set(add, option.clone());
                    }
                })
                .child(text(label).font(if mono { MONO } else { BODY }).color(if on { u.ink } else { u.muted }).truncate()),
        );
    }
    list
}

/// The Add from GitHub dialog and the Apply as written confirm.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if let Some(id) = &s.performance.custom.as_written {
        let config = config(s, id)?;
        let id = id.clone();
        let body = col()
            .w_full()
            .gap(8.0)
            .child(text(fill(tr("Apply {} as written?"), &[config.name()])).font(H1).color(u.ink))
            .child(
                text(tr("Merges the author's ConVars values into your gameinfo.gi. Only plain values are taken; your search paths and everything else stay. A stock copy is kept beside it, and Turn off puts it back."))
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
            )
            .child(text(tr("Takes effect the next time you start the game.")).font(BODY).color(u.muted).wrap());
        return Some(shell(
            u,
            "perf-as-written-dialog",
            body,
            u.action(tr("Apply"), ActionKind::Primary)
                .key("perf-confirm-as-written")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| start_as_written(st, id.clone())),
            |st: &mut State| st.performance.custom.as_written = None,
        ));
    }
    let add = s.performance.custom.add.as_ref()?;
    let looking = s.performance.custom.inspect_job.is_running();
    let importing = s.performance.custom.import_job.is_running();
    let mut body = col()
        .w_full()
        .gap(10.0)
        .child(text(tr("Add a preset from GitHub")).font(H1).color(u.ink))
        .child(
            text(tr("Type a public repository. Sigil reads it at its newest commit and keeps a copy, so the preset never changes under you. Only plain values are used."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(el().grow(1.0).min_w(0.0).child(field(u, &add.repo, "perf-custom-repo", "owner/repository", repo_field)))
                .child(
                    u.action(if looking { tr("Looking up") } else { tr("Look up") }, ActionKind::Secondary)
                        .key("perf-custom-lookup")
                        .opacity(if looking { 0.5 } else { 1.0 })
                        .cursor(rune::Cursor::Pointer)
                        .on_press(start_inspect),
                ),
        );
    if let Some(found) = &add.found {
        body = body
            .child(text(fill(tr("{} at {}"), &[&found.repository, &found.date])).font(SMALL).color(u.muted))
            .child(text(tr("gameinfo file")).font(BODY_MED).color(u.ink))
            .child(picks(u, "perf-custom-config", &found.config_paths, &add.config, "None, video only", |add, pick| add.config = pick))
            .child(text(tr("Video file")).font(BODY_MED).color(u.ink))
            .child(picks(u, "perf-custom-video", &found.video_paths, &add.video, "None", |add, pick| add.video = pick))
            .child(text(tr("Name")).font(BODY_MED).color(u.ink))
            .child(field(u, &add.name, "perf-custom-name", "", name_field));
        if !found.unsupported.is_empty() {
            body = body.child(
                text(fill(tr("Not used: {}"), &[&found.unsupported.join(", ")]))
                    .font(SMALL)
                    .color(u.muted)
                    .wrap(),
            );
        }
    }
    if let Some(error) = &add.error {
        body = body.child(text(tr(error).to_string()).font(SMALL).color(u.rose).wrap());
    }
    let confirm = u
        .action(if importing { tr("Adding") } else { tr("Add") }, ActionKind::Primary)
        .key("perf-custom-import")
        .opacity(if add.found.is_some() && !importing { 1.0 } else { 0.4 })
        .cursor(rune::Cursor::Pointer)
        .on_press(start_import);
    Some(shell(u, "perf-custom-dialog", body, confirm, close_add))
}

fn shell(u: &Ui, key: &str, body: El, confirm: El, close: fn(&mut State)) -> El {
    let buttons = row()
        .w_full()
        .gap(8.0)
        .child(spacer())
        .child(
            u.action(tr("Cancel"), ActionKind::Ghost)
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        )
        .child(confirm);
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
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
        .child(body)
        .child(buttons)
}

/// The Add dialog after a Look up, for snapshots.
pub fn scene_add(s: &mut State) {
    open_add(s);
    let found = Inspection {
        repository: "someone/fps-config".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        date: "2026-11-20".into(),
        config_paths: vec!["Competitive/gameinfo.gi".into(), "Potato/gameinfo.gi".into()],
        video_paths: vec!["Competitive/video.txt".into()],
        addons: Vec::new(),
        unsupported: vec!["autoexec.cfg".into()],
        files: Default::default(),
    };
    if let Some(add) = s.performance.custom.add.as_mut() {
        add.repo = TextEdit::new("someone/fps-config");
        add.name = TextEdit::new("fps-config");
        add.config = Some("Competitive/gameinfo.gi".into());
        add.video = Some("Competitive/video.txt".into());
        add.found = Some(found);
    }
}
