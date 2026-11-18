//! Filter menus, the card context menu and the view menu.

use super::*;

// --- filter menus ------------------------------------------------------------------

/// The open filter dropdown, mounted at the root so it draws over the page and escapes its
/// scroll.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    if s.screen != Screen::Browse {
        return None;
    }
    if s.browse_view.context.is_some() {
        return context_menu(s, u);
    }
    // The docked details' file menu; the modal mounts its own over the dialog.
    if s.browse_view.docked
        && let Some(menu) = super::super::quick::files_menu(s, u)
    {
        return Some(menu);
    }
    let menu = s.browse_view.menu?;
    let f = &s.browse_view.filters;
    let (align, width) = match menu {
        FilterMenu::Sort => (Align::End, 232.0),
        FilterMenu::Sub => (Align::Start, 288.0),
        FilterMenu::Date => (Align::Start, 312.0),
        FilterMenu::View => (Align::End, 268.0),
        _ => (Align::Start, 248.0),
    };
    let mut items = col().w_full().gap(0.0);
    match menu {
        FilterMenu::Category => match categories_of(s, f.section) {
            Some(list) => {
                items = items.child(
                    menu_item(u, "menu-cat-all", None, "All categories", "", f.category.is_none())
                        .on_press_tx(|st: &mut State, tx: &mut Tx| {
                            set_filters(st, Some(tx), |f| {
                                f.category = None;
                                f.sub = None;
                            })
                        }),
                );
                for root in list {
                    let pick = Pick {
                        id: root.id,
                        name: root.name.clone(),
                    };
                    let on = f.category.as_ref().is_some_and(|p| p.id == root.id);
                    items = items.child(
                        menu_item(
                            u,
                            &format!("menu-cat-{}", root.id),
                            None,
                            &root.name,
                            &format_count(root.count),
                            on,
                        )
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            let pick = pick.clone();
                            set_filters(st, Some(tx), move |f| {
                                if f.category.as_ref().map(|p| p.id) != Some(pick.id) {
                                    f.sub = None;
                                }
                                f.category = Some(pick);
                            })
                        }),
                    );
                }
            }
            None => items = items.child(categories_pending(s, u, f.section)),
        },
        FilterMenu::Sub => {
            let root = f.category.as_ref().and_then(|pick| {
                categories_of(s, f.section)?
                    .iter()
                    .find(|root| root.id == pick.id)
            })?;
            let noun = sub_noun(&root.name);
            let filter = s.browse_view.menu_query.text.trim().to_lowercase();
            if root.children.len() > 10 {
                items = items.child(
                    row()
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .mb(4.0)
                        .gap(8.0)
                        .center_y()
                        .radius(crate::ui::r(6.0))
                        .fill(u.field)
                        .border(1.0, u.hair)
                        .child(icon(&icons::SEARCH, 14.0).color(u.faint).a11y_hidden())
                        .child(
                            rune::text_input(&s.browse_view.menu_query, SMALL)
                                .key("menu-sub-search")
                                .label(format!("Find a {}", noun.to_lowercase()))
                                .single_line()
                                .grow(1.0)
                                .color(u.ink)
                                .placeholder(format!("Find a {}", noun.to_lowercase()), u.faint)
                                .caret(u.accent, u.accent.alpha(0.20))
                                .on_edit(|st: &mut State| &mut st.browse_view.menu_query),
                        ),
                );
            }
            let mut list = col().w_full();
            let portraits = root.children.iter().any(|child| child.icon_url.is_some());
            if filter.is_empty() {
                list = list.child(
                    menu_item(
                        u,
                        "menu-sub-all",
                        portraits.then(|| el().size(20.0, 20.0)),
                        &format!("Every {}", noun.to_lowercase()),
                        "",
                        f.sub.is_none(),
                    )
                    .on_press_tx(|st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), |f| f.sub = None)
                    }),
                );
            }
            let mut children: Vec<&BrowseCategory> = root
                .children
                .iter()
                .filter(|child| filter.is_empty() || child.name.to_lowercase().contains(&filter))
                .collect();
            children.sort_by_key(|child| child.name.to_lowercase());
            for child in children {
                let pick = Pick {
                    id: child.id,
                    name: child.name.clone(),
                };
                let on = f.sub.as_ref().is_some_and(|p| p.id == child.id);
                let portrait = child
                    .icon_url
                    .as_ref()
                    .and_then(|url| s.thumbnails.get(url));
                // Every row keeps the portrait slot so the names share one edge.
                let lead = match portrait {
                    Some(pixels) => Some(
                        el().size(20.0, 20.0)
                            .radius(4.0)
                            .pixels_fit(pixels, SurfaceFit::Fill)
                            .a11y_hidden(),
                    ),
                    None if portraits => Some(el().size(20.0, 20.0).radius(4.0).fill(u.hover)),
                    None => None,
                };
                let item = menu_item(
                    u,
                    &format!("menu-sub-{}", child.id),
                    lead,
                    &child.name,
                    &format_count(child.count),
                    on,
                );
                list = list.child(item.on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    let pick = pick.clone();
                    set_filters(st, Some(tx), move |f| f.sub = Some(pick))
                }));
            }
            items = items.child(
                col()
                    .key("menu-sub-scroll")
                    .w_full()
                    .max_h(360.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(list),
            );
        }
        FilterMenu::Sort => {
            let query = current_query(s);
            let shown = query.shown_order();
            for order in BrowseOrder::ALL {
                // A dated window is ordered here and rows carry no download count.
                let usable = !(order == BrowseOrder::MostDownloaded && f.period != BrowsePeriod::Any);
                let item = menu_item(
                    u,
                    &format!("menu-sort-{}", order.index_token()),
                    None,
                    order.label(),
                    if usable { "" } else { "Any time only" },
                    order == shown,
                );
                items = items.child(if usable {
                    item.on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), move |f| f.order = Some(order))
                    })
                } else {
                    item.opacity(0.5)
                });
            }
        }
        FilterMenu::More => {
            items = items.child(
                menu_item(
                    u,
                    "menu-featured",
                    Some(icon(&icons::SPARKLES, 14.0).color(u.muted).a11y_hidden()),
                    "Featured only",
                    "",
                    f.featured,
                )
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    set_filters(st, Some(tx), |f| f.featured = !f.featured)
                }),
            );
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Installed"));
            for owned in Owned::ALL {
                items = items.child(
                    menu_item(u, &format!("menu-owned-{owned:?}"), None, owned.label(), "", f.owned == owned)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            set_filters(st, Some(tx), move |f| f.owned = owned)
                        }),
                );
            }
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Mature content"));
            for mature in MatureView::ALL {
                items = items.child(
                    menu_item(
                        u,
                        &format!("menu-mature-{mature:?}"),
                        None,
                        mature.label(),
                        "",
                        f.mature == mature,
                    )
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), move |f| f.mature = mature)
                    }),
                );
            }
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Match"));
            for exclude in [false, true] {
                items = items.child(
                    menu_item(
                        u,
                        if exclude { "menu-exclude" } else { "menu-include" },
                        None,
                        if exclude { "Exclude the picks" } else { "Include the picks" },
                        "",
                        f.exclude == exclude,
                    )
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), move |f| f.exclude = exclude)
                    }),
                );
            }
            items = items.child(el().w_full().h(1.0).my(4.0).fill(u.hair)).child(
                menu_item(
                    u,
                    "menu-outdated",
                    None,
                    "Hide outdated",
                    "Before Jan 22",
                    f.hide_outdated,
                )
                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                    set_filters(st, Some(tx), |f| f.hide_outdated = !f.hide_outdated)
                }),
            );
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(
                    menu_item(
                        u,
                        "menu-hidden",
                        Some(icon(&EYE_OFF, 14.0).color(u.muted).a11y_hidden()),
                        "Hidden items",
                        &format_count(
                            (s.browse_view.hidden_mods.len() + s.browse_view.hidden_authors.len())
                                as u64,
                        ),
                        false,
                    )
                    .on_press(|st: &mut State| {
                        st.browse_view.panel = Some(Panel::Hidden);
                        st.browse_view.menu = None;
                    }),
                )
                .child(
                    menu_item(
                        u,
                        "menu-ignored",
                        Some(icon(&icons::REFRESH_CW, 14.0).color(u.muted).a11y_hidden()),
                        "Ignored updates",
                        &format_count(s.browse_view.ignored_updates.len() as u64),
                        false,
                    )
                    .on_press(|st: &mut State| {
                        st.browse_view.panel = Some(Panel::Ignored);
                        st.browse_view.menu = None;
                    }),
                );
        }
        FilterMenu::Date => {
            items = items.child(menu_label(u, "Show by"));
            for field in DateField::ALL {
                items = items.child(
                    menu_item(
                        u,
                        &format!("menu-date-field-{}", field.id()),
                        None,
                        field.label(),
                        "",
                        f.date_field == field,
                    )
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), move |f| f.date_field = field)
                    }),
                );
            }
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Window"));
            for period in BrowsePeriod::ALL {
                items = items.child(
                    menu_item(
                        u,
                        &format!("menu-date-{}", period.id()),
                        None,
                        period.label(),
                        "",
                        f.period == period && f.date_from == 0 && f.date_to == 0,
                    )
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                        set_filters(st, Some(tx), move |f| {
                            f.period = period;
                            f.date_from = 0;
                            f.date_to = 0;
                        })
                    }),
                );
            }
            let field = |label: &str, edit: &TextEdit, side: &'static str| {
                row()
                    .w(148.0)
                    .h(32.0)
                    .px(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(crate::ui::r(6.0))
                    .fill(u.field)
                    .border(1.0, u.hair)
                    .child(icon(&CLOCK, 12.0).color(u.faint).a11y_hidden())
                    .child(
                        rune::text_input(edit, SMALL)
                            .key(format!("menu-date-{side}"))
                            .label(label.to_string())
                            .single_line()
                            .grow(1.0)
                            .color(u.ink)
                            .placeholder("YYYY-MM-DD", u.faint)
                            .caret(u.accent, u.accent.alpha(0.20))
                            .on_edit(move |st: &mut State| {
                                if side == "from" {
                                    &mut st.browse_view.date_from_text
                                } else {
                                    &mut st.browse_view.date_to_text
                                }
                            }),
                    )
            };
            let from = field("From", &s.browse_view.date_from_text, "from");
            let to = field("To", &s.browse_view.date_to_text, "to");
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Custom range"))
                .child(row().w_full().gap(8.0).child(from).child(to))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .py(4.0)
                        .child(spacer())
                        .child(
                            u.action("Clear", ActionKind::Ghost)
                                .key("menu-date-clear")
                                .cursor(rune::Cursor::Pointer)
                                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                                    st.browse_view.date_from_text = TextEdit::new("");
                                    st.browse_view.date_to_text = TextEdit::new("");
                                    set_filters(st, Some(tx), |f| {
                                        f.date_from = 0;
                                        f.date_to = 0;
                                        f.period = BrowsePeriod::Any;
                                    })
                                }),
                        )
                        .child(
                            u.action("Apply", ActionKind::Primary)
                                .key("menu-date-apply")
                                .cursor(rune::Cursor::Pointer)
                                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                                    let from = st.browse_view.date_from_text.text.trim().to_string();
                                    let to = st.browse_view.date_to_text.text.trim().to_string();
                                    let parsed_from = if from.is_empty() {
                                        Some(0)
                                    } else {
                                        parse_date(&from)
                                    };
                                    let parsed_to = if to.is_empty() {
                                        Some(0)
                                    } else {
                                        parse_date(&to)
                                    };
                                    let (Some(from), Some(to)) = (parsed_from, parsed_to) else {
                                        st.status = "Dates read as YYYY-MM-DD, for example 2026-01-22.".to_string();
                                        return;
                                    };
                                    if from > 0 && to > 0 && to < from {
                                        st.status = "The end of the range is before its start.".to_string();
                                        return;
                                    }
                                    set_filters(st, Some(tx), move |f| {
                                        f.date_from = from;
                                        f.date_to = to;
                                        f.period = BrowsePeriod::Any;
                                    });
                                }),
                        ),
                );
        }
        FilterMenu::View => {
            items = items.child(menu_label(u, "Layout"));
            for layout in Layout::ALL {
                items = items.child(
                    menu_item(
                        u,
                        &format!("menu-layout-{}", layout.id()),
                        None,
                        layout.label(),
                        "",
                        s.browse_view.layout == layout,
                    )
                    .on_press(move |st: &mut State| {
                        st.browse_view.layout = layout;
                        save_browse_state(st);
                    }),
                );
            }
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Card size"));
            for size in CardSize::ALL {
                items = items.child(
                    menu_item(
                        u,
                        &format!("menu-size-{}", size.id()),
                        None,
                        size.label(),
                        "",
                        s.browse_view.card_size == size,
                    )
                    .on_press(move |st: &mut State| {
                        st.browse_view.card_size = size;
                        save_browse_state(st);
                    }),
                );
            }
            items = items
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(
                    menu_item(
                        u,
                        "menu-paged",
                        None,
                        "Numbered pages",
                        "",
                        s.browse_view.paged,
                    )
                    .on_press(|st: &mut State| {
                        st.browse_view.paged = !st.browse_view.paged;
                        st.browse_view.auto_streak = 0;
                        save_browse_state(st);
                        if !st.demo {
                            crate::data::start_browse_fetch(st, 1);
                        }
                    }),
                )
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(menu_label(u, "Preview volume"))
                .child(
                    row()
                        .w_full()
                        .px(10.0)
                        .py(6.0)
                        .gap(10.0)
                        .center_y()
                        .child(audio_slider(s, u, "browse-audio-volume"))
                        .child(
                            text(format!("{}%", (s.browse_view.audio.volume * 100.0).round() as i32))
                                .font(SMALL)
                                .color(u.faint)
                                .w(40.0),
                        ),
                )
                .child(el().w_full().h(1.0).my(4.0).fill(u.hair))
                .child(
                    row()
                        .w_full()
                        .px(10.0)
                        .py(6.0)
                        .gap(8.0)
                        .items_start()
                        .child(icon(&icons::BOX, 14.0).color(u.faint).a11y_hidden())
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(2.0)
                                .child(
                                    text("Saved catalog")
                                        .font(BODY)
                                        .color(u.prose),
                                )
                                .child(
                                    text(if s.browse_view.mirror_summary.is_empty() {
                                        "Not saved yet".to_string()
                                    } else {
                                        s.browse_view.mirror_summary.clone()
                                    })
                                    .font(SMALL)
                                    .color(u.faint)
                                    .wrap(),
                                ),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .py(4.0)
                        .child(spacer())
                        .child(
                            u.action("Save now", ActionKind::Ghost)
                                .key("menu-mirror-sync")
                                .cursor(rune::Cursor::Pointer)
                                .on_press(|st: &mut State| start_mirror_sync(st)),
                        ),
                );
        }
    }
    Some(
        col()
            .key("browse-menu")
            .role("menu")
            .w(width)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
            .anchored(menu.anchor(), Side::Bottom, align, 6.0)
            .modal(true, |st: &mut State| st.browse_view.menu = None)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.browse_view.menu = None;
                    true
                } else {
                    false
                }
            })
            .child(items),
    )
}

