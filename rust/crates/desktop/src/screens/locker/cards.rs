//! The custom card picker, the launch shuffle switches, and the applied changes review.
//!
//! All three are dialogs mounted at the root by `locker::dialog`. File picking and copying
//! go through `data`, which keeps the copy beside the app's other data and never touches the
//! game folder.

use rune::{
    Color, Drag, El, FileDialogOptions, Key, KeyEvent, Phase, Tx, col, row, spacer, text,
};

use crate::app::State;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, SMALL, Ui};

use super::{
    BriefRow, CardDialog, Dialog, HeroCard, close_dialog, cover_at, dialog_title, pixels_for,
    plural, shell,
};

/// Width of the crop slider's track. It sits in the dialog's right column.
const SLIDER_W: f32 = 200.0;

pub(super) fn open_card_dialog(st: &mut State, hero: &HeroCard) {
    let card = st.locker.prefs.card(&hero.display_name).cloned();
    let path = hero.custom.clone();
    let focus_x = card
        .as_ref()
        .map(|card| card.focus_x)
        .unwrap_or(hero.face_x);
    if let Some(path) = path.clone() {
        crate::data::request_locker_art(st, &path);
    }
    st.locker.dialog = Some(Dialog::Card(CardDialog {
        hero: hero.display_name.clone(),
        display_name: hero.display_name.clone(),
        path,
        size: hero.custom_size,
        focus_x,
        existing: card.is_some(),
        error: None,
    }));
}

fn choose_image(_st: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Choose a card image".into(),
        ..Default::default()
    }
    .filter("Images", &["png", "jpg", "jpeg", "webp"]);
    tx.open_file(options, |st: &mut State, result, _tx| {
        let Ok(Some(path)) = result else {
            return;
        };
        let size = card_size(&path);
        if let Some(Dialog::Card(dialog)) = st.locker.dialog.as_mut() {
            dialog.size = size;
            dialog.path = Some(path.display().to_string());
            dialog.error = None;
            if size.is_none() {
                dialog.error = Some("That file is not a PNG, JPEG or WebP image.".to_string());
            }
        }
        crate::data::request_locker_art(st, &path.display().to_string());
    });
}

/// Header pixels of a chosen image, for the preview ratio. The copy itself reads the
/// header again on the worker; this only keeps the preview the right shape.
fn card_size(path: &std::path::Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut header = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut header)
        .ok()?;
    deadlock_core::locker::art_dimensions(&header)
}

fn slider(u: &Ui, focus_x: f32, set: impl Fn(&mut State, f32) + 'static) -> El {
    let knob = 14.0;
    let track_y = 10.0;
    let frac = (focus_x / 100.0).clamp(0.0, 1.0);
    let set = std::rc::Rc::new(set);
    let drag_set = set.clone();
    let key_set = set;
    el_slider(u, track_y, knob, frac)
        .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
            if d.phase == Phase::Up && d.cancelled() {
                return;
            }
            let width = if d.w > 0.0 { d.w } else { SLIDER_W };
            drag_set(st, (d.x / width).clamp(0.0, 1.0) * 100.0);
        })
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            let step = match ev.key {
                Key::Left | Key::Down => -2.0,
                Key::Right | Key::Up => 2.0,
                _ => return false,
            };
            key_set(st, (focus_x + step).clamp(0.0, 100.0));
            true
        })
}

fn el_slider(u: &Ui, track_y: f32, knob: f32, frac: f32) -> El {
    let key = "locker-card-focus";
    rune::el()
        .key(key)
        .w(SLIDER_W)
        .h(24.0)
        .focusable(true)
        .role("slider")
        .label("Crop")
        .cursor(rune::Cursor::Grab)
        .child(
            rune::el()
                .absolute(0.0, track_y)
                .w(SLIDER_W)
                .h(4.0)
                .radius(crate::ui::r(2.0))
                .fill(u.over.alpha(0.10))
                .a11y_hidden(),
        )
        .child(
            rune::el()
                .absolute(0.0, track_y)
                .w((SLIDER_W * frac).max(0.0))
                .motion(crate::motion::fast())
                .h(4.0)
                .radius(crate::ui::r(2.0))
                .fill(u.muted)
                .a11y_hidden(),
        )
        .child(
            rune::el()
                .absolute((SLIDER_W - knob) * frac, (24.0 - knob) / 2.0)
                .size(knob, knob)
                .motion(crate::motion::fast())
                .radius(knob / 2.0)
                .fill(Color::WHITE)
                .shadow(1.0, 3.0, Color::BLACK.alpha(0.4))
                .a11y_hidden(),
        )
}

