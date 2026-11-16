//! Browse: the public GameBanana Deadlock feed, read only.
//!
//! The page is fetched on a worker and thumbnails arrive one by one through
//! Rune's image cache. There is no Install action until download, ZIP
//! extraction and slot rules exist in core; the one action opens the real
//! submission page in the browser.

use rune::{Color, El, Key, KeyEvent, ScrollSpec, SurfaceFit, TextEdit, Tx, col, el, row, spacer, text};

use crate::app::State;
use crate::model::{BrowseCard, Screen};
use crate::ui::{ActionKind, BODY, BODY_MED, H2, LABEL, MONO, SMALL, Tone, Ui};

pub fn view(s: &State, u: &Ui) -> El {
    let trailing = row()
        .gap(7.0)
        .center_y()
        .child(page_button(s, u, "‹", false))
        .child(
            text(
                s.browse
                    .page
                    .as_ref()
                    .map(|page| page.page_label())
                    .unwrap_or_else(|| "no page".to_string()),
            )
            .font(SMALL)
            .color(u.muted),
        )
        .child(page_button(s, u, "›", true))
        .child(
            u.pill("Refresh", "↻", ActionKind::Ghost)
                .key("browse-refresh")
                .on_press(|st: &mut State| refresh(st)),
        );

    let query = s.browse_query.text.trim().to_lowercase();
    let filtered: Vec<&BrowseCard> = s
        .browse
        .page
        .as_ref()
        .map(|page| {
            page.mods
                .iter()
                .filter(|card| matches_query(card, &query))
                .collect()
        })
        .unwrap_or_default();

    let detail = match &s.browse.page {
        Some(page) => {
            let records = if page.complete {
                format!("{} of {} feed records", page.mods.len(), page.total)
            } else {
                format!("{} mods on this page of {}", page.mods.len(), page.total)
            };
            if query.is_empty() {
                format!("{records} · read only")
            } else {
                format!("{} of {} mods match \"{}\"", filtered.len(), page.mods.len(), s.browse_query.text.trim())
            }
        }
        None if s.browse.loading => "Reading the Deadlock subfeed in the background.".to_string(),
        None => "The public Deadlock subfeed from GameBanana. Read only.".to_string(),
    };

    let mut page = col()
        .key("browse-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "browse-glow",
            [0.12, 0.0],
            (760.0, 460.0),
            u.glow_cool,
        ))
        .child(u.page_head(
            Screen::Browse.kicker(),
            "Browse",
            &detail,
            Some(trailing),
        ))
        .child(status_strip(s, u));

    if let Some(error) = &s.browse.error {
        page = page.child(error_card(u, error));
    }

    let body = if s.browse.page.is_none() && s.browse.loading {
        loading_grid(u)
    } else if s.browse.page.is_none() {
        u.empty_state(
            "⊕",
            "No catalog page",
            s.browse
                .error
                .as_deref()
                .unwrap_or("The feed has not been read yet. Refresh to fetch page one."),
            Some(
                u.action("Read GameBanana", ActionKind::Primary)
                    .on_press(|st: &mut State| refresh(st)),
            ),
        )
    } else if filtered.is_empty() {
        u.empty_state(
            "⌕",
            "Nothing on this page matches",
            "Search filters the loaded page, not the whole feed. Clear it or move to another page.",
            Some(
                u.action("Clear search", ActionKind::Ghost).on_press(|st: &mut State| {
                    st.browse_query = rune::TextEdit::new("");
                }),
            ),
        )
    } else {
        let mut grid = col().w_full().gap(12.0);
        for chunk in filtered.chunks(4) {
            let mut r = row().w_full().gap(12.0).items_stretch();
            for card in chunk {
                r = r.child(card_el(s, u, card));
            }
            for _ in chunk.len()..4 {
                r = r.child(el().grow(1.0).min_w(0.0).h(1.0));
            }
            grid = grid.child(r);
        }
        col()
            .key("browse-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(grid)
    };

    page.child(body)
}

/// The install dialog is mounted at the root so it overlays the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if s.install.open {
        Some(install_dialog(s, u))
    } else {
        None
    }
}

