//! Hidden mods and creators that say what they were (Grimoire #580, MIT): each row shows the
//! mod's picture, or a creator's three most liked uploads, blurred until the pointer is on the
//! row (these are things the player chose not to see); the date it was hidden and, for a
//! creator, the mod of theirs it was hidden from; and a short note the player writes. No
//! request is added: pictures come from what was shown when the mod was hidden and from the
//! saved catalog, and an entry neither knows keeps its letter.
//!
//! Deadlock Mod Manager hides mods and creators too but keeps no date or note. Sigil shows the
//! same rows in Browse's Hidden items and in Settings > Mods.

use serde_json::{Value, json};

use super::*;
use crate::screens::settings::i18n::{fill, tr};

/// Longest note kept, as Grimoire's.
pub const NOTE_MAX: usize = 140;

/// What Sigil remembers about one hidden entry beyond its id or name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiddenMeta {
    /// Unix seconds; 0 for an entry hidden before Sigil kept the date.
    pub hidden_at: i64,
    pub note: String,
    /// For a creator: the mod of theirs whose details they were hidden from.
    pub from: Option<(u64, String)>,
    /// For a mod: its picture when it was hidden.
    pub thumb: Option<String>,
    /// For a mod: its creator.
    pub author: Option<String>,
}

impl HiddenMeta {
    /// The shape `browse/state.json` keeps under `hiddenMeta`.
    pub fn to_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        if self.hidden_at > 0 {
            map.insert("hiddenAt".into(), json!(self.hidden_at));
        }
        if !self.note.is_empty() {
            map.insert("note".into(), json!(self.note));
        }
        if let Some((id, name)) = &self.from {
            map.insert("fromMod".into(), json!({ "id": id, "name": name }));
        }
        if let Some(thumb) = &self.thumb {
            map.insert("thumb".into(), json!(thumb));
        }
        if let Some(author) = &self.author {
            map.insert("author".into(), json!(author));
        }
        Value::Object(map)
    }

    pub fn from_json(value: &Value) -> HiddenMeta {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        HiddenMeta {
            hidden_at: value.get("hiddenAt").and_then(Value::as_i64).unwrap_or(0).max(0),
            note: clean_note(&text("note").unwrap_or_default()),
            from: value.get("fromMod").and_then(|from| {
                Some((from.get("id")?.as_u64()?, from.get("name")?.as_str()?.to_string()))
            }),
            thumb: text("thumb").filter(|url| url.starts_with("https://")),
            author: text("author").filter(|name| !name.is_empty()),
        }
    }
}

pub fn mod_key(id: u64) -> String {
    format!("mod:{id}")
}

pub fn author_key(name: &str) -> String {
    format!("author:{}", name.trim().to_lowercase())
}

/// The note as it is stored: trimmed and at most [`NOTE_MAX`] characters.
pub fn clean_note(text: &str) -> String {
    text.trim().chars().take(NOTE_MAX).collect::<String>().trim().to_string()
}

/// The line under a hidden entry's name: when it was hidden and where from, else its kind.
pub fn hidden_line(meta: Option<&HiddenMeta>, kind: &str) -> String {
    let Some(meta) = meta.filter(|meta| meta.hidden_at > 0) else {
        return tr(kind).to_string();
    };
    let date = super::store::date_input(meta.hidden_at);
    match &meta.from {
        Some((_, name)) if !name.is_empty() => fill(tr("Hidden {} from {}"), &[&date, name]),
        _ => fill(tr("Hidden {}"), &[&date]),
    }
}

/// One picture on a row: the mod it shows, its address and whether it is mature.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub nsfw: bool,
}

/// A hidden mod's picture: the one kept when it was hidden, else the saved catalog's.
fn mod_pictures(s: &State, id: u64) -> Vec<Picture> {
    let meta = s.browse_view.hidden_meta.get(&mod_key(id));
    let entry = s
        .browse_view
        .mirror
        .as_ref()
        .and_then(|mirror| mirror.entries.iter().find(|entry| entry.id == id));
    let name = s.browse_view.hidden_names.get(&id).cloned().unwrap_or_default();
    let url = meta
        .and_then(|meta| meta.thumb.clone())
        .or_else(|| entry.and_then(|entry| entry.thumbnail_url.clone()));
    url.map(|url| Picture { id, name, url, nsfw: entry.is_some_and(|entry| entry.nsfw) })
        .into_iter()
        .collect()
}

