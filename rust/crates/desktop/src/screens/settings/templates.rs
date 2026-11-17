//! Discord presence templates: the top and bottom line Discord shows, per game phase (read
//! from console.log) and while you use Sigil, with placeholders and per-hero overrides. The
//! editor follows Deadlock Mod Manager's (tabs, chips, Reset this tab, Reset all, Discard,
//! Save; behaviour only). Presence text stays English by default; a template is the user's.

use std::collections::BTreeMap;

use deadlock_core::game_session::Session;
use rune::{Color, El, TextEdit, col, el, row, spacer, text};

use super::discord::Activity;
use super::i18n::{fill, tr};
use super::options::{self, Lines};
use super::button;
use crate::app::State;
use crate::ui::{ActionKind, BODY, BODY_MED, LABEL, MONO, SMALL, Ui};

/// Tabs: key, label, built-in top line, built-in bottom line.
pub const PHASES: [(&str, &str, &str, &str); 9] = [
    ("sigil", "In Sigil", "{page}", "{mods} mods on"),
    ("menu", "Main menu", "In the main menu", ""),
    ("hideout", "Hideout", "In the hideout", "Playing solo"),
    ("party_hideout", "Hideout, party", "In the hideout", "Party of {party}"),
    ("queue", "Finding a match", "Finding a match", ""),
    ("match", "In a match", "Playing {hero}", "{mode}"),
    ("party_match", "In a match, party", "Playing {hero}", "{mode}, party of {party}"),
    ("post_match", "After a match", "Match over", "Played {hero}"),
    ("spectating", "Spectating", "Watching a match", ""),
];

pub const PLACEHOLDERS: [(&str, &str); 6] = [
    ("{hero}", "Hero"),
    ("{mode}", "Mode"),
    ("{party}", "Party size"),
    ("{party_max}", "Party limit"),
    ("{mods}", "Mods on"),
    ("{page}", "Sigil page"),
];

/// Discord's limit for a line.
pub const MAX: usize = 128;

/// Values for the placeholders.
#[derive(Default, Clone)]
pub struct Values {
    pub hero: String,
    pub mode: String,
    pub party: u8,
    pub party_max: u8,
    pub mods: usize,
    pub page: String,
}

pub fn fill_line(template: &str, v: &Values) -> String {
    let out = template
        .replace("{hero}", &v.hero)
        .replace("{mode}", &v.mode)
        .replace("{party_max}", &v.party_max.to_string())
        .replace("{party}", &v.party.to_string())
        .replace("{mods}", &v.mods.to_string())
        .replace("{page}", &v.page);
    let trimmed = out.trim().trim_end_matches(',').trim();
    trimmed.chars().take(MAX).collect()
}

fn builtin(phase: &str) -> Lines {
    PHASES
        .iter()
        .find(|p| p.0 == phase)
        .map(|p| Lines {
            top: p.2.into(),
            bottom: p.3.into(),
        })
        .unwrap_or_default()
}

/// The lines for a phase: the hero's override, then the user's line, then the built-in, field
/// by field. An empty result falls back to the built-in.
pub fn render(phase: &str, hero: Option<&str>, o: &options::Options, v: &Values) -> (String, String) {
    let base = builtin(phase);
    let user = o.presence_lines.get(phase);
    let over = hero.and_then(|h| o.hero_lines.get(&format!("{h}/{phase}")));
    let pick = |f: fn(&Lines) -> &String| -> String {
        let t = over
            .map(f)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| user.map(f).filter(|s| !s.trim().is_empty()))
            .cloned()
            .unwrap_or_else(|| f(&base).clone());
        let out = fill_line(&t, v);
        if out.is_empty() { fill_line(f(&base), v) } else { out }
    };
    (pick(|l| &l.top), pick(|l| &l.bottom))
}

pub fn hero_name(codename: &str) -> String {
    deadlock_core::locker::display_name_for(codename)
}

/// Discord's activity while the game runs, from the console session. None when game presence
/// is off or the log says nothing yet.
pub fn game_activity(s: &State, since: u64) -> Option<Activity> {
    let o = options::get();
    if !o.presence_game {
        return None;
    }
    let session: &Session = super::watch::session(s)?;
    let hero = session.hero.as_deref().map(hero_name);
    let v = Values {
        hero: hero.clone().unwrap_or_default(),
        mode: session.mode().to_string(),
        party: session.party.max(1),
        party_max: session.party_max(),
        mods: s.addons.enabled_count(),
        page: String::new(),
    };
    let (top, bottom) = render(session.template_key(), hero.as_deref(), &o, &v);
    Some(Activity {
        details: top,
        state: (!bottom.is_empty()).then_some(bottom),
        since,
    })
}