fn matches_query(card: &BrowseCard, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    card.name.to_lowercase().contains(query)
        || card.author.to_lowercase().contains(query)
        || card.category.to_lowercase().contains(query)
}

fn refresh(st: &mut State) {
    if st.demo {
        st.status = "Demo fixture: the catalog stays on the fixture. Nothing was fetched."
            .to_string();
        return;
    }
    let page = st.browse.page.as_ref().map(|page| page.page).unwrap_or(1);
    crate::data::start_browse_fetch(st, page);
}

fn page_button(s: &State, u: &Ui, glyph: &str, next: bool) -> El {
    let enabled = if next {
        s.browse.can_next()
    } else {
        s.browse.can_prev()
    } && !s.browse.loading;
    let mut b = u
        .action(glyph, ActionKind::Secondary)
        .key(if next { "browse-next" } else { "browse-prev" })
        .label(if next { "Next page" } else { "Previous page" })
        .w(30.0)
        .px(0.0);
    if !enabled {
        b = b.opacity(0.42);
    }
    b.on_press(move |st: &mut State| {
        if st.browse.loading {
            return;
        }
        let Some(page) = st.browse.page.as_ref() else {
            return;
        };
        let target = if next {
            if page.page >= page.page_count {
                return;
            }
            page.page + 1
        } else {
            if page.page <= 1 {
                return;
            }
            page.page - 1
        };
        if st.demo {
            st.status =
                "Demo fixture: the catalog page stays on the fixture. Nothing was fetched."
                    .to_string();
        } else {
            crate::data::start_browse_fetch(st, target);
        }
    })
}

fn status_strip(s: &State, u: &Ui) -> El {
    let mut chips = row().gap(6.0).center_y();
    chips = chips.child(u.chip("GAMEBANANA · 20948", u.violet, u.violet.alpha(0.10)));
    if s.browse.loading {
        chips = chips.child(u.status_chip("reading feed", Tone::Warn));
    }
    if s.thumb_crew.is_loading() {
        chips = chips.child(u.status_chip("loading previews", Tone::Info));
    }
    if let Some(page) = &s.browse.page {
        chips = chips
            .child(u.chip(
                &format!("{} records", page.total),
                u.muted,
                Color::WHITE.alpha(0.04),
            ))
            .child(u.chip(
                &format!("stride {}", page.per_page),
                u.faint,
                Color::WHITE.alpha(0.035),
            ));
    }
    row()
        .key("browse-status")
        .w_full()
        .padding(11.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.022))
        .border(1.0, u.hair)
        .child(text("FEED").font(LABEL).color(u.faint))
        .child(
            text("Install downloads into the workspace, validates the payload and plans a slot. Nothing reaches the game until apply is approved.")
                .font(SMALL)
                .color(u.muted)
                .grow(1.0)
                .truncate(),
        )
        .child(chips)
}