/// A hidden creator's three most liked uploads in the saved catalog, and how many it holds.
pub fn creator_pictures(entries: &[deadlock_core::catalog_mirror::MirrorEntry], name: &str) -> (Vec<Picture>, usize) {
    let mut theirs: Vec<&deadlock_core::catalog_mirror::MirrorEntry> = entries
        .iter()
        .filter(|entry| entry.author.eq_ignore_ascii_case(name.trim()))
        .collect();
    let count = theirs.len();
    theirs.sort_by(|a, b| b.likes.cmp(&a.likes).then(a.id.cmp(&b.id)));
    let pictures = theirs
        .into_iter()
        .filter_map(|entry| {
            entry.thumbnail_url.clone().map(|url| Picture {
                id: entry.id,
                name: entry.name.clone(),
                url,
                nsfw: entry.nsfw,
            })
        })
        .take(3)
        .collect();
    (pictures, count)
}

fn author_pictures(s: &State, name: &str) -> (Vec<Picture>, usize) {
    match s.browse_view.mirror.as_ref() {
        Some(mirror) => creator_pictures(&mirror.entries, name),
        None => (Vec::new(), 0),
    }
}

/// Ask for the rows' pictures while a list that shows them is open. The image crew skips what
/// it has or is already reading.
pub fn poll(st: &mut State) {
    let shown = st.browse_view.panel == Some(Panel::Hidden) || st.screen == Screen::Settings;
    if !shown || st.demo || rune::is_headless() {
        return;
    }
    let mut urls: Vec<String> = Vec::new();
    for id in st.browse_view.hidden_mods.iter() {
        urls.extend(mod_pictures(st, *id).into_iter().map(|p| p.url));
    }
    for name in st.browse_view.hidden_authors.iter() {
        urls.extend(author_pictures(st, name).0.into_iter().map(|p| p.url));
    }
    urls.retain(|url| !st.thumbnails.contains_key(url));
    if !urls.is_empty() {
        crate::data::queue_images(st, urls, false);
    }
}

/// Every hidden entry as a row with its pictures, line, note and Restore. `scope` keeps keys
/// apart between Browse's dialog and Settings.
pub fn rows(s: &State, u: &Ui, scope: &'static str) -> Vec<El> {
    let mut out = Vec::new();
    let mut ids: Vec<u64> = s.browse_view.hidden_mods.iter().copied().collect();
    // Newest first; entries without a date keep their id order after them.
    let at = |key: &str| s.browse_view.hidden_meta.get(key).map(|m| m.hidden_at).unwrap_or(0);
    ids.sort_by(|a, b| at(&mod_key(*b)).cmp(&at(&mod_key(*a))).then(a.cmp(b)));
    for id in ids {
        let key = mod_key(id);
        let name = s
            .browse_view
            .hidden_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| fill(tr("Mod {}"), &[&id.to_string()]));
        let meta = s.browse_view.hidden_meta.get(&key);
        let mut line = hidden_line(meta, "Hidden mod");
        if let Some(author) = meta.and_then(|m| m.author.as_ref()) {
            line = format!("{line} · {}", fill(tr("by {}"), &[author]));
        }
        let page = format!("https://gamebanana.com/mods/{id}");
        out.push(row_el(s, u, scope, &key, &name, line, mod_pictures(s, id), Some(page), Restore::Mod(id)));
    }
    let mut names: Vec<String> = s.browse_view.hidden_authors.iter().cloned().collect();
    names.sort_by(|a, b| {
        at(&author_key(b)).cmp(&at(&author_key(a))).then(a.to_lowercase().cmp(&b.to_lowercase()))
    });
    for name in names {
        let key = author_key(&name);
        let meta = s.browse_view.hidden_meta.get(&key);
        let (pictures, count) = author_pictures(s, &name);
        let mut line = hidden_line(meta, "Creator");
        if count > 0 {
            let uploads = if count == 1 { tr("1 upload").to_string() } else { fill(tr("{} uploads"), &[&count.to_string()]) };
            line = format!("{line} · {uploads}");
        }
        out.push(row_el(s, u, scope, &key, &name, line, pictures, None, Restore::Author(name.clone())));
    }
    out
}