/// Sigil's own activity through the "In Sigil" template, when the user wrote one.
pub fn app_activity(page: &str, mods: usize, since: u64) -> Option<Activity> {
    let o = options::get();
    o.presence_lines.get("sigil")?;
    let v = Values {
        page: page.to_string(),
        mods,
        ..Default::default()
    };
    let (top, bottom) = render("sigil", None, &o, &v);
    Some(Activity {
        details: top,
        state: (!bottom.is_empty() && !(mods == 0 && bottom.starts_with('0'))).then_some(bottom),
        since,
    })
}

/// Settings > Integrations > Discord: game presence and the lines editor.
pub fn rows(s: &State, u: &Ui, presence_on: bool) -> Vec<El> {
    let _ = s;
    let game_on = options::get().presence_game;
    vec![
        super::disabled(
            super::toggle_row(
                u,
                "settings-discord-game",
                "Game presence",
                "Your match phase and hero, read from the game's console log (-condebug).",
                game_on,
                |st| {
                    let on = !options::get().presence_game;
                    options::edit(st, |o| {
                        o.presence_game = on;
                        if on {
                            o.condebug = true;
                        }
                    });
                    if on {
                        let current = st.settings.launch_options.text.clone();
                        if !deadlock_core::game_session::has_condebug(&current)
                            && st.settings.launch_read.as_ref().is_some_and(|r| r.path.is_some())
                        {
                            let value = deadlock_core::game_session::with_condebug(&current, true);
                            st.settings.launch_options = TextEdit::new(value.clone());
                            st.settings.confirm = Some(super::Confirm::LaunchOptions(value));
                        }
                    }
                },
            ),
            !presence_on,
        ),
        super::setting_row(
            u,
            "settings-discord-lines",
            "Discord lines",
            super::detail(u, "The top and bottom line for each phase, with your own words and per-hero lines."),
            button(u, "settings-discord-lines-edit", "Edit", ActionKind::Secondary).on_press(open),
        ),
    ]
}

// --- editor -----------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Draft {
    pub lines: BTreeMap<String, Lines>,
    pub heroes: BTreeMap<String, Lines>,
    pub tab: &'static str,
    pub hero: Option<String>,
    pub top: TextEdit,
    pub bottom: TextEdit,
    /// 0 top, 1 bottom: where a chip goes.
    pub field: u8,
    pub picking: bool,
}

impl Draft {
    pub fn new() -> Draft {
        let o = options::get();
        let mut d = Draft {
            lines: o.presence_lines.clone(),
            heroes: o.hero_lines.clone(),
            tab: "match",
            hero: None,
            top: TextEdit::new(""),
            bottom: TextEdit::new(""),
            field: 0,
            picking: false,
        };
        d.load();
        d
    }

    fn key(&self) -> String {
        match &self.hero {
            Some(h) => format!("{h}/{}", self.tab),
            None => self.tab.to_string(),
        }
    }

    fn load(&mut self) {
        let key = self.key();
        let map = if self.hero.is_some() { &self.heroes } else { &self.lines };
        let l = map.get(&key).cloned().unwrap_or_default();
        self.top = TextEdit::new(l.top);
        self.bottom = TextEdit::new(l.bottom);
    }

    /// Put the fields into the maps; empty fields drop the entry.
    pub fn commit(&mut self) {
        let key = self.key();
        let l = Lines {
            top: self.top.text.trim().to_string(),
            bottom: self.bottom.text.trim().to_string(),
        };
        let map = if self.hero.is_some() { &mut self.heroes } else { &mut self.lines };
        if l.top.is_empty() && l.bottom.is_empty() {
            map.remove(&key);
        } else {
            map.insert(key, l);
        }
    }

    pub fn switch(&mut self, tab: &'static str, hero: Option<String>) {
        self.commit();
        self.tab = tab;
        self.hero = hero;
        self.picking = false;
        self.load();
    }

    pub fn changed(&self) -> bool {
        let mut c = self.clone();
        c.commit();
        let o = options::get();
        c.lines != o.presence_lines || c.heroes != o.hero_lines
    }

    fn custom(&self, tab: &str) -> bool {
        self.lines.contains_key(tab) || self.heroes.keys().any(|k| k.ends_with(&format!("/{tab}")))
    }
}

fn draft(st: &mut State) -> &mut Draft {
    st.settings.extra.draft.get_or_insert_with(Draft::new)
}

