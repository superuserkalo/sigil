//! Bulk paste import (Grimoire's "Links or profile" box; DMM has only whole collections): paste
//! many GameBanana mod links or ids, or one collection link. A preview lists every mod with what
//! Sigil knows of it; mods already installed, duplicates, other sites and anything over 100 are
//! skipped with the reason. Skip NSFW drops mods marked mature. Each row can pick one of the mod's
//! files (Grimoire's variant pick; the newest file when none is picked). Queue sends the picked
//! ones to the download queue one by one; Cancel remaining takes back those still waiting; once
//! some are installed, Save as profile keeps them as a profile. A collection link opens that
//! collection; a profile code (`mp1:`) or profile JSON goes to Profiles > Import, as Grimoire
//! routes a profile code to its import dialog.

use std::collections::{BTreeSet, HashMap};

use deadlock_core::gamebanana::CollectionSummary;
use deadlock_core::mod_lookup::ModFile;

use super::*;
use crate::screens::settings::i18n::{fill, tr};

/// Grimoire queues at most 100 pasted links.
pub const MAX_ITEMS: usize = 100;

#[derive(Default)]
pub struct Paste {
    pub open: bool,
    pub text: TextEdit,
    /// The parsed preview, once Preview was pressed.
    pub rows: Option<Vec<Row>>,
    /// Lines that were not mod links: duplicates, other kinds, over the limit.
    pub skipped: Skipped,
    pub picked: BTreeSet<u64>,
    pub skip_nsfw: bool,
    /// Mods this paste queued, to cancel the rest.
    pub queued: Vec<u64>,
    /// Each mod's files, read when its row opens; a failed read keeps its reason.
    pub files: HashMap<u64, Result<Vec<ModFile>, String>>,
    pub files_job: Job<(u64, Vec<ModFile>)>,
    files_asked: Option<u64>,
    /// The row whose files are open.
    pub open_row: Option<u64>,
    /// The file picked per mod (none: the newest).
    pub variant: HashMap<u64, u64>,
    /// The profile Save as profile wrote.
    pub saved: Option<Result<String, String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    pub duplicates: usize,
    pub invalid: usize,
    pub over: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub name: String,
    pub nsfw: bool,
    pub installed: bool,
}

/// What the box holds: mod ids in order, or one collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Mods(Vec<u64>, Skipped),
    Collection(u64),
}

fn id_after(token: &str, marker: &str) -> Option<u64> {
    let at = token.find(marker)?;
    let rest = &token[at + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Split on spaces, commas and lines; read `gamebanana.com/mods/<id>` links and bare ids. A lone
/// collection link is a collection.
pub fn parse(text: &str) -> Parsed {
    let tokens: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
    if tokens.len() == 1
        && let Some(id) = id_after(tokens[0], "gamebanana.com/collections/")
    {
        return Parsed::Collection(id);
    }
    let mut ids = Vec::new();
    let mut skipped = Skipped::default();
    for token in tokens {
        let id = if token.chars().all(|c| c.is_ascii_digit()) {
            token.parse().ok()
        } else {
            id_after(token, "gamebanana.com/mods/")
        };
        match id {
            Some(id) if ids.contains(&id) => skipped.duplicates += 1,
            Some(_) if ids.len() >= MAX_ITEMS => skipped.over += 1,
            Some(id) => ids.push(id),
            None => skipped.invalid += 1,
        }
    }
    Parsed::Mods(ids, skipped)
}

pub fn open(st: &mut State) {
    st.browse_view.paste.open = true;
}

fn close(st: &mut State) {
    let queued = std::mem::take(&mut st.browse_view.paste.queued);
    st.browse_view.paste = Paste { queued, ..Paste::default() };
}

/// A profile code or profile JSON: Profiles > Import reads it (codes, JSON and the mods behind).
pub fn is_profile(text: &str) -> bool {
    let t = text.trim();
    t.starts_with(deadlock_core::profiles::SHARE_CODE_PREFIX) || t.starts_with('{')
}

fn to_profiles(st: &mut State, text: String) {
    close(st);
    st.profiles.import = crate::screens::profiles::ImportView::default();
    st.profiles.import.input = TextEdit::new(text);
    st.profiles.dialog = crate::screens::profiles::Dialog::Import;
    st.go(Screen::Profiles);
}

/// Open a row's files, reading them once on a worker.
fn toggle_files(st: &mut State, id: u64) {
    let p = &mut st.browse_view.paste;
    p.open_row = if p.open_row == Some(id) { None } else { Some(id) };
    if p.open_row != Some(id) || p.files.contains_key(&id) || p.files_job.is_running() {
        return;
    }
    if st.demo || rune::is_headless() || st.prefs.offline {
        st.browse_view.paste.files.insert(id, Err(tr("Sigil is offline.").to_string()));
        return;
    }
    st.browse_view.paste.files_asked = Some(id);
    let wake = st.wake.clone();
    st.browse_view.paste.files_job.start(wake, move || {
        deadlock_core::mod_lookup::fetch_mod_files(id).map(|files| (id, files)).map_err(|e| e.to_string())
    });
}

/// Take a finished file read. Called from Browse's poll.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.browse_view.paste.files_job.try_take() else {
        return false;
    };
    let asked = st.browse_view.paste.files_asked.take();
    match result {
        Ok((id, files)) => {
            st.browse_view.paste.files.insert(id, Ok(files));
        }
        Err(error) => {
            if let Some(id) = asked {
                st.browse_view.paste.files.insert(id, Err(error));
            }
        }
    }
    true
}