#[derive(Clone)]
enum Restore {
    Mod(u64),
    Author(String),
}

#[allow(clippy::too_many_arguments)]
fn row_el(
    s: &State,
    u: &Ui,
    scope: &'static str,
    key: &str,
    name: &str,
    line: String,
    pictures: Vec<Picture>,
    page: Option<String>,
    restore: Restore,
) -> El {
    let hovered = s.browse_view.hidden_hover.as_deref() == Some(key);
    let blur_mature = s.prefs.mature != crate::prefs::Mature::Show;
    let meta = s.browse_view.hidden_meta.get(key);
    let note = meta.map(|m| m.note.clone()).unwrap_or_default();
    let slug: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();

    let mut strip = row().gap(4.0).w(if pictures.is_empty() { 40.0 } else { 40.0 * pictures.len() as f32 + 4.0 }).h(40.0);
    if pictures.is_empty() {
        let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        strip = strip.child(
            el().w(40.0)
                .h(40.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(text(letter).font(BODY_MED).color(u.muted)),
        );
    }
    for picture in pictures {
        // Blurred until the row is under the pointer; a mature picture keeps its cover.
        let blur = !hovered || (picture.nsfw && blur_mature);
        let section = BrowseSection::Mods;
        let (id, title, url) = (picture.id, picture.name.clone(), picture.url.clone());
        strip = strip.child(
            el().key(format!("{scope}-{slug}-pic-{id}"))
                .w(40.0)
                .h(40.0)
                .clip_rounded(crate::ui::r(6.0))
                .focusable(true)
                .role("button")
                .label(fill(tr("Open {}"), &[&picture.name]))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    to_browse(st);
                    show_details(st, Details::from_id(section, id, &title, Some(url.clone())));
                })
                .child(image_fill(s, format!("{scope}-{slug}-img-{id}"), Some(&picture.url), [crate::ui::r(6.0); 4], blur)),
        );
    }

    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(name.to_string()).font(BODY).color(u.ink).truncate())
        .child(text(line).font(SMALL).color(u.faint).truncate());
    let editing = s.browse_view.hidden_note.as_ref().filter(|(k, _)| k == key);
    if editing.is_none() && !note.is_empty() {
        text_col = text_col.child(text(note.clone()).font(SMALL).color(u.muted).wrap().w_full());
    }

    let note_key = key.to_string();
    let note_now = note.clone();
    let mut actions = row().gap(4.0).center_y().child(
        u.icon_button(&icons::SQUARE_PEN, tr(if note.is_empty() { "Add a note" } else { "Edit note" }))
            .key(format!("{scope}-{slug}-note"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                st.browse_view.hidden_note = Some((note_key.clone(), TextEdit::new(note_now.clone())));
            }),
    );
    if let Some(page) = page {
        actions = actions.child(
            u.icon_button(&icons::EXTERNAL_LINK, tr("Open on GameBanana"))
                .key(format!("{scope}-{slug}-open"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    if let Err(error) = tx.open_url(&page) {
                        st.status = format!("The browser could not open {page}: {error}");
                    }
                }),
        );
    }
    actions = actions.child(
        u.action(tr("Restore"), ActionKind::Ghost)
            .key(format!("{scope}-{slug}-restore"))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| match &restore {
                Restore::Mod(id) => unhide_mod(st, *id),
                Restore::Author(name) => unhide_author(st, name),
            }),
    );

    let hover_key = key.to_string();
    let mut body = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(strip)
            .child(text_col)
            .child(actions),
    );
    if let Some((_, edit)) = editing {
        let save_key = key.to_string();
        body = body.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .h(32.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, u.hair_strong)
                        .child(
                            rune::text_input(edit, BODY)
                                .key(format!("{scope}-{slug}-note-input"))
                                .label(fill(tr("Note for {}"), &[name]))
                                .single_line()
                                .grow(1.0)
                                .color(u.ink)
                                .placeholder(tr("Why did you hide this?"), u.faint)
                                .caret(u.accent, u.accent.alpha(0.22))
                                .on_edit(|st: &mut State| &mut st.browse_view.hidden_note.as_mut().expect("note").1)
                                .on_key(move |st: &mut State, ev: &KeyEvent, _| match ev.key {
                                    Key::Enter => {
                                        save_note(st);
                                        true
                                    }
                                    Key::Escape => {
                                        st.browse_view.hidden_note = None;
                                        true
                                    }
                                    _ => false,
                                }),
                        ),
                )
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .key(format!("{scope}-{slug}-note-cancel"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.browse_view.hidden_note = None),
                )
                .child(
                    u.action(tr("Save"), ActionKind::Secondary)
                        .key(format!("{scope}-{slug}-note-save"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            if st.browse_view.hidden_note.as_ref().is_some_and(|(k, _)| *k == save_key) {
                                save_note(st);
                            }
                        }),
                ),
        );
    }
    col()
        .key(format!("{scope}-{slug}"))
        .w_full()
        .min_h(56.0)
        .px(12.0)
        .py(8.0)
        .radius(crate::ui::r(8.0))
        .hover(u.hover)
        .on_hover_change(move |st: &mut State, inside: bool, _tx: &mut Tx| {
            if inside {
                st.browse_view.hidden_hover = Some(hover_key.clone());
            } else if st.browse_view.hidden_hover.as_deref() == Some(hover_key.as_str()) {
                st.browse_view.hidden_hover = None;
            }
        })
        .child(body)
}