pub fn open(st: &mut State) {
    st.settings.extra.draft = Some(Draft::new());
    st.settings.extra.dialog = Some(super::extra::Dialog::Templates);
}

fn save(st: &mut State) {
    let mut d = draft(st).clone();
    d.commit();
    options::edit(st, |o| {
        o.presence_lines = d.lines.clone();
        o.hero_lines = d.heroes.clone();
    });
    st.settings.extra.draft = None;
    super::extra::close(st);
    super::say(st, tr("Presence lines saved."), false);
}

fn chip(u: &Ui, key: String, label: &str, on: bool) -> El {
    row()
        .key(key)
        .h(28.0)
        .px(10.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .border(1.0, if on { u.accent } else { u.hair })
        .hover(if on { u.selected } else { u.hover })
        .focusable(true)
        .role("tab")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .child(text(label.to_string()).font(LABEL).color(if on { u.ink } else { u.muted }).a11y_hidden())
}

fn line_field(u: &Ui, key: &str, label: &str, edit: &TextEdit, placeholder: &str, which: u8) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(text(tr(label).to_string()).font(BODY_MED).color(u.ink))
        .child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    rune::text_input(edit, MONO)
                        .key(key.to_string())
                        .label(tr(label).to_string())
                        .single_line()
                        .grow(1.0)
                        .color(u.ink)
                        .placeholder(placeholder.to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.22))
                        .on_edit(move |st: &mut State| {
                            let d = draft(st);
                            d.field = which;
                            if which == 0 { &mut d.top } else { &mut d.bottom }
                        }),
                ),
        )
}