/// Save the pasted mods that are installed now as a profile, all on (Grimoire's Save as
/// profile). App data only; applying it later is the Profiles page's own approved step.
fn save_profile(st: &mut State) {
    let ids = st.browse_view.paste.queued.clone();
    let mut slots = Vec::new();
    for id in ids {
        for slot in super::replace::candidates(st, id) {
            let Some(addon) = st.addons.addons.iter().find(|a| a.slot == slot) else { continue };
            let name = crate::screens::installed::meta_for(st, addon).map(|m| m.name.clone());
            slots.push(deadlock_core::profiles::ProfileSlot {
                slot,
                file: addon.file.clone(),
                enabled: true,
                name,
                gamebanana_id: Some(id),
                gamebanana_file_id: None,
                fingerprint: None,
            });
        }
    }
    if slots.is_empty() {
        st.browse_view.paste.saved = Some(Err(tr("None of these mods is installed yet.").to_string()));
        return;
    }
    let workspace = crate::data::workspace_dir();
    let stamp = {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (y, m, d) = crate::screens::settings::log::civil_date((secs / 86_400) as i64);
        format!("{y}-{m:02}-{d:02} {:02}:{:02}", secs % 86_400 / 3_600, secs % 3_600 / 60)
    };
    let mut name = fill(tr("Pasted mods {}"), &[&stamp]);
    let mut n = 2;
    while deadlock_core::profiles::profile_path(&workspace, &deadlock_core::profiles::slug_for(&name)).exists() {
        name = fill(tr("Pasted mods {} ({})"), &[&stamp, &n.to_string()]);
        n += 1;
    }
    let now = format!("{stamp}:00");
    let profile = deadlock_core::profiles::Profile {
        schema: deadlock_core::profiles::PROFILE_SCHEMA,
        slug: deadlock_core::profiles::slug_for(&name),
        name: name.clone(),
        created_at: now.clone(),
        updated_at: now,
        slots,
        crosshair: None,
        autoexec: None,
        author: None,
        extensions: Default::default(),
        isolated: false,
    };
    let result = if st.demo || rune::is_headless() {
        Ok(())
    } else {
        deadlock_core::profiles::write_profile(&workspace, &profile).map_err(|e| e.to_string())
    };
    st.browse_view.paste.saved = Some(result.map(|_| name));
}

/// What Sigil knows of a mod: a card on screen, then the saved catalog.
fn known(s: &State, id: u64) -> (String, bool) {
    if let Some(card) = card_by_id(s, id) {
        return (card.name.clone(), card.nsfw);
    }
    if let Some(entry) = s.browse_view.mirror.as_ref().and_then(|m| m.entries.iter().find(|e| e.id == id)) {
        return (entry.name.clone(), entry.nsfw);
    }
    (fill(tr("Mod {}"), &[&id.to_string()]), false)
}