/// Keep the note being written and close the field. An empty note removes it.
pub fn save_note(st: &mut State) {
    let Some((key, edit)) = st.browse_view.hidden_note.take() else { return };
    let note = clean_note(&edit.text);
    let meta = st.browse_view.hidden_meta.entry(key).or_default();
    meta.note = note;
    save_browse_state(st);
}

/// A picture pressed in Settings opens the mod in Browse, over the closed list.
fn to_browse(st: &mut State) {
    if st.screen != Screen::Browse {
        st.go(Screen::Browse);
    }
    st.browse_view.panel = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::catalog_mirror::MirrorEntry;

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(clean_note("  spoilers  "), "spoilers");
        assert_eq!(clean_note(&"x".repeat(300)).chars().count(), NOTE_MAX);
        assert_eq!(clean_note("   "), "");
    }

    #[test]
    fn the_line_says_when_and_where_from() {
        assert_eq!(hidden_line(None, "Creator"), "Creator");
        let old = HiddenMeta::default();
        assert_eq!(hidden_line(Some(&old), "Hidden mod"), "Hidden mod", "no date kept: the kind");
        let meta = HiddenMeta { hidden_at: 1_763_683_200, ..HiddenMeta::default() };
        assert_eq!(hidden_line(Some(&meta), "Creator"), "Hidden 2025-11-21");
        let from = HiddenMeta { from: Some((9, "Josuke Shiv".into())), ..meta };
        assert_eq!(hidden_line(Some(&from), "Creator"), "Hidden 2025-11-21 from Josuke Shiv");
    }

    #[test]
    fn a_creator_shows_their_three_most_liked_uploads() {
        let entry = |id: u64, author: &str, likes: u64, thumb: bool| MirrorEntry {
            id,
            model: "Mod".into(),
            name: format!("Mod {id}"),
            author: author.into(),
            likes,
            thumbnail_url: thumb.then(|| format!("https://images.gamebanana.com/{id}.jpg")),
            ..MirrorEntry::default()
        };
        let entries = vec![
            entry(1, "Krak4n", 5, true),
            entry(2, "krak4n", 50, true),
            entry(3, "Krak4n", 20, false),
            entry(4, "Krak4n", 10, true),
            entry(5, "Krak4n", 1, true),
            entry(6, "Someone", 99, true),
        ];
        let (pictures, count) = creator_pictures(&entries, "Krak4n");
        assert_eq!(count, 5);
        assert_eq!(pictures.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 4, 1]);
        assert_eq!(creator_pictures(&entries, "Nobody"), (Vec::new(), 0));
    }
}
