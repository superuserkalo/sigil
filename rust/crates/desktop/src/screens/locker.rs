//! Locker: hero renders and asset paths resolved from the local machine.
//!
//! `deadlock_core::locker` resolves the hero list from the game catalog and the
//! local art root. The art is read at runtime from a configured folder and is
//! never copied into this repository or redistributed. A missing art root is a
//! valid state: heroes list, and each card says the render is unavailable.

use std::collections::HashMap;

use rune::{Color, El, ScrollSpec, SurfaceFit, col, el, row, spacer, text};

use crate::app::State;
use crate::model::Screen;
use crate::ui::{ActionKind, BODY_MED, H2, LABEL, SMALL, Tone, Ui};

#[derive(Clone)]
pub struct HeroCard {
    pub codename: String,
    pub display_name: String,
    /// Render bytes and the path they came from.
    pub render: Option<(String, Vec<u8>)>,
    pub name_art: Option<(String, Vec<u8>)>,
    pub mod_count: usize,
    pub asset_count: usize,
    pub in_catalog: bool,
}

pub struct Load {
    pub heroes: Vec<HeroCard>,
    pub assets_dir: String,
    pub catalog_label: String,
    pub error: Option<String>,
}

pub struct View {
    pub loaded: bool,
    pub heroes: Vec<HeroCard>,
    pub assets_dir: String,
    pub catalog_label: String,
    pub error: Option<String>,
    pub modded_only: bool,
    pub selected: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            heroes: Vec::new(),
            assets_dir: String::new(),
            catalog_label: String::new(),
            error: None,
            modded_only: false,
            selected: None,
        }
    }
}

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.locker;
    let shown: Vec<&HeroCard> = v
        .heroes
        .iter()
        .filter(|hero| !v.modded_only || hero.mod_count > 0)
        .collect();
    let with_art = v.heroes.iter().filter(|hero| hero.render.is_some()).count();

    let trailing = row()
        .gap(8.0)
        .center_y()
        .child(if s.locker_job.is_running() {
            u.status_chip("resolving", Tone::Warn)
        } else if v.error.is_some() {
            u.status_chip("catalog error", Tone::Bad)
        } else if v.loaded {
            u.status_chip(
                &format!("{with_art} with art"),
                if with_art == 0 { Tone::Quiet } else { Tone::Ok },
            )
        } else {
            u.status_chip("not resolved", Tone::Quiet)
        })
        .child(toggle_chip(u, "Modded only", v.modded_only, |st| {
            st.locker.modded_only = !st.locker.modded_only
        }))
        .child(
            u.pill("Rescan", "↻", ActionKind::Ghost)
                .key("locker-rescan")
                .on_press(|st: &mut State| crate::data::start_locker_load(st)),
        );

    let detail = if let Some(error) = &v.error {
        error.clone()
    } else if !v.loaded {
        "Loading the game catalog and resolving local art.".to_string()
    } else if v.heroes.is_empty() {
        "The game catalog has no hero folders yet. Run a catalog load from Settings or Updates.".to_string()
    } else {
        format!(
            "{} heroes · {} · art read from {}",
            v.heroes.len(),
            v.catalog_label,
            if v.assets_dir.is_empty() {
                "no local art root"
            } else {
                &v.assets_dir
            }
        )
    };

    let mut page = col()
        .key("locker-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .gap(13.0)
        .child(u.glow(
            "locker-glow",
            [0.86, 0.02],
            (760.0, 460.0),
            u.glow_warm,
        ))
        .child(u.page_head(
            Screen::Locker.kicker(),
            "Locker",
            &detail,
            Some(trailing),
        ));

    if v.loaded && with_art == 0 && !v.heroes.is_empty() {
        page = page.child(art_notice(v));
    }

    let body = if !v.loaded && s.locker_job.is_running() {
        loading(u)
    } else if v.heroes.is_empty() {
        u.empty_state(
            "◐",
            "No heroes resolved",
            "The Locker reads hero folders from the game index. Without a catalog there is no hero list, and the screen stays empty instead of inventing one.",
            Some(
                u.action("Rescan", ActionKind::Secondary)
                    .on_press(|st: &mut State| crate::data::start_locker_load(st)),
            ),
        )
    } else if shown.is_empty() {
        u.empty_state(
            "◐",
            "No modded heroes",
            "No enabled addon ships a path under a hero folder. Clear the filter to see every hero.",
            Some(
                u.action("Show all heroes", ActionKind::Secondary)
                    .on_press(|st: &mut State| st.locker.modded_only = false),
            ),
        )
    } else {
        let mut grid = col().w_full().gap(12.0);
        for start in (0..shown.len()).step_by(4) {
            let mut r = row().w_full().gap(12.0).items_start();
            for index in start..(start + 4).min(shown.len()) {
                r = r.child(card(s, u, shown[index]));
            }
            for _ in (start..(start + 4).min(shown.len())).count()..4 {
                r = r.child(el().grow(1.0).min_w(0.0).h(1.0));
            }
            grid = grid.child(r);
        }
        col()
            .key("locker-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(grid)
    };

    page.child(body)
}

fn toggle_chip(u: &Ui, label: &str, on: bool, press: fn(&mut State)) -> El {
    row()
        .h(27.0)
        .px(10.0)
        .gap(6.0)
        .center()
        .radius(13.0)
        .fill(if on {
            u.accent.alpha(0.16)
        } else {
            Color::WHITE.alpha(0.03)
        })
        .border(
            1.0,
            if on {
                u.accent.alpha(0.28)
            } else {
                u.hair
            },
        )
        .focusable(true)
        .role("switch")
        .checked(on)
        .label(label)
        .child(u.dot(if on { Tone::Accent } else { Tone::Quiet }))
        .child(
            text(label)
                .font(LABEL)
                .color(if on { u.accent } else { u.muted }),
        )
        .on_press(press)
}

fn art_notice(v: &View) -> El {
    let detail = if v.assets_dir.is_empty() {
        "Set DAH_LOCKER_ASSETS to a folder with heroes/ and names/, or place a Grimoire checkout at vendor/grimoire/public. Art is read there, never copied into this app."
    } else {
        "The art root exists but no hero render matched. The resolver tries the hero code name, the display name and the portrait fallback."
    };
    row()
        .w_full()
        .padding(11.0)
        .gap(10.0)
        .items_start()
        .radius(10.0)
        .fill(Color::WHITE.alpha(0.024))
        .border(1.0, Color::WHITE.alpha(0.06))
        .child(text("ART").font(LABEL).color(Color::WHITE.alpha(0.55)))
        .child(text(detail).font(SMALL).color(Color::WHITE.alpha(0.62)).wrap())
}

fn card(s: &State, u: &Ui, hero: &HeroCard) -> El {
    let codename = hero.codename.clone();
    let display_name = hero.display_name.clone();
    let asset_count = hero.asset_count;
    let pixels = hero
        .render
        .as_ref()
        .and_then(|(path, _)| s.thumbnails.get(path));
    let image = match pixels {
        Some(pixels) => el()
            .key(format!("render-{}", hero.codename))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pixels_fit(pixels, SurfaceFit::Fill)
            .a11y_hidden(),
        None => el()
            .key(format!("render-missing-{}", hero.codename))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(hero_tint(u, &hero.codename))
            .center()
            .child(
                col()
                    .gap(6.0)
                    .items_center()
                    .child(
                        text(initials(&hero.display_name))
                            .font(H2)
                            .color(Color::WHITE.alpha(0.35)),
                    )
                    .child(text("no render").font(LABEL).color(Color::WHITE.alpha(0.30))),
            )
            .a11y_hidden(),
    };

    let selected = s.locker.selected.as_deref() == Some(hero.codename.as_str());
    let mod_chip = if hero.mod_count > 0 {
        u.chip(
            &format!("{} mod{}", hero.mod_count, if hero.mod_count == 1 { "" } else { "s" }),
            u.accent_ink,
            u.accent,
        )
    } else {
        u.chip("vanilla", Color::WHITE.alpha(0.66), Color::BLACK.alpha(0.45))
    };

    col()
        .key(format!("locker-{}", hero.codename))
        .grow(1.0)
        .min_w(0.0)
        .radius(14.0)
        .fill(Color::WHITE.alpha(0.026))
        .border(
            1.0,
            if selected {
                u.accent.alpha(0.45)
            } else {
                u.hair
            },
        )
        .hover(Color::WHITE.alpha(0.04))
        .focusable(true)
        .role("button")
        .label(format!("{} hero card", hero.display_name))
        .child(
            el()
                .w_full()
                .aspect_ratio(0.78)
                .fill(Color::BLACK.alpha(0.25))
                .child(image)
                .child(
                    row()
                        .absolute(8.0, 8.0)
                        .gap(5.0)
                        .center_y()
                        .child(mod_chip),
                ),
        )
        .child(
            col()
                .w_full()
                .grow(1.0)
                .padding(11.0)
                .gap(3.0)
                .child(
                    text(hero.display_name.clone())
                        .font(BODY_MED)
                        .color(u.ink)
                        .truncate(),
                )
                .child(
                    text(hero.codename.clone())
                        .font(LABEL)
                        .color(u.faint)
                        .truncate(),
                )
                .child(
                    text(if hero.in_catalog {
                        format!(
                            "{} catalog path{}",
                            hero.asset_count,
                            if hero.asset_count == 1 { "" } else { "s" }
                        )
                    } else {
                        "art only, not in this catalog".to_string()
                    })
                    .font(SMALL)
                    .color(if hero.in_catalog { u.muted } else { u.amber })
                    .truncate(),
                ),
        )
        .on_press(move |st: &mut State| {
            st.locker.selected = Some(codename.clone());
            st.status = format!(
                "{} resolves {} catalog path(s) under models/heroes. Recolor and sound editing need a Source 2 asset backend.",
                display_name, asset_count
            );
        })
}

fn hero_tint(u: &Ui, codename: &str) -> Color {
    let palette = [u.violet, u.teal, u.accent, u.rose];
    let index = codename.bytes().map(|byte| byte as usize).sum::<usize>() % palette.len();
    palette[index].alpha(0.10)
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn loading(u: &Ui) -> El {
    let mut grid = col().w_full().gap(12.0);
    for _ in 0..2 {
        let mut r = row().w_full().gap(12.0);
        for _ in 0..4 {
            r = r.child(
                el()
                    .grow(1.0)
                    .min_w(0.0)
                    .aspect_ratio(0.78)
                    .radius(14.0)
                    .fill(Color::WHITE.alpha(0.022))
                    .border(1.0, u.hair),
            );
        }
        grid = grid.child(r);
    }
    grid
}

/// Decode every resolved render on the UI thread. Bytes were read on a worker.
pub fn decode(s: &mut State, heroes: &[HeroCard]) {
    let Some(cache) = s.images.as_ref() else {
        return;
    };
    let mut map: HashMap<String, ()> = HashMap::new();
    for hero in heroes {
        if let Some((path, bytes)) = &hero.render
            && map.insert(path.clone(), ()).is_none()
        {
            let image = cache.load(bytes.clone());
            if rune::is_headless() {
                let _ = image.wait();
            }
            s.thumbnails.insert(path.clone(), image.pixels().clone());
        }
    }
}
