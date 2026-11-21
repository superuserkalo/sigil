//! Management dialogs over Browse: hidden mods and creators, ignored updates and the hero
//! picker.

use super::*;

/// The hidden items list, with a way back for each one.
pub(crate) fn hidden_dialog(s: &State, u: &Ui) -> Option<El> {
    if s.browse_view.panel != Some(Panel::Hidden) {
        return None;
    }
    let close = |st: &mut State| st.browse_view.panel = None;
    let mods = &s.browse_view.hidden_mods;
    let authors = &s.browse_view.hidden_authors;
    let mut rows = col().w_full();
    if mods.is_empty() && authors.is_empty() {
        rows = rows.child(
            row()
                .w_full()
                .py(24.0)
                .center()
                .child(text("Nothing is hidden.").font(BODY).color(u.muted)),
        );
    }
    for (index, item) in super::hidden::rows(s, u, "hidden").into_iter().enumerate() {
        if index > 0 {
            rows = rows.child(el().w_full().h(1.0).fill(u.hair));
        }
        rows = rows.child(item);
    }
    Some(
        u.dialog_shell()
            .key("browse-hidden-dialog")
            .role("dialog")
            .label("Hidden items")
            .w(600.0)
            .max_w(600.0)
            .max_h(640.0)
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
                    .gap(4.0)
                    .child(text("Hidden items").font(H1).color(u.ink))
                    .child(
                        text("Hidden mods and creators stay out of Browse until you restore them.")
                            .font(SMALL)
                            .color(u.muted),
                    ),
            )
            .child(
                col()
                    .w_full()
                    .max_h(400.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(rows),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(spacer())
                    .child(
                        u.action("Restore all", ActionKind::Ghost)
                            .key("hidden-restore-all")
                            .disabled(mods.is_empty() && authors.is_empty())
                            .opacity(if mods.is_empty() && authors.is_empty() { 0.45 } else { 1.0 })
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                st.browse_view.hidden_mods.clear();
                                st.browse_view.hidden_authors.clear();
                                st.browse_view.hidden_meta.clear();
                                st.browse_view.hidden_note = None;
                                save_browse_state(st);
                            }),
                    )
                    .child(
                        u.action("Done", ActionKind::Secondary)
                            .key("hidden-done")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}

/// The updates the player chose to skip.
pub(crate) fn ignored_dialog(s: &State, u: &Ui) -> Option<El> {
    if s.browse_view.panel != Some(Panel::Ignored) {
        return None;
    }
    let close = |st: &mut State| st.browse_view.panel = None;
    let ids = &s.browse_view.ignored_updates;
    let mut rows = col().w_full();
    if ids.is_empty() {
        rows = rows.child(
            row()
                .w_full()
                .py(24.0)
                .center()
                .child(text("No ignored updates.").font(BODY).color(u.muted)),
        );
    }
    for (index, id) in ids.iter().enumerate() {
        let id = *id;
        let name = name_for_id(s, id).unwrap_or_else(|| format!("Mod {id}"));
        if index > 0 {
            rows = rows.child(el().w_full().h(1.0).fill(u.hair));
        }
        rows = rows.child(
            row()
                .key(format!("ignored-{id}"))
                .w_full()
                .min_h(48.0)
                .px(12.0)
                .py(8.0)
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(name).font(BODY).color(u.ink).truncate())
                        .child(text(format!("Mod {id}")).font(SMALL).color(u.faint)),
                )
                .child(
                    u.action("Check again", ActionKind::Ghost)
                        .key(format!("ignored-restore-{id}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| set_update_ignored(st, id, false)),
                ),
        );
    }
    Some(
        u.dialog_shell()
            .key("browse-ignored-dialog")
            .role("dialog")
            .label("Ignored updates")
            .w(480.0)
            .max_w(480.0)
            .max_h(600.0)
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
                    .gap(4.0)
                    .child(text("Ignored updates").font(H1).color(u.ink))
                    .child(
                        text("Sigil will not offer these updates again until you check.")
                            .font(SMALL)
                            .color(u.muted),
                    ),
            )
            .child(
                col()
                    .w_full()
                    .max_h(360.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(rows),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(spacer())
                    .child(
                        u.action("Done", ActionKind::Secondary)
                            .key("ignored-done")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            ),
    )
}

pub(crate) fn name_for_id(s: &State, id: u64) -> Option<String> {
    s.browse
        .page
        .as_ref()
        .and_then(|page| page.mods.iter().find(|card| card.id == id))
        .map(|card| card.name.clone())
        .or_else(|| {
            s.installed
                .library
                .mods
                .iter()
                .find(|meta| meta.gamebanana_id == Some(id))
                .map(|meta| meta.name.clone())
        })
        .or_else(|| {
            s.browse_view
                .mirror
                .as_ref()
                .and_then(|mirror| mirror.entries.iter().find(|entry| entry.id == id))
                .map(|entry| entry.name.clone())
        })
}

/// The hero picker: a search field over the Skins tree. Writes the grid filter or the open
/// mod's hero, depending on who opened it.
pub(crate) fn hero_picker_dialog(s: &State, u: &Ui) -> Option<El> {
    let target = s.browse_view.hero_picker?;
    let close = |st: &mut State| st.browse_view.hero_picker = None;
    let query = s.browse_view.hero_query.text.trim().to_lowercase();
    let heroes: Vec<Pick> = hero_list(s)
        .into_iter()
        .filter(|hero| query.is_empty() || hero.name.to_lowercase().contains(&query))
        .collect();
    let loading = hero_list(s).is_empty();
    let mut rows = col().w_full();
    if loading {
        rows = rows.child(
            row()
                .w_full()
                .py(20.0)
                .center()
                .gap(8.0)
                .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
                .child(text("Loading the heroes").font(BODY).color(u.muted)),
        );
    } else if heroes.is_empty() {
        rows = rows.child(
            row()
                .w_full()
                .py(20.0)
                .center()
                .child(text("No hero by that name.").font(BODY).color(u.muted)),
        );
    }
    for hero in heroes {
        let name = hero.name.clone();
        let portrait_url = hero.icon_for(s);
        let portrait = portrait_url
            .as_ref()
            .and_then(|url| s.thumbnails.get(url));
        rows = rows.child(
            row()
                .key(format!("hero-pick-{}", hero.id))
                .w_full()
                .h(44.0)
                .px(10.0)
                .gap(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(hero.name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    match target {
                        HeroTarget::Details => {
                            if let Some(details) = st.browse_view.details.as_mut() {
                                details.hero_override = Some(name.clone());
                                details.hero = Some(name.clone());
                            }
                            st.browse_view.hero_picker = None;
                        }
                        HeroTarget::Filter => {
                            let skins_tab = st
                                .browse_view
                                .filters
                                .category
                                .as_ref()
                                .is_none_or(|pick| pick.name.eq_ignore_ascii_case("Skins"));
                            if st.browse_view.tab == Tab::Mods
                                && skins_tab
                                && let Some(skins) = skins_pick(&*st)
                            {
                                let sub = Some(Pick {
                                    id: hero.id,
                                    name: name.clone(),
                                });
                                set_filters(st, Some(tx), move |f| {
                                    f.category = Some(skins);
                                    f.sub = sub;
                                    f.hero = None;
                                });
                            } else {
                                let hero = name.clone();
                                set_filters(st, Some(tx), move |f| {
                                    f.hero = Some(hero);
                                });
                            }
                        }
                    }
                })
                .child(match portrait {
                    Some(pixels) => el()
                        .size(28.0, 28.0)
                        .radius(14.0)
                        .pixels_fit(pixels, SurfaceFit::Fill)
                        .a11y_hidden(),
                    None => el()
                        .size(28.0, 28.0)
                        .radius(14.0)
                        .center()
                        .fill(u.hover)
                        .child(
                            text(hero.name.chars().next().unwrap_or('?').to_string())
                                .font(SMALL)
                                .color(u.muted),
                        ),
                })
                .child(text(hero.name.clone()).font(BODY).color(u.ink).truncate())
                .child(spacer()),
        );
    }
    Some(
        u.dialog_shell()
            .key("browse-hero-picker")
            .role("dialog")
            .label("Choose a hero")
            .w(440.0)
            .max_w(440.0)
            .max_h(640.0)
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
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(text("Choose a hero").font(H1).color(u.ink))
                    .child(spacer())
                    .child(
                        u.icon_button(&icons::X, "Close")
                            .key("hero-picker-close")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .h(32.0)
                    .px(10.0)
                    .gap(8.0)
                    .center_y()
                    .radius(crate::ui::r(8.0))
                    .fill(u.field)
                    .border(1.0, u.hair)
                    .child(icon(&icons::SEARCH, 14.0).color(u.faint).a11y_hidden())
                    .child(
                        rune::text_input(&s.browse_view.hero_query, BODY)
                            .key("hero-picker-search")
                            .label("Find a hero")
                            .single_line()
                            .grow(1.0)
                            .color(u.ink)
                            .placeholder("Find a hero", u.faint)
                            .caret(u.accent, u.accent.alpha(0.20))
                            .on_edit(|st: &mut State| &mut st.browse_view.hero_query),
                    ),
            )
            .child(
                col()
                    .w_full()
                    .max_h(420.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(rows),
            )
            .child(
                row().w_full().gap(8.0).center_y().child(spacer()).child(
                    u.action("Cancel", ActionKind::Ghost)
                        .key("hero-picker-cancel")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                ),
            ),
    )
}

trait HeroIcon {
    fn icon_for(&self, s: &State) -> Option<String>;
}

impl HeroIcon for Pick {
    fn icon_for(&self, s: &State) -> Option<String> {
        categories_of(s, BrowseSection::Mods)?
            .iter()
            .find(|root| root.name.eq_ignore_ascii_case("Skins"))?
            .children
            .iter()
            .find(|child| child.id == self.id)?
            .icon_url
            .clone()
    }
}