pub fn dialog(s: &State, u: &Ui) -> El {
    let Some(d) = s.settings.extra.draft.as_ref() else {
        return el();
    };
    let base = builtin(d.tab);
    let mut tabs = el().key("settings-presence-tabs").w_full().equal_columns(3).gap(6.0);
    for (key, label, _, _) in PHASES {
        let mark = if d.custom(key) { format!("{} ·", tr(label)) } else { tr(label).to_string() };
        tabs = tabs.child(
            chip(u, format!("settings-presence-tab-{key}"), &mark, d.tab == key)
                .on_press(move |st: &mut State| {
                    let hero = if key == "sigil" { None } else { draft(st).hero.clone() };
                    draft(st).switch(key, hero)
                }),
        );
    }
    let scope_label = match &d.hero {
        Some(h) => fill(tr("Only when playing {}"), &[h]),
        None => tr("All heroes").to_string(),
    };
    let mut scope = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(text(tr("Customize for")).font(SMALL).color(u.muted))
            .child(
                button(u, "settings-presence-scope", &scope_label, ActionKind::Secondary)
                    .on_press(|st: &mut State| {
                        let d = draft(st);
                        d.picking = !d.picking
                    }),
            )
            .child(if d.hero.is_some() {
                button(u, "settings-presence-hero-reset", "Reset to default", ActionKind::Ghost).on_press(|st: &mut State| {
                    let d = draft(st);
                    if let Some(h) = d.hero.clone() {
                        d.heroes.retain(|k, _| !k.starts_with(&format!("{h}/")));
                        let tab = d.tab;
                        d.hero = None;
                        d.switch(tab, Some(h));
                    }
                })
            } else {
                el()
            }),
    );
    if d.picking && d.tab != "sigil" {
        let mut grid = el().key("settings-presence-heroes").w_full().equal_columns(5).gap(4.0);
        grid = grid.child(chip(u, "settings-presence-hero-all".into(), tr("All heroes"), d.hero.is_none()).on_press(|st: &mut State| {
            let tab = draft(st).tab;
            draft(st).switch(tab, None)
        }));
        for hero in deadlock_core::locker::ROSTER.iter() {
            let name = hero.name.to_string();
            let on = d.hero.as_deref() == Some(hero.name);
            grid = grid.child(chip(u, format!("settings-presence-hero-{}", hero.name), hero.name, on).on_press(move |st: &mut State| {
                let tab = draft(st).tab;
                draft(st).switch(tab, Some(name.clone()))
            }));
        }
        scope = scope.child(col().w_full().max_h(180.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(grid));
    }
    let mut chips = row().w_full().gap(6.0).center_y().child(text(tr("Insert")).font(SMALL).color(u.muted));
    for (token, label) in PLACEHOLDERS {
        chips = chips.child(
            chip(u, format!("settings-presence-chip-{label}"), tr(label), false)
                .on_press(move |st: &mut State| {
                    let d = draft(st);
                    let edit = if d.field == 0 { &mut d.top } else { &mut d.bottom };
                    let mut t = edit.text.clone();
                    if !t.is_empty() && !t.ends_with(' ') {
                        t.push(' ');
                    }
                    t.push_str(token);
                    *edit = TextEdit::new(t);
                }),
        );
    }
    // Preview with sample values.
    let mut preview_o = options::get();
    let mut c = d.clone();
    c.commit();
    preview_o.presence_lines = c.lines;
    preview_o.hero_lines = c.heroes;
    let sample = Values {
        hero: d.hero.clone().unwrap_or_else(|| "Haze".into()),
        mode: "Standard (6v6)".into(),
        party: 3,
        party_max: 6,
        mods: 12,
        page: "Browsing mods".into(),
    };
    let (top, bottom) = render(d.tab, d.hero.as_deref(), &preview_o, &sample);
    let changed = d.changed();
    super::extra::shell(u, "settings-presence-dialog", tr("Discord lines"))
        .w(640.0)
        .child(
            text(tr("An empty line uses Sigil's own text."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(tabs)
        .child(scope)
        .child(line_field(u, "settings-presence-top", "Top line", &d.top, &base.top, 0))
        .child(line_field(u, "settings-presence-bottom", "Bottom line", &d.bottom, &base.bottom, 1))
        .child(chips)
        .child(
            col()
                .w_full()
                .padding(12.0)
                .gap(2.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(text(tr("Discord shows")).font(SMALL).color(u.faint))
                .child(text(top).font(BODY_MED).color(u.ink).truncate())
                .child(text(bottom).font(SMALL).color(u.muted).truncate()),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(button(u, "settings-presence-reset-tab", "Reset this tab", ActionKind::Ghost).on_press(|st: &mut State| {
                    let d = draft(st);
                    d.top = TextEdit::new("");
                    d.bottom = TextEdit::new("");
                }))
                .child(button(u, "settings-presence-reset-all", "Reset all", ActionKind::Ghost).on_press(|st: &mut State| {
                    let d = draft(st);
                    d.lines.clear();
                    d.heroes.clear();
                    d.top = TextEdit::new("");
                    d.bottom = TextEdit::new("");
                }))
                .child(spacer())
                .child(button(u, "settings-presence-discard", "Discard", ActionKind::Ghost).on_press(|st: &mut State| {
                    st.settings.extra.draft = None;
                    super::extra::close(st)
                }))
                .child(super::disabled(
                    button(u, "settings-presence-save", "Save changes", ActionKind::Primary).on_press(save),
                    !changed,
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_fill_and_lines_fall_back() {
        let v = Values {
            hero: "Haze".into(),
            mode: "Street Brawl".into(),
            party: 2,
            party_max: 4,
            mods: 7,
            page: "Browsing mods".into(),
        };
        assert_eq!(fill_line("{hero} ({party}/{party_max}), {mods} mods", &v), "Haze (2/4), 7 mods");
        let mut o = options::Options::default();
        assert_eq!(render("match", Some("Haze"), &o, &v), ("Playing Haze".into(), "Street Brawl".into()));
        o.presence_lines.insert("match".into(), Lines { top: "Grinding as {hero}".into(), bottom: String::new() });
        o.hero_lines.insert("Haze/match".into(), Lines { top: String::new(), bottom: "Sleep dart time".into() });
        assert_eq!(render("match", Some("Haze"), &o, &v), ("Grinding as Haze".into(), "Sleep dart time".into()));
        assert_eq!(render("match", Some("Abrams"), &o, &v).1, "Street Brawl");
        o.presence_lines.insert("menu".into(), Lines { top: "{hero}".into(), bottom: String::new() });
        let empty = Values::default();
        assert_eq!(render("menu", None, &o, &empty).0, "In the main menu", "an empty result uses the built-in");
        assert_eq!(fill_line(&"x".repeat(300), &v).len(), MAX);
    }

    #[test]
    fn drafts_commit_per_tab_and_hero() {
        let mut d = Draft {
            lines: BTreeMap::new(),
            heroes: BTreeMap::new(),
            tab: "match",
            hero: None,
            top: TextEdit::new("A"),
            bottom: TextEdit::new(""),
            field: 0,
            picking: false,
        };
        d.switch("hideout", None);
        assert_eq!(d.lines.get("match").unwrap().top, "A");
        d.top = TextEdit::new("H");
        d.switch("match", Some("Haze".into()));
        assert!(d.heroes.is_empty());
        d.bottom = TextEdit::new("B");
        d.switch("match", None);
        assert_eq!(d.heroes.get("Haze/match").unwrap().bottom, "B");
        assert_eq!(d.top.text, "A", "switching back loads what was there");
    }
}