fn preview(st: &mut State) {
    let text = st.browse_view.paste.text.text.clone();
    if is_profile(&text) {
        return to_profiles(st, text);
    }
    match parse(&text) {
        Parsed::Collection(id) => {
            close(st);
            st.browse_view.tab = Tab::Collections;
            collections::open_collection(
                st,
                CollectionSummary { id, name: fill(tr("Collection {}"), &[&id.to_string()]), ..CollectionSummary::default() },
            );
        }
        Parsed::Mods(ids, skipped) => {
            let owned: BTreeSet<u64> = st.browse_view.installs.owned.keys().copied().collect();
            let rows: Vec<Row> = ids
                .into_iter()
                .map(|id| {
                    let (name, nsfw) = known(st, id);
                    Row { id, name, nsfw, installed: owned.contains(&id) }
                })
                .collect();
            let p = &mut st.browse_view.paste;
            p.picked = rows.iter().filter(|r| !r.installed && !(p.skip_nsfw && r.nsfw)).map(|r| r.id).collect();
            p.rows = Some(rows);
            p.skipped = skipped;
        }
    }
}

fn queue(st: &mut State) {
    let picks: Vec<(u64, String)> = st
        .browse_view
        .paste
        .rows
        .iter()
        .flatten()
        .filter(|r| st.browse_view.paste.picked.contains(&r.id))
        .map(|r| (r.id, r.name.clone()))
        .collect();
    for (id, name) in &picks {
        let file = st.browse_view.paste.variant.get(id).and_then(|file| {
            st.browse_view.paste.files.get(id).and_then(|f| f.as_ref().ok()).and_then(|files| {
                files.iter().find(|f| f.file_id == *file).map(|f| (f.file_id, f.download_url.clone()))
            })
        });
        match file {
            Some((file_id, url)) => quick::press_file(st, *id, name, url, file_id),
            None => quick::press(st, *id, name),
        }
    }
    st.browse_view.paste.queued = picks.into_iter().map(|(id, _)| id).collect();
}

fn cancel_remaining(st: &mut State) {
    let queued = st.browse_view.paste.queued.clone();
    for id in queued {
        if quick::button(st, id) == quick::Button::Queued {
            quick::cancel(st, id);
        }
    }
}

pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let p = &s.browse_view.paste;
    if !p.open {
        return None;
    }
    let mut body = col()
        .w_full()
        .gap(12.0)
        .child(text(tr("Paste mods")).font(H1).color(u.ink))
        .child(
            text(tr("Paste GameBanana mod links or ids, one or many, or a collection link."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .min_h(96.0)
                .px(10.0)
                .py(8.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(&p.text, BODY)
                        .key("paste-input")
                        .label(tr("Links or ids"))
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder("https://gamebanana.com/mods/123456", u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(|st: &mut State| &mut st.browse_view.paste.text)
                        .autofocus(),
                ),
        );
    if let Some(rows) = &p.rows {
        let skipped = &p.skipped;
        let mut notes = Vec::new();
        if skipped.duplicates > 0 {
            notes.push(fill(tr("{} duplicates skipped"), &[&skipped.duplicates.to_string()]));
        }
        if skipped.invalid > 0 {
            notes.push(fill(tr("{} entries were not mod links"), &[&skipped.invalid.to_string()]));
        }
        if skipped.over > 0 {
            notes.push(fill(tr("{} over the 100 mod limit"), &[&skipped.over.to_string()]));
        }
        let all: Vec<u64> = rows.iter().filter(|r| !r.installed && !(p.skip_nsfw && r.nsfw)).map(|r| r.id).collect();
        let every = !all.is_empty() && all.iter().all(|id| p.picked.contains(id));
        body = body.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(text(fill(tr("{} mods, {} picked"), &[&rows.len().to_string(), &p.picked.len().to_string()])).font(BODY_MED).color(u.ink))
                .child(spacer())
                .child(
                    u.action(tr(if every { "Deselect all" } else { "Select all" }), ActionKind::Ghost)
                        .key("paste-select-all")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            let picked = &mut st.browse_view.paste.picked;
                            if every {
                                picked.clear();
                            } else {
                                picked.extend(all.iter().copied());
                            }
                        }),
                )
                .child(
                    row()
                        .key("paste-skip-nsfw")
                        .gap(8.0)
                        .center_y()
                        .role("checkbox")
                        .checked(p.skip_nsfw)
                        .label(tr("Skip NSFW"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            let p = &mut st.browse_view.paste;
                            p.skip_nsfw = !p.skip_nsfw;
                            if p.skip_nsfw {
                                let nsfw: Vec<u64> = p.rows.iter().flatten().filter(|r| r.nsfw).map(|r| r.id).collect();
                                for id in nsfw {
                                    p.picked.remove(&id);
                                }
                            }
                        })
                        .child(u.toggle_mark(p.skip_nsfw))
                        .child(text(tr("Skip NSFW")).font(SMALL).color(u.muted)),
                ),
        );
        if !notes.is_empty() {
            body = body.child(text(notes.join(" · ")).font(SMALL).color(u.faint).wrap());
        }
        let mut list = col().key("paste-rows").w_full().max_h(280.0).scroll(ScrollSpec { stick_bottom: false });
        for row_data in rows {
            let id = row_data.id;
            let reason = if row_data.installed {
                Some(tr("Installed already"))
            } else if p.skip_nsfw && row_data.nsfw {
                Some(tr("Skipped: NSFW"))
            } else {
                None
            };
            let on = p.picked.contains(&id);
            let state = quick::button(s, id);
            let mut line = row()
                .key(format!("paste-row-{id}"))
                .w_full()
                .h(36.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(row_data.name.clone()).font(BODY).color(if reason.is_some() { u.faint } else { u.ink }).truncate().grow(1.0).min_w(0.0));
            line = match reason {
                Some(reason) => line.child(text(reason).font(SMALL).color(u.faint)),
                None if state.busy() || state == quick::Button::Installed => line.child(text(state.label()).font(SMALL).color(u.accent)),
                None => {
                    let variant = p.variant.get(&id).and_then(|file| {
                        p.files.get(&id).and_then(|f| f.as_ref().ok()).and_then(|files| files.iter().find(|f| f.file_id == *file))
                    });
                    line.child(
                        u.action(
                            &match variant {
                                Some(file) => file.file_name.clone(),
                                None => tr("Newest file").to_string(),
                            },
                            ActionKind::Ghost,
                        )
                        .key(format!("paste-files-{id}"))
                        .label(fill(tr("Pick a file of {}"), &[&row_data.name]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| toggle_files(st, id)),
                    )
                    .child(
                        u.toggle(on)
                            .key(format!("paste-pick-{id}"))
                            .label(row_data.name.clone())
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                let picked = &mut st.browse_view.paste.picked;
                                if !picked.remove(&id) {
                                    picked.insert(id);
                                }
                            }),
                    )
                }
            };
            list = list.child(line);
            if p.open_row == Some(id) {
                list = list.child(files_list(s, u, id));
            }
        }
        body = body.child(u.scroll_area(list));
    }
    let waiting = p.queued.iter().filter(|id| quick::button(s, **id) == quick::Button::Queued).count();
    let mut actions = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(u.action(tr("Close"), ActionKind::Ghost).key("paste-close").cursor(rune::Cursor::Pointer).on_press(close))
        .child(spacer());
    let installed = p.queued.iter().filter(|id| quick::button(s, **id) == quick::Button::Installed).count();
    if installed > 0 && waiting == 0 {
        actions = actions.child(
            u.action(&fill(tr("Save {} as a profile"), &[&installed.to_string()]), ActionKind::Secondary)
                .key("paste-save-profile")
                .cursor(rune::Cursor::Pointer)
                .on_press(save_profile),
        );
    }
    if waiting > 0 {
        actions = actions.child(
            u.action(tr("Cancel remaining"), ActionKind::Secondary)
                .key("paste-cancel-remaining")
                .cursor(rune::Cursor::Pointer)
                .on_press(cancel_remaining),
        );
    }
    actions = match &p.rows {
        None => actions.child(
            u.action(tr("Preview"), ActionKind::Primary)
                .key("paste-preview")
                .disabled(p.text.text.trim().is_empty())
                .cursor(rune::Cursor::Pointer)
                .on_press(preview),
        ),
        Some(_) => actions
            .child(u.action(tr("Preview again"), ActionKind::Ghost).key("paste-repreview").cursor(rune::Cursor::Pointer).on_press(preview))
            .child(
                u.action(&fill(tr("Queue {}"), &[&p.picked.len().to_string()]), ActionKind::Primary)
                    .key("paste-queue")
                    .disabled(p.picked.is_empty())
                    .cursor(rune::Cursor::Pointer)
                    .on_press(queue),
            ),
    };
    if let Some(saved) = &p.saved {
        body = body.child(match saved {
            Ok(name) => text(fill(tr("Profile saved: {}. Apply it from Profiles."), &[name])).font(SMALL).color(u.teal).wrap(),
            Err(error) => crate::notify::feedback::error_line(u, "paste-save-error", error),
        });
    }
    Some(
        u.dialog_shell()
            .key("browse-paste")
            .role("dialog")
            .label(tr("Paste mods"))
            .w(560.0)
            .padding(24.0)
            .modal(true, close)
            .child(body.child(actions)),
    )
}