fn card_el(s: &State, u: &Ui, card: &BrowseCard) -> El {
    let pixels = card
        .thumbnail_url
        .as_ref()
        .and_then(|url| s.thumbnails.get(url));
    let tint = tint_for(u, &card.category);
    let image = match pixels {
        Some(pixels) => el()
            .key(format!("thumb-{}", card.id))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .radius(12.0)
            .pixels_fit(pixels, SurfaceFit::Fill)
            .a11y_hidden(),
        None => {
            let label = if card.thumbnail_url.is_none() {
                "no preview"
            } else if s.thumb_crew.is_loading() {
                "loading preview"
            } else {
                "preview unavailable"
            };
            el().key(format!("thumb-missing-{}", card.id))
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .radius(12.0)
                .fill(tint)
                .center()
                .child(
                    col()
                        .gap(4.0)
                        .items_center()
                        .child(u.glyph_box("▦", Tone::Quiet, 26.0))
                        .child(text(label).font(LABEL).color(u.faint)),
                )
                .a11y_hidden()
        }
    };

    let category = if card.category.is_empty() {
        "MOD".to_string()
    } else {
        card.category.to_uppercase()
    };
    let overlay = row()
        .absolute(9.0, 9.0)
        .gap(5.0)
        .center_y()
        .child(u.chip(&category, Color::WHITE.alpha(0.88), Color::BLACK.alpha(0.58)));
    let overlay = match card.version_label() {
        Some(version) => overlay.child(u.chip(
            &version,
            Color::WHITE.alpha(0.75),
            Color::BLACK.alpha(0.45),
        )),
        None => overlay,
    };

    let url = card.profile_url.clone();
    let open_label = format!("Open page for {}", card.name);
    col()
        .key(format!("browse-card-{}", card.id))
        .grow(1.0)
        .min_w(0.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.028))
        .border(1.0, u.hair)
        .hover(Color::WHITE.alpha(0.042))
        .motion(rune::tok::motion::SNAPPY)
        .child(
            el().w_full()
                .aspect_ratio(1.55)
                .child(image)
                .child(overlay),
        )
        .child(
            col()
                .w_full()
                .grow(1.0)
                .padding(12.0)
                .gap(5.0)
                .child(
                    text(card.name.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .max_lines(2)
                        .min_h(40.0)
                        .wrap(),
                )
                .child(text(card.author_label()).font(SMALL).color(u.muted).truncate())
                .child(
                    text(card.meta_label())
                        .font(SMALL)
                        .color(u.faint)
                        .truncate(),
                )
                .child(
                    row()
                        .w_full()
                        .pt(5.0)
                        .gap(6.0)
                        .center_y()
                        .child(
                            text(format!("id {}", card.id))
                                .font(LABEL)
                                .color(u.faint)
                                .grow(1.0),
                        )
                        .child(
                            u.action("Install", ActionKind::Primary)
                                .key(format!("install-{}", card.id))
                                .label(&format!("Install {}", card.name))
                                .on_press({
                                    let id = card.id;
                                    let name = card.name.clone();
                                    move |st: &mut State| {
                                        st.install = InstallView::for_mod(id, &name);
                                    }
                                }),
                        )
                        .child(
                            u.action("Open page", ActionKind::Secondary)
                                .key(format!("open-{}", card.id))
                                .label(&open_label)
                                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                    match tx.open_url(&url) {
                                        Ok(()) => {
                                            st.status = format!(
                                                "Opened {url} in the browser. Nothing was downloaded."
                                            )
                                        }
                                        Err(error) => {
                                            st.status = format!(
                                                "The page could not be opened: {error}"
                                            )
                                        }
                                    }
                                }),
                        ),
                ),
        )
}

fn tint_for(u: &Ui, category: &str) -> Color {
    let hues = [u.violet, u.teal, u.amber, u.rose, u.accent, u.glow_cool];
    let seed = category.bytes().fold(0usize, |acc, byte| acc + byte as usize);
    hues[seed % hues.len()].alpha(0.10)
}

fn error_card(u: &Ui, error: &str) -> El {
    row()
        .w_full()
        .gap(11.0)
        .items_start()
        .padding(12.0)
        .radius(10.0)
        .fill(u.rose.alpha(0.05))
        .border(1.0, u.rose.alpha(0.20))
        .child(u.glyph_box("!", Tone::Bad, 28.0))
        .child(
            col()
                .grow(1.0)
                .gap(3.0)
                .child(
                    text("The GameBanana read failed")
                        .font(BODY_MED)
                        .color(u.ink),
                )
                .child(text(error.to_string()).font(SMALL).color(u.muted).wrap())
                .child(
                    text("The feed needs api.gamebanana.com. No key or account is used.")
                        .font(SMALL)
                        .color(u.faint)
                        .wrap(),
                ),
        )
}

