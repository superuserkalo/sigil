//! The Collections tab: GameBanana collections for Deadlock, their items, and the queue that
//! installs a whole collection one mod at a time.

use super::*;

/// How many collections a page asks for. GameBanana serves 15 a page whatever is asked.
const COLLECTIONS_PER_PAGE: usize = 15;

/// Which collections the grid keeps, by how many mods they hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bucket {
    #[default]
    Any,
    Small,
    Medium,
    Large,
}

impl Bucket {
    const ALL: [Bucket; 4] = [Bucket::Any, Bucket::Small, Bucket::Medium, Bucket::Large];

    fn label(self) -> &'static str {
        match self {
            Bucket::Any => "All",
            Bucket::Small => "Under 10",
            Bucket::Medium => "10 to 30",
            Bucket::Large => "Over 30",
        }
    }

    fn keeps(self, count: u64) -> bool {
        match self {
            Bucket::Any => true,
            Bucket::Small => count < 10,
            Bucket::Medium => (10..=30).contains(&count),
            Bucket::Large => count > 30,
        }
    }
}

/// Start a collections page read.
pub(crate) fn start_collections(st: &mut State, page: usize) {
    if st.demo || rune::is_headless() || st.browse_view.collections_job.is_running() {
        return;
    }
    let wake = st.wake.clone();
    st.browse_view.collections_job.start(wake, move || {
        gamebanana::fetch_collections(page, COLLECTIONS_PER_PAGE)
            .map(|read| CollectionLoad {
                page: read.page,
                total: read.total,
                complete: read.complete,
                items: read.items,
                error: None,
            })
            .map_err(|error| error.to_string())
    });
}

/// Read the next page of collections and append it.
pub(crate) fn load_more_collections(st: &mut State) {
    let next = st
        .browse_view
        .collections
        .as_ref()
        .map(|load| load.page + 1)
        .unwrap_or(1);
    start_collections(st, next);
}

/// Open one collection: the items arrive on a worker.
pub(crate) fn open_collection(st: &mut State, summary: CollectionSummary) {
    st.browse_view.collection = Some(CollectionOpen {
        summary,
        items: Vec::new(),
        page: 0,
        complete: false,
        selected: HashSet::new(),
        job: Job::default(),
        error: None,
    });
    start_collection_items(st, 1);
}

pub(crate) fn start_collection_items(st: &mut State, page: usize) {
    let Some(collection) = st.browse_view.collection.as_ref() else {
        return;
    };
    if collection.job.is_running() {
        return;
    }
    let collection_id = collection.summary.id;
    let wake = st.wake.clone();
    if let Some(collection) = st.browse_view.collection.as_mut() {
        collection.job.start(wake, move || {
            Ok(CollectionItemsLoad {
                collection_id,
                page,
                result: gamebanana::fetch_collection_items(collection_id, page, 50)
                    .map_err(|error| error.to_string()),
            })
        });
    }
}

/// Apply finished collection item reads. True when the view must run again.
pub(crate) fn poll_collection(st: &mut State) -> bool {
    let Some(result) = st
        .browse_view
        .collection
        .as_mut()
        .and_then(|collection| collection.job.try_take())
    else {
        return false;
    };
    let Ok(load) = result else {
        if let Some(collection) = st.browse_view.collection.as_mut() {
            collection.error = Some("The collection could not be loaded.".to_string());
        }
        return true;
    };
    let Some(collection) = st.browse_view.collection.as_mut() else {
        return true;
    };
    if collection.summary.id != load.collection_id {
        return true;
    }
    match load.result {
        Ok(read) => {
            let before: HashSet<u64> = collection.items.iter().map(|card| card.id).collect();
            for row in &read.mods {
                if !before.contains(&row.id) {
                    collection.items.push(card_from_browse(row));
                }
            }
            collection.page = load.page;
            collection.complete = read.complete;
            collection.error = None;
        }
        Err(error) => collection.error = Some(error),
    }
    let urls: Vec<String> = st
        .browse_view
        .collection
        .as_ref()
        .map(|collection| {
            collection
                .items
                .iter()
                .filter_map(|card| card.thumbnail_url.clone())
                .collect()
        })
        .unwrap_or_default();
    crate::data::queue_images(st, urls, false);
    true
}