/// The card's right-click menu, anchored to the card itself so it never leaves the window.
pub(crate) fn context_menu(s: &State, u: &Ui) -> Option<El> {
    let id = s.browse_view.context?;
    let card = card_by_id(s, id)?;
    let url = card.profile_url.clone();
    let image = card.thumbnail_url.clone();
    let name = card.name.clone();
    let author = card.author.clone();
    let is_mod = card.model.eq_ignore_ascii_case("mod");
    let install_name = name.clone();
    let mut items = col().w_full();
    items = items
        .child(
            menu_item(u, "ctx-open", None, "Open", "", false)
                .on_press(move |st: &mut State| open_details(st, id)),
        )
        .child(
            menu_item(
                u,
                "ctx-link",
                Some(icon(&icons::COPY, 14.0).color(u.muted).a11y_hidden()),
                "Copy link",
                "",
                false,
            )
            .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                tx.copy(url.clone());
                st.status = "Link copied.".to_string();
                st.browse_view.context = None;
            }),
        );
    if let Some(image) = image {
        items = items.child(
            menu_item(
                u,
                "ctx-image",
                Some(icon(&icons::COPY, 14.0).color(u.muted).a11y_hidden()),
                "Copy image link",
                "",
                false,
            )
            .on_press_tx({
                let image = image.clone();
                move |st: &mut State, tx: &mut Tx| {
                    tx.copy(image.clone());
                    st.status = "Image link copied.".to_string();
                    st.browse_view.context = None;
                }
            }),
        );
        // Wave 4: Grimoire's Copy image and Open image beside the address.
        items = items.child(
            menu_item(
                u,
                "ctx-image-more",
                Some(icon(&icons::EXTERNAL_LINK, 14.0).color(u.muted).a11y_hidden()),
                crate::screens::settings::i18n::tr("Image..."),
                "",
                false,
            )
            .on_press(move |st: &mut State| {
                st.browse_view.context = None;
                super::super::w4::open_image_menu(st, format!("browse-card-{id}"), image.clone());
            }),
        );
    }
    items = items.child(el().w_full().h(1.0).my(4.0).fill(u.hair));
    if is_mod {
        // The menu says what the card's button would do now.
        let state = super::super::quick::button(s, id);
        let label = match state {
            super::super::quick::Button::Installed => crate::screens::settings::i18n::tr("Open Installed").to_string(),
            other => other.label(),
        };
        items = items.child(
            menu_item(
                u,
                "ctx-install",
                Some(icon(&icons::DOWNLOAD, 14.0).color(u.muted).a11y_hidden()),
                &label,
                "",
                false,
            )
            .on_press(move |st: &mut State| {
                st.browse_view.context = None;
                super::super::quick::press(st, id, &install_name);
            }),
        );
    }
    items = items.child(
        menu_item(
            u,
            "ctx-hide",
            Some(icon(&EYE_OFF, 14.0).color(u.muted).a11y_hidden()),
            &if name.is_empty() {
                "Hide this mod".to_string()
            } else {
                format!("Hide {name}")
            },
            "",
            false,
        )
        .on_press(move |st: &mut State| hide_mod(st, id)),
    );
    if !author.is_empty() {
        items = items.child(
            menu_item(
                u,
                "ctx-hide-author",
                Some(icon(&EYE_OFF, 14.0).color(u.muted).a11y_hidden()),
                &format!("Hide {author}"),
                "Every mod by this creator",
                false,
            )
            .on_press(move |st: &mut State| hide_author(st, author.clone())),
        );
    }
    Some(
        col()
            .key("browse-context")
            .role("menu")
            .w(240.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
            .anchored(format!("browse-card-{id}"), Side::Bottom, Align::Start, 6.0)
            .modal(true, |st: &mut State| st.browse_view.context = None)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.browse_view.context = None;
                    true
                } else {
                    false
                }
            })
            .child(items),
    )
}