pub(super) fn card_dialog(s: &State, u: &Ui, dialog: &CardDialog) -> El {
    let preview = match dialog.path.as_deref().and_then(|path| pixels_for(s, Some(path))) {
        Some(pixels) => cover_at(&pixels, dialog.size, 200.0, 267.0, dialog.focus_x, 35.0),
        None => rune::col()
            .absolute(0.0, 0.0)
            .size(200.0, 267.0)
            .center()
            .child(
                text("Choose an image")
                    .font(SMALL)
                    .color(u.faint)
                    .a11y_hidden(),
            ),
    };
    let preview = rune::el()
        .w(200.0)
        .h(267.0)
        .radius(crate::ui::r(12.0))
        .clip_rounded(crate::ui::r(12.0))
        .fill(Color::WHITE.alpha(0.04))
        .child(preview)
        .a11y_hidden();

    let choose = u
        .action("Choose image", ActionKind::Secondary)
        .key("locker-card-choose")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(choose_image);

    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(
            u,
            format!("Card for {}", dialog.display_name),
            "locker-card-close",
        ))
        .child(
            text("Pick an image. The crop keeps the face in the middle of the card.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );

    if let Some(error) = &dialog.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }

    let mut buttons = row()
        .w_full()
        .gap(8.0)
        .child(spacer())
        .child(
            u.action("Cancel", ActionKind::Ghost)
                .key("locker-card-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(close_dialog),
        );
    if dialog.existing {
        let hero = dialog.hero.clone();
        buttons = buttons.child(
            u.action("Remove", ActionKind::Danger)
                .key("locker-card-remove")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    crate::data::remove_locker_card(st, &hero)
                }),
        );
    }
    let ready = dialog.path.is_some();
    buttons = buttons.child(
        u.action("Save", ActionKind::Primary)
            .key("locker-card-save")
            .disabled(!ready)
            .opacity(if ready { 1.0 } else { 0.45 })
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| crate::data::save_locker_card(st)),
    );

    body = body
        .child(
            row()
                .w_full()
                .gap(20.0)
                .items_start()
                .child(preview)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(12.0)
                        .child(choose)
                        .child(
                            text("Crop")
                                .font(BODY_MED)
                                .color(u.ink),
                        )
                        .child(slider(u, dialog.focus_x, |st, value| {
                            if let Some(Dialog::Card(dialog)) = st.locker.dialog.as_mut() {
                                dialog.focus_x = value.clamp(0.0, 100.0);
                            }
                        })),
                ),
        )
        .child(buttons);

    shell(
        u,
        "locker-card-dialog",
        format!("Card for {}", dialog.display_name),
        body,
    )
}