/// Queue the ticked mods, or every mod when none is ticked, and start the first install.
pub(crate) fn queue_collection(st: &mut State, all: bool) {
    let Some(collection) = st.browse_view.collection.as_ref() else {
        return;
    };
    let name = collection.summary.name.clone();
    let mut queue: Vec<(u64, String)> = collection
        .items
        .iter()
        .filter(|card| card.model.eq_ignore_ascii_case("mod"))
        .filter(|card| all || collection.selected.is_empty() || collection.selected.contains(&card.id))
        .map(|card| (card.id, card.name.clone()))
        .collect();
    queue.dedup_by_key(|(id, _)| *id);
    if queue.is_empty() {
        st.status = "This collection has no mods to install.".to_string();
        return;
    }
    st.browse_view.collection = None;
    // Each mod goes through the download queue on its own card; they install one at a time.
    let total = queue.len();
    for (id, mod_name) in queue {
        if matches!(
            super::quick::button(st, id),
            super::quick::Button::Install | super::quick::Button::Retry(_)
        ) {
            st.browse_view.installs.items.remove(&id);
            super::quick::start(st, id, &mod_name, id.to_string(), None, Vec::new());
        }
    }
    st.status = format!("Installing {} from {name}.", plural(total, "mod", "mods"));
}

/// The Collections tab body: bucket chips and the collection cards.
pub(crate) fn collections_page(s: &State, u: &Ui) -> El {
    let load = s.browse_view.collections.as_ref();
    if load.is_none() && !s.browse_view.collections_job.is_running() {
        // A scene can mount the tab without a read; the poll starts one for a live run.
        return u.empty_state(
            "",
            "Collections",
            "Lists other players put together.",
            None,
        );
    }
    let Some(load) = load else {
        return u.empty_state("", "Loading collections", "", None);
    };
    if let Some(error) = &load.error {
        return u.empty_state(
            "",
            "Collections did not load",
            error,
            Some(
                u.action("Try again", ActionKind::Secondary)
                    .key("collections-retry")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| start_collections(st, 1)),
            ),
        );
    }
    let bucket = s.browse_view.collection_bucket;
    let shown: Vec<&CollectionSummary> = load
        .items
        .iter()
        .filter(|item| bucket.keeps(item.item_count))
        .collect();
    let mut c = col().w_full().gap(16.0).pb(16.0);
    let mut chips = row().w_full().gap(8.0).center_y().flex_wrap();
    for choice in Bucket::ALL {
        let on = choice == bucket;
        chips = chips.child(
            row()
                .key(format!("collections-bucket-{}", choice.label()))
                .h(32.0)
                .px(12.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(if on { u.over.alpha(0.06) } else { u.over.alpha(0.025) })
                .border(1.0, if on { u.over.alpha(0.18) } else { u.hair_strong })
                .hover(u.hover)
                .focusable(true)
                .role("button")
                .label(choice.label())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.browse_view.collection_bucket = choice)
                .child(text(choice.label()).font(LABEL).color(if on { u.ink } else { u.muted })),
        );
    }
    c = c.child(chips);
    if shown.is_empty() {
        return c.child(u.empty_state(
            "",
            "No collections here",
            "Try another size.",
            None,
        ));
    }
    let mut grid = el().w_full().equal_columns(3).gap(GRID_GAP);
    for collection in shown {
        let summary = (*collection).clone();
        let chosen = summary.clone();
        let image = summary.image_url.clone();
        let count = summary.item_count;
        grid = grid.child(
            col()
                .key(format!("collection-{}", summary.id))
                .min_w(0.0)
                .radius(CARD_RADIUS)
                .fill(u.well)
                .border(1.0, Color::WHITE.alpha(0.07))
                .hover(Color::WHITE.alpha(0.025))
                .clip_rounded(CARD_RADIUS)
                .focusable(true)
                .role("button")
                .label(format!("{} by {}", summary.name, summary.author))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| open_collection(st, chosen.clone()))
                .child(
                    el().w_full()
                        .aspect_ratio(16.0 / 7.0)
                        .clip_corners([CARD_RADIUS - 1.0, CARD_RADIUS - 1.0, 0.0, 0.0])
                        .child(image_fill(
                            s,
                            format!("collection-thumb-{}", summary.id),
                            image.as_ref(),
                            [CARD_RADIUS - 1.0, CARD_RADIUS - 1.0, 0.0, 0.0],
                            false,
                        )),
                )
                .child(
                    col()
                        .w_full()
                        .px(14.0)
                        .pt(12.0)
                        .pb(14.0)
                        .gap(4.0)
                        .child(text(summary.name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(
                            row()
                                .w_full()
                                .gap(8.0)
                                .center_y()
                                .child(
                                    text(if summary.author.is_empty() {
                                        "Unknown author".to_string()
                                    } else {
                                        format!("by {}", summary.author)
                                    })
                                    .font(SMALL)
                                    .color(u.muted)
                                    .truncate()
                                    .min_w(0.0)
                                    .shrink(),
                                )
                                .child(spacer())
                                .child(
                                    text(plural(count as usize, "mod", "mods"))
                                        .font(SMALL)
                                        .color(u.faint),
                                ),
                        ),
                ),
        );
    }
    c = c.child(grid);
    if !load.complete {
        c = c.child(
            row().w_full().center().pt(8.0).child(
                u.action("Load more", ActionKind::Secondary)
                    .key("collections-more")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(load_more_collections),
            ),
        );
    }
    c
}