/// The 120 point preview volume control. The view menu and the status bar both draw it; they
/// move the one volume every preview plays at.
pub(crate) fn audio_slider(s: &State, u: &Ui, key: &str) -> El {
    let width = 120.0;
    let frac = s.browse_view.audio.volume.clamp(0.0, 1.0);
    let knob = 14.0;
    let track_y = 10.0;
    el().key(key.to_string())
        .w(width)
        .h(24.0)
        .focusable(true)
        .role("slider")
        .label("Preview volume")
        .cursor(rune::Cursor::Grab)
        .child(
            el().absolute(0.0, track_y)
                .w(width)
                .h(4.0)
                .radius(2.0)
                .fill(u.over.alpha(0.10))
                .a11y_hidden(),
        )
        .child(
            el().absolute(0.0, track_y)
                .w((width * frac).max(0.0))
                .h(4.0)
                .radius(2.0)
                .fill(u.muted)
                .a11y_hidden(),
        )
        .child(
            el().absolute((width - knob) * frac, (24.0 - knob) / 2.0)
                .size(knob, knob)
                .radius(knob / 2.0)
                .fill(Color::WHITE)
                .shadow(1.0, 3.0, Color::BLACK.alpha(0.4))
                .a11y_hidden(),
        )
        .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
            if d.phase == Phase::Up && d.cancelled() {
                return;
            }
            let width = if d.w > 0.0 { d.w } else { width };
            st.browse_view.audio.set_volume((d.x / width).clamp(0.0, 1.0));
            if d.phase == Phase::Up {
                save_browse_state(st);
            }
        })
}