pub(super) fn shuffle_dialog(s: &State, u: &Ui) -> El {
    let v = &s.locker;
    let count = v
        .heroes
        .iter()
        .filter(|hero| v.prefs.shuffles(&hero.display_name))
        .count();
    let mut list = col().w_full();

    for hero in &v.heroes {
        let on = v.prefs.shuffles(&hero.display_name);
        let name = hero.display_name.clone();
        list = list.child(
            row()
                .key(format!("locker-shuffle-{}", hero.codename))
                .w_full()
                .h(44.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(hero.display_name.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    u.toggle(on)
                        .label(format!("Shuffle {} at launch", hero.display_name))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::data::toggle_locker_shuffle(st, &name)
                        }),
                ),
        );
        // C14: Grimoire's shuffle pool. With none picked every skin of the hero can come up;
        // picking some keeps the roll to those. The roll never repeats the skin on now.
        if on {
            for m in v.mods.iter().filter(|m| m.heroes.contains(&hero.display_name)) {
                let key = m.key.clone();
                let in_pool = v.prefs.in_pool(&m.key);
                list = list.child(
                    row()
                        .key(format!("locker-pool-{}", m.slot))
                        .w_full()
                        .h(34.0)
                        .pl(16.0)
                        .gap(10.0)
                        .center_y()
                        .role("checkbox")
                        .checked(in_pool)
                        .label(fill(tr("{} in the shuffle pool"), &[&m.label]))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            st.locker.prefs.toggle_pool(&key);
                            crate::data::save_locker_prefs(st);
                        })
                        .child(
                            rune::icon(&crate::icons::CIRCLE_CHECK, 14.0)
                                .color(if in_pool { u.accent } else { u.faint.alpha(0.5) })
                                .a11y_hidden(),
                        )
                        .child(text(m.label.clone()).font(BODY).color(u.prose).grow(1.0).min_w(0.0).truncate()),
                );
            }
        }
    }
    let multi = v.prefs.multi_skin;
    let multi_row = row()
        .key("locker-multi-skin")
        .w_full()
        .gap(12.0)
        .center_y()
        .label(tr("Several skins per hero"))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(tr("Several skins per hero")).font(BODY_MED).color(u.ink))
                .child(
                    text(tr("Equip adds a skin and keeps the others on, for skins that change different parts. Off: equip swaps."))
                        .font(SMALL)
                        .color(u.muted)
                        .wrap(),
                ),
        )
        .child(u.toggle(multi).label(tr("Several skins per hero")).cursor(rune::Cursor::Pointer).on_press(|st: &mut State| {
            st.locker.prefs.multi_skin = !st.locker.prefs.multi_skin;
            crate::data::save_locker_prefs(st);
        }));

    let mut buttons = row()
        .w_full()
        .gap(8.0)
        .child(spacer());
    if count > 0 {
        buttons = buttons.child(
            u.action("Shuffle now", ActionKind::Secondary)
                .key("locker-shuffle-now")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| crate::data::shuffle_now(st)),
        );
    }
    buttons = buttons.child(
        u.action("Done", ActionKind::Primary)
            .key("locker-shuffle-done")
            .cursor(rune::Cursor::Pointer)
            .on_press(close_dialog),
    );

    let body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(u, "Shuffle on launch".to_string(), "locker-shuffle-close"))
        .child(multi_row)
        .child(
            text("Each launch turns on one random skin for these heroes. Their other skins turn off.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(
                    text(if count == 0 {
                        "No hero shuffles yet.".to_string()
                    } else {
                        format!("{} of {} heroes", count, v.heroes.len())
                    })
                    .font(SMALL)
                    .color(u.faint),
                ),
        )
        .child(
            u.scroll_area(col()
                .key("locker-shuffle-scroll")
                .w_full()
                .max_h(360.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list)),
        )
        .child(buttons);

    shell(u, "locker-shuffle-dialog", "Shuffle on launch".to_string(), body)
}

fn empty_line(u: &Ui, line: &str) -> El {
    text(line.to_string()).font(BODY).color(u.muted).wrap()
}

fn sound_enabled(s: &State, m: &super::ModRow) -> bool {
    s.addons
        .addons
        .iter()
        .find(|addon| addon.slot == m.slot)
        .map(|addon| addon.enabled)
        .unwrap_or(m.enabled)
}

fn section_title(u: &Ui, title: &str, count: usize) -> El {
    row()
        .w_full()
        .h(28.0)
        .gap(8.0)
        .center_y()
        .child(text(title.to_string()).font(BODY_MED).color(u.ink))
        .child(
            text(plural(count, "item", "items"))
                .font(SMALL)
                .color(u.faint),
        )
}