/// The open collection over the grid.
pub(crate) fn collection_dialog(s: &State, u: &Ui) -> Option<El> {
    let collection = s.browse_view.collection.as_ref()?;
    let summary = &collection.summary;
    let total = summary.item_count;
    let close = |st: &mut State| st.browse_view.collection = None;
    let url = summary.profile_url.clone();
    let count = collection
        .items
        .iter()
        .filter(|card| card.model.eq_ignore_ascii_case("mod"))
        .count();
    let install_all = collection.selected.is_empty();

    let mut rows = col().w_full();
    if collection.items.is_empty() && collection.error.is_none() {
        rows = rows.child(
            row()
                .w_full()
                .h(80.0)
                .center()
                .gap(8.0)
                .child(icon(&icons::LOADER, 16.0).color(u.faint).a11y_hidden())
                .child(text("Loading the list").font(BODY).color(u.faint)),
        );
    }
    if let Some(error) = &collection.error {
        rows = rows.child(
            text(error.clone())
                .font(BODY)
                .color(u.amber)
                .wrap()
                .padding(12.0),
        );
    }
    let last = collection.items.len().saturating_sub(1);
    for (index, card) in collection.items.iter().enumerate() {
        let is_mod = card.model.eq_ignore_ascii_case("mod");
        let ticked = collection.selected.contains(&card.id);
        let id = card.id;
        let name = card.name.clone();
        let thumb = card.thumbnail_url.clone();
        let mut row = row()
            .key(format!("collection-item-{id}"))
            .w_full()
            .min_h(56.0)
            .px(12.0)
            .py(8.0)
            .gap(12.0)
            .center_y()
            .hover(u.hover);
        if is_mod {
            row = row
                .focusable(true)
                .role("checkbox")
                .checked(ticked)
                .label(format!("Choose {name}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    if let Some(collection) = st.browse_view.collection.as_mut()
                        && !collection.selected.remove(&id)
                    {
                        collection.selected.insert(id);
                    }
                });
        }
        row = row
            .child(
                el().size(20.0, 20.0)
                    .radius(4.0)
                    .center()
                    .border(1.0, if ticked { u.accent.alpha(0.7) } else { u.hair_strong })
                    .fill(if ticked { u.accent.alpha(0.18) } else { Color::TRANSPARENT })
                    .child(if ticked {
                        icon(&icons::CHECK, 12.0).color(u.accent).a11y_hidden()
                    } else {
                        el()
                    }),
            )
            .child(
                el().size(56.0, 36.0)
                    .radius(crate::ui::r(6.0))
                    .clip_rounded(crate::ui::r(6.0))
                    .child(image_fill(
                        s,
                        format!("collection-item-thumb-{id}"),
                        thumb.as_ref(),
                        [6.0; 4],
                        veiled(s, id, card.nsfw),
                    )),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(card.name.clone()).font(BODY).color(u.ink).truncate())
                    .child(
                        text(if card.author.is_empty() {
                            card.category.clone()
                        } else {
                            format!("by {} · {}", card.author, card.category)
                        })
                        .font(SMALL)
                        .color(u.muted)
                        .truncate(),
                    ),
            );
        if !is_mod {
            row = row.child(text(card.model.clone()).font(SMALL).color(u.faint));
        } else {
            // The mod's own install state, as on its card.
            let state = super::quick::button(s, id);
            row = row.child(super::quick::button_el(
                u,
                &state,
                format!("collection-install-{id}"),
                id,
                &name,
                super::quick::Size::Row,
            ));
        }
        rows = rows.child(row);
        if index < last {
            rows = rows.child(el().w_full().h(1.0).fill(u.hair));
        }
    }
    if !collection.complete {
        let next = collection.page + 1;
        rows = rows.child(
            row().w_full().h(48.0).center().child(
                u.action("Load more", ActionKind::Ghost)
                    .key("collection-more")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| start_collection_items(st, next)),
            ),
        );
    }

    Some(
        u.dialog_shell()
            .key("collection-dialog")
            .role("dialog")
            .label(summary.name.clone())
            .w(720.0)
            .max_w(720.0)
            .max_h(760.0)
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
                    .gap(12.0)
                    .items_start()
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(text(summary.name.clone()).font(H1).color(u.ink).truncate())
                            .child(
                                text(format!(
                                    "{} · {}",
                                    if summary.author.is_empty() {
                                        "Unknown author".to_string()
                                    } else {
                                        format!("by {}", summary.author)
                                    },
                                    plural(total as usize, "mod", "mods")
                                ))
                                .font(SMALL)
                                .color(u.muted),
                            ),
                    )
                    .child(
                        u.icon_button(&icons::EXTERNAL_LINK, "Open on GameBanana")
                            .key("collection-open-gb")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &url)),
                    )
                    .child(
                        u.icon_button(&icons::X, "Close")
                            .key("collection-close")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    ),
            )
            .child(
                col()
                    .w_full()
                    .max_h(480.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(rows),
            )
            .child({
                let mut actions = row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(if collection.selected.is_empty() {
                            "Every mod in the list will be installed.".to_string()
                        } else {
                            format!(
                                "{} chosen.",
                                plural(collection.selected.len(), "mod", "mods")
                            )
                        })
                        .font(SMALL)
                        .color(u.muted),
                    )
                    .child(spacer());
                actions = actions.child(
                        u.action("Close", ActionKind::Ghost)
                            .key("collection-cancel")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(close),
                    );
                if !collection.selected.is_empty() {
                    actions = actions.child(
                        u.action("Install all", ActionKind::Secondary)
                            .key("collection-install-all")
                            .disabled(count == 0)
                            .opacity(if count == 0 { 0.45 } else { 1.0 })
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| queue_collection(st, true)),
                    );
                }
                actions = actions.child(
                    u.icon_action(
                        &icons::DOWNLOAD,
                        if collection.selected.is_empty() {
                            "Install all"
                        } else {
                            "Install chosen"
                        },
                        ActionKind::Primary,
                    )
                    .key("collection-install")
                    .disabled(count == 0)
                    .opacity(if count == 0 { 0.45 } else { 1.0 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        queue_collection(st, install_all)
                    }),
                );
                actions
            }),
    )
}