/// The category list has not arrived: loading, or a retry after a failed read.
pub(crate) fn categories_pending(s: &State, u: &Ui, section: BrowseSection) -> El {
    if s.browse_view.categories_failed.contains(&section) {
        let lead = icon(&icons::REFRESH_CW, 14.0).color(u.muted).a11y_hidden();
        return menu_item(u, "menu-cat-retry", Some(lead), "Categories did not load", "Retry", false)
            .on_press(move |st: &mut State| {
                st.browse_view.categories_failed.retain(|s| *s != section);
                start_categories(st, section);
            });
    }
    row()
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
        .child(text("Loading categories").font(SMALL).color(u.faint))
}

pub(crate) fn menu_label(u: &Ui, label: &str) -> El {
    row()
        .w_full()
        .h(28.0)
        .px(10.0)
        .center_y()
        .child(text(label.to_string()).font(SMALL).color(u.faint))
}

/// One 32 point menu row: an optional icon, the label, a quiet detail and a check when it
/// is the current choice.
pub(crate) fn menu_item(
    u: &Ui,
    key: &str,
    lead: Option<El>,
    label: &str,
    detail: &str,
    on: bool,
) -> El {
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(u.over.alpha(0.08))
        .focusable(true)
        .role("menuitemradio")
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer);
    if let Some(lead) = lead {
        r = r.child(lead);
    }
    r = r.child(
        text(label.to_string())
            .font(BODY)
            .color(if on { u.ink } else { u.prose })
            .truncate()
            .min_w(0.0)
            .shrink()
            .a11y_hidden(),
    );
    r = r.child(spacer());
    if !detail.is_empty() {
        r = r.child(text(detail.to_string()).font(SMALL).color(u.faint).a11y_hidden());
    }
    if on {
        r = r.child(icon(&icons::CHECK, 14.0).color(u.ink).a11y_hidden());
    } else {
        r = r.child(el().w(14.0));
    }
    r
}