fn loading_grid(u: &Ui) -> El {
    let mut grid = col().w_full().gap(12.0);
    for _ in 0..2 {
        let mut r = row().w_full().gap(12.0);
        for _ in 0..4 {
            r = r.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .radius(14.0)
                    .fill(Color::WHITE.alpha(0.022))
                    .border(1.0, u.hair)
                    .child(
                        el().w_full()
                            .aspect_ratio(1.55)
                            .radius(12.0)
                            .fill(Color::WHITE.alpha(0.02)),
                    )
                    .child(
                        col()
                            .padding(12.0)
                            .gap(7.0)
                            .child(text("reading feed").font(LABEL).color(u.faint))
                            .child(
                                text("GameBanana page one arrives on a worker.")
                                    .font(SMALL)
                                    .color(u.faint),
                            ),
                    ),
            );
        }
        grid = grid.child(r);
    }
    col()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(grid)
}

// --- install ---------------------------------------------------------------

#[derive(Clone)]
pub struct PlanSummary {
    pub id: String,
    pub source_label: String,
    pub slot: u8,
    pub target: String,
    pub files: usize,
    pub bytes: u64,
    pub conflicts: usize,
    pub replace_required: bool,
    pub replaced: Vec<String>,
    pub gameinfo_repair: bool,
    pub warnings: Vec<String>,
    pub unmapped: Vec<String>,
    pub artifact: String,
}

pub struct InstallOutcome {
    pub plan: Option<PlanSummary>,
    pub applied: Option<String>,
    pub error: Option<String>,
}

pub struct InstallView {
    pub open: bool,
    pub target: String,
    pub label: String,
    pub slot: TextEdit,
    pub force: bool,
    pub plan: Option<PlanSummary>,
    pub busy: bool,
    pub log: Option<String>,
    pub tone: Tone,
}

impl Default for InstallView {
    fn default() -> Self {
        InstallView {
            open: false,
            target: String::new(),
            label: String::new(),
            slot: TextEdit::new(""),
            force: false,
            plan: None,
            busy: false,
            log: None,
            tone: Tone::Quiet,
        }
    }
}

impl InstallView {
    pub fn for_mod(id: u64, name: &str) -> Self {
        InstallView {
            open: true,
            target: id.to_string(),
            label: name.to_string(),
            slot: TextEdit::new(""),
            force: false,
            plan: None,
            busy: false,
            log: None,
            tone: Tone::Quiet,
        }
    }

    pub fn slot_value(&self) -> Option<u8> {
        self.slot.text.trim().parse::<u8>().ok()
    }
}