/// One row's files: the newest by default, or one picked by hand.
fn files_list(s: &State, u: &Ui, id: u64) -> El {
    let p = &s.browse_view.paste;
    let mut c = col().key(format!("paste-files-list-{id}")).w_full().pl(16.0).py(4.0).gap(2.0);
    let Some(result) = p.files.get(&id) else {
        return c.child(text(tr("Reading the files")).font(SMALL).color(u.muted));
    };
    let files = match result {
        Ok(files) => files,
        Err(error) => return c.child(crate::notify::feedback::error_line(u, "paste-read-error", error)),
    };
    let picked = p.variant.get(&id).copied();
    let choice = |key: String, label: String, detail: String, on: bool, file: Option<u64>| {
        row()
            .key(key)
            .w_full()
            .h(30.0)
            .px(8.0)
            .gap(10.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .fill(if on { u.selected } else { Color::TRANSPARENT })
            .hover(u.hover)
            .focusable(true)
            .role("radio")
            .checked(on)
            .label(label.clone())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                match file {
                    Some(file) => st.browse_view.paste.variant.insert(id, file),
                    None => st.browse_view.paste.variant.remove(&id),
                };
            })
            .child(icon(if on { &icons::CIRCLE_CHECK } else { &icons::FILE }, 13.0).color(if on { u.accent } else { u.faint }).a11y_hidden())
            .child(text(label).font(SMALL).color(u.ink).truncate().grow(1.0).min_w(0.0))
            .child(text(detail).font(SMALL).color(u.faint))
    };
    c = c.child(choice(format!("paste-file-{id}-newest"), tr("Newest file").to_string(), String::new(), picked.is_none(), None));
    for file in files {
        let size = file.size.map(crate::model::format_size).unwrap_or_default();
        c = c.child(choice(
            format!("paste-file-{id}-{}", file.file_id),
            file.file_name.clone(),
            size,
            picked == Some(file.file_id),
            Some(file.file_id),
        ));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_codes_and_json_go_to_profiles() {
        assert!(is_profile("mp1:H4sIAAAA"));
        assert!(is_profile("  {\"schema\": 1}"));
        assert!(!is_profile("https://gamebanana.com/mods/1"));
    }

    #[test]
    fn links_ids_duplicates_and_junk() {
        let text = "https://gamebanana.com/mods/741478, 741608\nhttps://gamebanana.com/mods/741478 nope https://x.org/1";
        assert_eq!(
            parse(text),
            Parsed::Mods(vec![741478, 741608], Skipped { duplicates: 1, invalid: 2, over: 0 })
        );
        assert_eq!(parse("https://gamebanana.com/collections/1234"), Parsed::Collection(1234));
        let many: String = (1..=105).map(|i| format!("{i} ")).collect();
        match parse(&many) {
            Parsed::Mods(ids, skipped) => {
                assert_eq!(ids.len(), MAX_ITEMS);
                assert_eq!(skipped.over, 5);
            }
            other => panic!("{other:?}"),
        }
    }
}