/// The grid as virtual rows of `cols` cards plus one row for the end of the catalog. Only
/// the rows near the viewport are built.
pub(crate) fn virtual_grid(s: &State, shown: Rc<Vec<usize>>, cols: usize, card_w: f32) -> El {
    let rows = shown.len().div_ceil(cols);
    // Numbered pages end with the page itself; the long feed ends with a tail row.
    let tail = usize::from(!s.browse_view.paged);
    let estimate = card_w * 9.0 / 16.0 + CARD_TEXT_H + 2.0 + GRID_GAP;
    let keys = shown.clone();
    let build = shown.clone();
    let watch = shown.clone();
    let version = (
        cols,
        shown.len(),
        s.browse.page.as_ref().map(|page| page.page).unwrap_or(0),
        s.browse_query.text.clone(),
        s.browse_view.hidden_mods.len(),
        s.browse_view.hidden_authors.len(),
        s.browse_view.card_size,
        s.browse_view.paged,
    );
    col()
        .key("browse-rows")
        .w_full()
        .growth(GrowthSpec {
            spring: rune::GROWTH,
            edge: rune::edge::NO_MASK,
        })
        .virtual_rows(
            rows + tail,
            estimate,
            move |s: &State, i| -> Cow<'_, str> {
                if i >= rows {
                    return Cow::Borrowed("browse-tail");
                }
                let first = keys
                    .get(i * cols)
                    .and_then(|&index| s.browse.page.as_ref()?.mods.get(index))
                    .map(|card| card.id)
                    .unwrap_or(0);
                Cow::Owned(format!("browse-row-{cols}-{first}"))
            },
            move |s: &State, i| {
                let u = Ui::new();
                if i >= rows {
                    return tail_row(s, &u);
                }
                let mut grid = el().w_full().equal_columns(cols).gap(GRID_GAP).pb(GRID_GAP);
                if let Some(page) = s.browse.page.as_ref() {
                    for &index in build.iter().skip(i * cols).take(cols) {
                        if let Some(card) = page.mods.get(index) {
                            grid = grid.child(card_el(s, &u, card, card_w));
                        }
                    }
                }
                grid
            },
        )
        .virtual_rows_version(version)
        .on_visible_rows(move |st: &mut State, range: Range<usize>| {
            track_visible(st, range, &watch, cols)
        })
}