fn install_dialog(s: &State, u: &Ui) -> El {
    let v = &s.install;
    let mut body = col()
        .w_full()
        .gap(14.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(5.0)
                        .child(u.eyebrow("GAMEBANANA · DOWNLOAD · VALIDATE · SLOT"))
                        .child(
                            text(format!("Install {}", v.label))
                                .font(H2)
                                .color(u.ink),
                        )
                        .child(
                            text("The archive is downloaded into the workspace, extracted with zip-slip checks, validated against the payload rules and planned for a free slot. Nothing reaches the game until the apply step is approved.")
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                        ),
                )
                .child(u.glyph_box("⊕", Tone::Accent, 38.0)),
        )
        .child(u.divider())
        .child(u.kv("Source", &v.target, u.muted));

    if let Some(plan) = &v.plan {
        body = body
            .child(u.kv("Plan", &plan.id, u.faint))
            .child(u.kv("Archive", &plan.source_label, u.muted))
            .child(u.kv(
                "Slot",
                &format!("{:02} -> {}", plan.slot, plan.target),
                u.ink,
            ))
            .child(u.kv("Artifact", &plan.artifact, u.muted))
            .child(u.kv(
                "Payload",
                &format!(
                    "{} file(s), {}",
                    plan.files,
                    crate::model::format_size(plan.bytes)
                ),
                u.ink,
            ))
            .child(u.kv(
                "Conflicts",
                &if plan.conflicts == 0 {
                    "none with a lower slot".to_string()
                } else {
                    format!("{} shared path(s), the lower slot wins", plan.conflicts)
                },
                if plan.conflicts == 0 { u.faint } else { u.amber },
            ))
            .child(u.kv(
                "Replace",
                &if plan.replace_required {
                    format!("{} existing file(s) would be backed up", plan.replaced.len())
                } else {
                    "not required, the slot is free".to_string()
                },
                if plan.replace_required { u.amber } else { u.faint },
            ));
        if !plan.warnings.is_empty() {
            for warning in &plan.warnings {
                body = body.child(
                    text(format!("WARNING {warning}"))
                        .font(SMALL)
                        .color(u.amber)
                        .wrap(),
                );
            }
        }
        if !plan.unmapped.is_empty() {
            body = body.child(
                text(format!(
                    "{} file(s) could not be mapped to a game path and are not installed.",
                    plan.unmapped.len()
                ))
                .font(SMALL)
                .color(u.faint)
                .wrap(),
            );
        }
        if plan.gameinfo_repair {
            body = body.child(
                text("gameinfo.gi does not list citadel/addons. The apply step repairs it and keeps a backup.")
                    .font(SMALL)
                    .color(u.amber)
                    .wrap(),
            );
        }
    } else {
        body = body
            .child(
                row()
                    .w_full()
                    .gap(10.0)
                    .items_start()
                    .child(
                        col()
                            .w(120.0)
                            .gap(4.0)
                            .child(u.field_label("SLOT"))
                            .child(
                                rune::text_input(&v.slot, BODY)
                                    .key("install-slot")
                                    .label("Addon slot")
                                    .single_line()
                                    .color(u.ink)
                                    .placeholder("auto", u.faint)
                                    .caret(u.accent, u.accent.alpha(0.20))
                                    .on_edit(|st: &mut State| &mut st.install.slot),
                            ),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .gap(4.0)
                            .child(u.field_label("SLOT STATE"))
                            .child(
                                text(if s.install.force {
                                    "an occupied slot may be replaced; its files are backed up first"
                                } else {
                                    "the lowest free slot is chosen"
                                })
                                .font(SMALL)
                                .color(u.muted)
                                .wrap(),
                            ),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .gap(9.0)
                    .center_y()
                    .child(u.toggle(v.force))
                    .child(
                        text("Allow replacing an occupied slot")
                            .font(SMALL)
                            .color(u.muted),
                    ),
            );
    }

    if let Some(log) = &v.log {
        body = body.child(
            col()
                .w_full()
                .padding(10.0)
                .radius(9.0)
                .fill(u.field)
                .border(1.0, v.tone.fg(u).alpha(0.22))
                .child(
                    text(log.clone())
                        .font(MONO)
                        .color(u.prose)
                        .wrap_anywhere()
                        .selectable(true),
                ),
        );
    }

    let plan_ready = !v.busy && !v.target.is_empty();
    let apply_ready = v.plan.is_some() && !v.busy && s.game_detected;
    let mut actions = row()
        .w_full()
        .gap(9.0)
        .child(
            u.action("Close", ActionKind::Ghost)
                .grow(1.0)
                .on_press(|st: &mut State| st.install.open = false),
        );
    if v.plan.is_none() {
        actions = actions.child(
            u.action("Plan install", ActionKind::Primary)
                .grow(1.0)
                .disabled(!plan_ready)
                .opacity(if plan_ready { 1.0 } else { 0.45 })
                .on_press(|st: &mut State| crate::data::start_install_plan(st)),
        );
    } else {
        actions = actions
            .child(
                u.action("Re-plan", ActionKind::Secondary)
                    .grow(1.0)
                    .disabled(!plan_ready)
                    .opacity(if plan_ready { 1.0 } else { 0.45 })
                    .on_press(|st: &mut State| {
                        st.install.plan = None;
                        crate::data::start_install_plan(st);
                    }),
            )
            .child(
                u.action("Apply with approval", ActionKind::Primary)
                    .grow(1.0)
                    .disabled(!apply_ready)
                    .opacity(if apply_ready { 1.0 } else { 0.45 })
                    .on_press(|st: &mut State| crate::data::start_install_apply(st)),
            );
    }
    body = body.child(actions);

    let close = |st: &mut State| st.install.open = false;
    u.dialog_shell()
        .key("install-dialog")
        .role("dialog")
        .label("Install a GameBanana mod")
        .w(620.0)
        .max_w(620.0)
        .padding(1.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(u.dialog_core().padding(20.0).child(body))
}