pub(super) fn applied_dialog(s: &State, u: &Ui) -> El {
    let v = &s.locker;
    let cards: Vec<&HeroCard> = v
        .heroes
        .iter()
        .filter(|hero| hero.custom.is_some())
        .collect();
    let shuffled: Vec<&HeroCard> = v
        .heroes
        .iter()
        .filter(|hero| v.prefs.shuffles(&hero.display_name))
        .collect();
    let sounds: Vec<&super::ModRow> = v
        .mods
        .iter()
        .filter(|m| !m.sounds.is_empty() && sound_enabled(s, m))
        .collect();
    let briefs: &[BriefRow] = &v.briefs;

    let mut body = col()
        .w_full()
        .gap(16.0)
        .child(dialog_title(u, "Applied in the Locker".to_string(), "locker-applied-close"))
        .child(
            text("What the Locker set up. Remove any of it here.")
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );

    let nothing = cards.is_empty() && shuffled.is_empty() && sounds.is_empty() && briefs.is_empty();
    if nothing {
        body = body.child(empty_line(u, "Nothing applied from the Locker yet."));
    }

    let mut list = col().w_full().gap(16.0);

    if !cards.is_empty() {
        let mut section = col()
            .w_full()
            .gap(4.0)
            .child(section_title(u, "Cards", cards.len()));
        for hero in cards {
            let name = hero.display_name.clone();
            section = section.child(
                row()
                    .w_full()
                    .h(44.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        text(format!("{} card", hero.display_name))
                            .font(BODY_MED)
                            .color(u.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(
                        u.action("Remove", ActionKind::Ghost)
                            .key(format!("locker-applied-card-{}", hero.codename))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                crate::data::remove_locker_card(st, &name)
                            }),
                    ),
            );
        }
        list = list.child(section);
    }

    if !shuffled.is_empty() {
        let mut section = col()
            .w_full()
            .gap(4.0)
            .child(section_title(u, "Shuffle", shuffled.len()));
        for hero in shuffled {
            let name = hero.display_name.clone();
            section = section.child(
                row()
                    .w_full()
                    .h(44.0)
                    .gap(12.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        text(hero.display_name.clone())
                            .font(BODY_MED)
                            .color(u.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(
                        u.toggle(true)
                            .key(format!("locker-applied-shuffle-{}", hero.codename))
                            .label(format!("Shuffle {} at launch", hero.display_name))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                crate::data::toggle_locker_shuffle(st, &name)
                            }),
                    ),
            );
        }
        list = list.child(section);
    }

    if !sounds.is_empty() {
        let mut section = col()
            .w_full()
            .gap(4.0)
            .child(section_title(u, "Ability sounds", sounds.len()));
        for m in sounds {
            let slot = m.slot;
            section = section.child(
                row()
                    .w_full()
                    .h(44.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                text(m.label.clone())
                                    .font(BODY_MED)
                                    .color(u.ink)
                                    .truncate(),
                            )
                            .child(
                                text(format!("Slot {}", deadlock_core::addons::slot_label(m.slot)))
                                    .font(SMALL)
                                    .color(u.faint)
                                    .truncate(),
                            ),
                    )
                    .child(
                        u.action("Turn off", ActionKind::Ghost)
                            .key(format!("locker-applied-sound-{}", m.slot))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                crate::data::set_addon_enabled(st, slot, false)
                            }),
                    ),
            );
        }
        list = list.child(section);
    }

    if !briefs.is_empty() {
        let mut section = col()
            .w_full()
            .gap(4.0)
            .child(section_title(u, "Recolor briefs", briefs.len()));
        for brief in briefs {
            let hero = brief.hero.clone();
            section = section.child(
                row()
                    .w_full()
                    .h(44.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                text(brief.display_name.clone())
                                    .font(BODY_MED)
                                    .color(u.ink)
                                    .truncate(),
                            )
                            .child(
                                text(format!(
                                    "{}{}",
                                    brief.palette,
                                    if brief.note.trim().is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", brief.note)
                                    }
                                ))
                                .font(SMALL)
                                .color(u.faint)
                                .truncate(),
                            ),
                    )
                    .child(
                        u.action("Remove", ActionKind::Ghost)
                            .key(format!("locker-applied-brief-{hero}"))
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                crate::data::remove_locker_brief(st, &hero)
                            }),
                    ),
            );
        }
        list = list.child(section);
    }

    if !nothing {
        body = body.child(
            u.scroll_area(col()
                .key("locker-applied-scroll")
                .w_full()
                .max_h(420.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list)),
        );
    }

    body = body.child(
        row().w_full().gap(8.0).child(spacer()).child(
            u.action("Done", ActionKind::Primary)
                .key("locker-applied-done")
                .cursor(rune::Cursor::Pointer)
                .on_press(close_dialog),
        ),
    );

    shell(
        u,
        "locker-applied-dialog",
        "Applied in the Locker".to_string(),
        body,
    )
}