/// The list layout: one row per mod, hairline separated, with the same facts as a card.
pub(crate) fn list_view(s: &State, _u: &Ui, shown: Rc<Vec<usize>>) -> El {
    let count = shown.len();
    let keys = shown.clone();
    let build = shown.clone();
    let watch = shown.clone();
    let version = (
        count,
        s.browse.page.as_ref().map(|page| page.page).unwrap_or(0),
        s.browse_query.text.clone(),
        s.browse_view.hidden_mods.len(),
        s.browse_view.hidden_authors.len(),
        s.browse_view.card_size,
    );
    col()
        .key("browse-list")
        .w_full()
        .growth(GrowthSpec {
            spring: rune::GROWTH,
            edge: rune::edge::NO_MASK,
        })
        .virtual_rows(
            count,
            77.0,
            move |s: &State, i| -> Cow<'_, str> {
                let id = keys
                    .get(i)
                    .and_then(|&index| s.browse.page.as_ref()?.mods.get(index))
                    .map(|card| card.id)
                    .unwrap_or(0);
                Cow::Owned(format!("browse-list-row-{id}"))
            },
            move |s: &State, i| {
                let u = Ui::new();
                let Some(card) = build
                    .get(i)
                    .and_then(|&index| s.browse.page.as_ref()?.mods.get(index))
                else {
                    return el();
                };
                list_row(s, &u, card)
            },
        )
        .virtual_rows_version(version)
        .on_visible_rows(move |st: &mut State, range: Range<usize>| {
            // The list has no columns; one row is one card.
            track_visible(st, range, &watch, 1)
        })
}

pub(crate) fn list_row(s: &State, u: &Ui, card: &BrowseCard) -> El {
    let id = card.id;
    let veil = veiled(s, id, card.nsfw);
    let age = gamebanana::relative_age(card.updated_ts, gamebanana::now_unix());
    let is_mod = card.model.eq_ignore_ascii_case("mod");
    let playing = s.browse_view.audio.playing == Some(id);
    let install = if is_mod {
        // The row reads its own mod's state, so one install never changes another row.
        let state = super::super::quick::button(s, id);
        super::super::quick::button_el(
            u,
            &state,
            format!("list-install-{id}"),
            id,
            &card.name,
            super::super::quick::Size::Row,
        )
    } else {
        super::super::quick::button_el(
            u,
            &super::super::quick::Button::Install,
            format!("list-install-{id}"),
            id,
            &card.name,
            super::super::quick::Size::Row,
        )
        .on_press(move |st: &mut State| open_details(st, id))
    };
    let status = is_mod.then(|| super::super::quick::status_line(s, u, id)).flatten();

    let saved = is_favorite(s, &card.model, id);
    let favorite = Favorite::from_card(card);
    let star = u
        .icon_button(if saved { &STAR_FILLED } else { &STAR }, if saved { "Remove from favorites" } else { "Save to favorites" })
        .key(format!("list-star-{id}"))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| toggle_favorite(st, favorite.clone()));

    let mut stats = row().gap(12.0).center_y().child(stat(u, &icons::THUMBS_UP, card.likes));
    stats = stats.child(stat(u, &icons::EYE, card.views));
    if let Some(downloads) = s.browse_view.download_counts.get(&id) {
        stats = stats.child(stat(u, &icons::DOWNLOAD, *downloads));
    }

    let audio = card.audio_url.clone().map(|url| {
        el().key(format!("list-sample-{id}"))
            .size(32.0, 32.0)
            .center()
            .radius(crate::ui::r(8.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(if playing { "Stop the sample" } else { "Play the sample" })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.browse_view.audio.toggle(id, url.clone()))
            .child(
                icon(if playing { &icons::SQUARE_STOP } else { &icons::PLAY }, 14.0)
                    .color(if playing { u.accent } else { u.muted })
                    .a11y_hidden(),
            )
    });

    let mut line = row()
        .key(format!("browse-card-{id}"))
        .w_full()
        .h(76.0)
        .px(8.0)
        .gap(14.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(format!("{} by {}", card.name, card.author))
        .cursor(rune::Cursor::Pointer)
        .on_hover_change(move |st: &mut State, over: bool, _tx: &mut Tx| {
            if over {
                st.browse_view.hovered = Some(id);
                st.browse_view.hover_since = Some((id, Instant::now()));
            } else if st.browse_view.hovered == Some(id) {
                st.browse_view.hovered = None;
                st.browse_view.hover_since = None;
            }
        })
        .on_secondary_press(move |st: &mut State, _press, _tx: &mut Tx| {
            st.browse_view.context = Some(id);
            st.browse_view.menu = None;
        })
        .on_press(move |st: &mut State| open_details(st, id))
        .child(
            el().w(112.0)
                .h(63.0)
                .radius(crate::ui::r(6.0))
                .clip_rounded(crate::ui::r(6.0))
                .child(image_fill(
                    s,
                    format!("list-thumb-{id}"),
                    card.thumbnail_url.as_ref(),
                    [6.0; 4],
                    veil,
                )),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(text(card.name.clone()).font(BODY_MED).color(u.ink).truncate())
                .child(
                    text(format!(
                        "{} · {}{}",
                        if card.author.is_empty() {
                            "Unknown author".to_string()
                        } else {
                            format!("by {}", card.author)
                        },
                        card.category,
                        if age.is_empty() {
                            String::new()
                        } else {
                            format!(" · {age}")
                        },
                    ))
                    .font(SMALL)
                    .color(if outdated(card) { u.amber } else { u.muted })
                    .truncate(),
                )
                .child(status.unwrap_or(stats)),
        )
        .child(spacer());
    if let Some(audio) = audio {
        line = line.child(audio);
    }
    line.child(star).child(install)
}

/// The last row: a loading line while the next page reads, a retry after a failed read, or
/// nothing at the end of the catalog.
pub(crate) fn tail_row(s: &State, u: &Ui) -> El {
    let base = row()
        .key("browse-tail-row")
        .w_full()
        .h(56.0)
        .gap(8.0)
        .center();
    if s.browse.loading {
        return base
            .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
            .child(text("Loading more mods").font(SMALL).color(u.faint));
    }
    if s.browse.error.is_some() && !s.browse_view.refreshing {
        return base
            .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
            .child(text("The next page did not load.").font(SMALL).color(u.muted))
            .child(
                u.action("Try again", ActionKind::Ghost)
                    .key("browse-more-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(load_next_page),
            );
    }
    if more_to_load(s) && s.browse_view.auto_streak >= AUTO_EMPTY_PAGES {
        return base.child(
            u.action("Load more", ActionKind::Secondary)
                .key("browse-more")
                .cursor(rune::Cursor::Pointer)
                .on_press(load_more_pressed),
        );
    }
    if !more_to_load(s) {
        let f = &s.browse_view.filters;
        let end = if f.favorites || f.owned == Owned::Installed {
            ""
        } else if f.active() > 0 || !s.browse_view.applied_search.is_empty() {
            "That is every match."
        } else {
            "That is the whole catalog."
        };
        return base.child(text(end).font(SMALL).color(u.faint));
    }
    base
}

pub(crate) fn page_error(u: &Ui) -> El {
    row()
        .key("browse-error")
        .w_full()
        .h(40.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
        .child(
            text("GameBanana did not answer. The mods below are from the last read.")
                .font(BODY)
                .color(u.muted)
                .grow(1.0),
        )
        .child(
            u.action("Try again", ActionKind::Ghost)
                .key("browse-error-retry")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(refresh),
        )
}

