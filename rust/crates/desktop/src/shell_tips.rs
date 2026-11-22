//! Shell additions from parity wave 4 (UI B), the second half (the first is `shell_w4.rs`):
//!
//! - Contextual feature tips (DMM's "Did you know?"): one card anchored to a control, Show me,
//!   Got it, Turn off tips. At most one a session, only after a page has been open a moment,
//!   never over a dialog, never before the setup is done. Got it, using it, or two dismissals
//!   retire a tip for good. Settings > General can turn them off and bring them back.
//! - The experimental tour: every experimental switch with what it does, opened from Settings >
//!   Experimental (and once on its first visit). Neither reference app has one.
//! - Easter eggs (Grimoire's), off by default: knock on the wordmark five times for the news
//!   computer, rare joke tooltips in the sidebar, and a rare window title.
//! - Seasonal themes (DMM's dates, Sigil's own look), off by default: a seasonal mark in the
//!   sidebar with a small toy each season, "Turn off until next time" in Settings.

use std::time::{Duration, Instant};

use rune::{Align, Color, El, Key, KeyEvent, Side, TextEdit, Tx, col, row, spacer, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::screens::settings::i18n::{fill, tr};
use crate::screens::settings::options;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

/// A page is open this long before a tip shows (DMM waits for 2 seconds of quiet).
const SETTLE: Duration = Duration::from_secs(2);
/// Two dismissals retire a tip, as DMM's do.
const IGNORES: u64 = 2;
/// Grimoire: five knocks inside two and a half seconds open the news computer.
const KNOCKS: usize = 5;
const KNOCK_WINDOW: Duration = Duration::from_millis(2_500);

#[derive(Default)]
pub struct Tips {
    /// The tip on screen, an index into `HINTS`.
    pub active: Option<usize>,
    /// A tip showed this run (one a session).
    shown: bool,
    /// The page and when it opened, for the settle delay.
    page_since: Option<(Screen, bool, Instant)>,
    /// The experimental tour's step, while it is open.
    pub tour: Option<usize>,
    /// Easter eggs: wordmark knocks, the news computer, joke tooltips rolled once.
    knocks: Vec<Instant>,
    pub toy: Option<Toy>,
    memes: Option<Vec<(Screen, &'static str)>>,
    titled: bool,
    /// Seasonal: Easter eggs found this run, the tree's lights.
    pub eggs_found: u32,
    pub lights: bool,
    counted_launch: bool,
    /// Scenes only: the season to show whatever today is.
    pub scene_season: Option<Season>,
    /// Seasonal effects: seconds of animation shown, and when the last step drew.
    pub effect_t: f32,
    effect_at: Option<Instant>,
    /// The season showing, worked out once a minute rather than every frame.
    season_cache: Option<(Instant, Option<Season>)>,
}

// --- contextual tips --------------------------------------------------------------------------

pub struct Hint {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    /// The element the card hangs from.
    pub anchor: &'static str,
}

pub const HINTS: [Hint; 4] = [
    Hint {
        id: "settings-search",
        title: "Search every setting",
        body: "Type a word like proxy or Discord. Cmd or Ctrl+F jumps here, Enter opens the first match.",
        anchor: "settings-search-field",
    },
    Hint {
        id: "sidebar-collapse",
        title: "More room for your mods",
        body: "Collapse the sidebar to icons. Cmd or Ctrl+B does it from anywhere.",
        anchor: "sidebar-collapse",
    },
    Hint {
        id: "stats-filter",
        title: "Ranked only, or one month",
        body: "The filter scopes every section, the overview and Heroes too.",
        anchor: "stats-filter",
    },
    Hint {
        id: "mods-folder",
        title: "Open the mods folder fast",
        body: "Cmd or Ctrl+Shift+M opens the addons folder from any page.",
        anchor: "nav-installed",
    },
];

fn bump(st: &mut State, key: &str) {
    let key = key.to_string();
    options::edit_quiet(|o| *o.counters.entry(key).or_insert(0) += 1);
    let _ = st;
}

fn counter(key: &str) -> u64 {
    options::get().counters.get(key).copied().unwrap_or(0)
}

/// Count a Settings visit (the settings-search tip waits for the third).
pub fn on_settings_open(st: &mut State) {
    bump(st, "settings_visits");
}

/// Count a page visit for the tips that wait for a habit.
pub fn on_page(st: &mut State, screen: Screen) {
    if screen == Screen::Stats {
        bump(st, "stats_visits");
    }
}

fn eligible(s: &State, hint: &Hint) -> bool {
    let busy = s.launch_dialog || s.launch.progress.is_some() || s.browse_view.details.is_some();
    match hint.id {
        "settings-search" => {
            s.settings.open && s.settings.confirm.is_none() && s.settings.search.text.is_empty() && counter("settings_visits") >= 3
        }
        "sidebar-collapse" => !s.settings.open && !busy && !s.shell.w4.collapsed && counter("launches") >= 3,
        "stats-filter" => {
            !s.settings.open && !busy && s.screen == Screen::Stats && s.stats.snapshot.is_some() && counter("stats_visits") >= 2
        }
        "mods-folder" => !s.settings.open && !busy && s.screen == Screen::Installed && !s.shell.w4.collapsed && counter("launches") >= 2,
        _ => false,
    }
}

/// Runs every poll: pick a tip once the page settled, at most one a run.
pub fn poll(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !st.shell.tips.counted_launch && !rune::is_headless() && !st.demo {
        st.shell.tips.counted_launch = true;
        bump(st, "launches");
    }
    changed |= rare_title(st, tx);
    changed |= crate::screens::settings::misc_w4::apply_type(st, tx);
    changed |= step_effects(st, tx);
    if rune::is_headless() || st.demo {
        return changed;
    }
    let here = (st.screen, st.settings.open);
    match st.shell.tips.page_since {
        Some((screen, open, _)) if (screen, open) == here => {}
        _ => st.shell.tips.page_since = Some((here.0, here.1, Instant::now())),
    }
    if let Some(index) = st.shell.tips.active {
        // The page moved on: the card goes, counting as a dismissal.
        if !eligible(st, &HINTS[index]) {
            dismiss(st);
            changed = true;
        }
        return changed;
    }
    let o = options::get();
    if st.shell.tips.shown || !o.feature_hints || !o.onboarded {
        return changed;
    }
    let Some((_, _, since)) = st.shell.tips.page_since else { return changed };
    if since.elapsed() < SETTLE {
        tx.wake_after(SETTLE - since.elapsed());
        return changed;
    }
    if let Some(index) = HINTS
        .iter()
        .position(|h| !o.hints_done.iter().any(|d| d == h.id) && eligible(st, h))
    {
        st.shell.tips.active = Some(index);
        st.shell.tips.shown = true;
        changed = true;
    }
    changed
}

fn retire(st: &mut State, id: &'static str) {
    st.shell.tips.active = None;
    options::edit(st, |o| {
        if !o.hints_done.iter().any(|d| d == id) {
            o.hints_done.push(id.to_string());
        }
    });
}

fn dismiss(st: &mut State) {
    let Some(index) = st.shell.tips.active.take() else { return };
    let id = HINTS[index].id;
    let key = format!("ignored_{id}");
    let ignored = counter(&key) + 1;
    options::edit(st, |o| {
        o.counters.insert(key, ignored);
        if ignored >= IGNORES && !o.hints_done.iter().any(|d| d == id) {
            o.hints_done.push(id.to_string());
        }
    });
}

fn show_me(st: &mut State, tx: &mut Tx) {
    let Some(index) = st.shell.tips.active else { return };
    let id = HINTS[index].id;
    retire(st, id);
    match id {
        "settings-search" => tx.focus("settings-search-input", true),
        "sidebar-collapse" => crate::shell_w4::toggle_sidebar(st),
        "stats-filter" => tx.focus("stats-filter", true),
        "mods-folder" => crate::shell::open_mods_folder(st),
        _ => {}
    }
}

/// The tip card, anchored to its control. `in_settings` mounts the settings tip inside the sheet.
pub fn view(s: &State, u: &Ui, in_settings: bool) -> Option<El> {
    let index = s.shell.tips.active?;
    let hint = &HINTS[index];
    if (hint.id == "settings-search") != in_settings {
        return None;
    }
    Some(
        col()
            .key("feature-tip")
            .role("dialog")
            .label(tr(hint.title))
            .w(300.0)
            .padding(14.0)
            .gap(8.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.accent.alpha(0.5))
            .shadow(12.0, 32.0, crate::ui::soft(Color::BLACK.alpha(0.45)))
            .anchored(hint.anchor, if hint.id == "mods-folder" || hint.id == "sidebar-collapse" { Side::Right } else { Side::Bottom }, Align::Start, 8.0)
            .transition(crate::motion::pop(false))
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    dismiss(st);
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
                    .child(rune::icon(&icons::SPARKLES, 14.0).color(u.accent).a11y_hidden())
                    .child(text(tr("Did you know?")).font(SMALL).color(u.accent).grow(1.0))
                    .child(
                        u.icon_button(&icons::X, tr("Dismiss"))
                            .key("feature-tip-close")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(dismiss),
                    ),
            )
            .child(text(tr(hint.title)).font(BODY_MED).color(u.ink))
            .child(text(tr(hint.body)).font(SMALL).color(u.muted).wrap())
            .child(
                row()
                    .w_full()
                    .gap(6.0)
                    .center_y()
                    .child(
                        u.action(tr("Turn off tips"), ActionKind::Ghost)
                            .key("feature-tip-off")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                st.shell.tips.active = None;
                                options::edit(st, |o| o.feature_hints = false);
                            }),
                    )
                    .child(spacer())
                    .child(
                        u.action(tr("Got it"), ActionKind::Ghost)
                            .key("feature-tip-ok")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(|st: &mut State| {
                                if let Some(index) = st.shell.tips.active {
                                    retire(st, HINTS[index].id);
                                }
                            }),
                    )
                    .child(
                        u.action(tr("Show me"), ActionKind::Primary)
                            .key("feature-tip-show")
                            .cursor(rune::Cursor::Pointer)
                            .on_press_tx(show_me),
                    ),
            ),
    )
}

// --- the experimental tour ------------------------------------------------------------------

/// One stop: a title, what it does, and the switch it is about (None for the intro and end).
struct Stop {
    title: &'static str,
    body: &'static str,
    flag: Option<&'static str>,
}

const TOUR: [Stop; 5] = [
    Stop {
        title: "Experimental features",
        body: "These work, but they are new or change how Sigil behaves. Each one is a switch you can turn off again here or in Settings > Experimental.",
        flag: None,
    },
    Stop {
        title: "Agent",
        body: "An assistant that builds, packs and checks mods with you, using your own model key. It runs as a second Sigil process while it is on, and asks before anything leaves your workspace.",
        flag: Some("agent"),
    },
    Stop {
        title: "Docked mod details",
        body: "Opens a mod from Browse in a panel beside the grid instead of over it. Drag the panel's edge to make it wider.",
        flag: Some("docked_details"),
    },
    Stop {
        title: "Imprint installed mods",
        body: "Writes the mod's identity into its VPK so Sigil, and other managers, can name it later. Settings > Experimental has the tool.",
        flag: None,
    },
    Stop {
        title: "That's the tour",
        body: "Open it again any time from Settings > Experimental.",
        flag: None,
    },
];

pub fn open_tour(st: &mut State) {
    st.shell.tips.tour = Some(0);
    options::edit_quiet(|o| o.tour_seen = true);
}

/// The first visit to Settings > Experimental opens the tour once.
pub fn on_experimental(st: &mut State) {
    if !options::get().tour_seen && !rune::is_headless() && !st.demo {
        open_tour(st);
    }
}

pub fn tour(s: &State, u: &Ui) -> Option<El> {
    let at = s.shell.tips.tour?;
    let stop = TOUR.get(at)?;
    let last = at + 1 == TOUR.len();
    let mut body = col()
        .w_full()
        .gap(10.0)
        .child(text(fill(tr("Step {} of {}"), &[&(at + 1).to_string(), &TOUR.len().to_string()])).font(SMALL).color(u.muted))
        .child(text(tr(stop.title)).font(H1).color(u.ink))
        .child(text(tr(stop.body)).font(BODY).color(u.muted).wrap());
    if let Some(flag) = stop.flag {
        let on = s.prefs.flag(flag);
        body = body.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(text(tr(if on { "On" } else { "Off" })).font(BODY_MED).color(u.ink).grow(1.0))
                .child(
                    u.toggle(on)
                        .key(format!("tour-flag-{flag}"))
                        .label(tr(stop.title))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |st: &mut State| {
                            crate::prefs::edit(st, |p| p.toggle_flag(flag));
                            crate::screens::settings::apply_flags(st);
                        }),
                ),
        );
    }
    let close = |st: &mut State| st.shell.tips.tour = None;
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .pt(6.0)
            .center_y()
            .child(
                u.action(tr("Close"), ActionKind::Ghost)
                    .key("tour-close")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(close),
            )
            .child(spacer())
            .children((at > 0).then(|| {
                u.action(tr("Back"), ActionKind::Secondary)
                    .key("tour-back")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.shell.tips.tour = Some(at - 1))
            }))
            .child(
                u.action(tr(if last { "Done" } else { "Next" }), ActionKind::Primary)
                    .key("tour-next")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| st.shell.tips.tour = if last { None } else { Some(at + 1) }),
            ),
    );
    Some(
        u.dialog_shell()
            .key("experimental-tour")
            .role("dialog")
            .label(tr("Experimental features"))
            .w(460.0)
            .padding(24.0)
            .modal(true, close)
            .child(body),
    )
}

// --- easter eggs ----------------------------------------------------------------------------

pub fn eggs_on() -> bool {
    options::get().easter_eggs
}

/// The news computer (Grimoire's loader toy): a little terminal that answers.
pub struct Toy {
    pub lines: Vec<String>,
    pub input: TextEdit,
}

/// A press on the wordmark; the fifth inside the window opens the news computer.
pub fn knock(st: &mut State) {
    if !eggs_on() {
        return;
    }
    let now = Instant::now();
    st.shell.tips.knocks.retain(|at| now.duration_since(*at) <= KNOCK_WINDOW);
    st.shell.tips.knocks.push(now);
    if st.shell.tips.knocks.len() >= KNOCKS {
        st.shell.tips.knocks.clear();
        st.shell.tips.toy = Some(Toy {
            lines: vec![tr("NEWS COMPUTER v0.1. Type help.").to_string()],
            input: TextEdit::new(""),
        });
    }
}

/// What the news computer says to a line.
pub fn answer(line: &str) -> Vec<String> {
    let lower = line.trim().to_ascii_lowercase();
    match lower.as_str() {
        "" => Vec::new(),
        "help" => vec![tr("help, door stuck, nerf bebop, sigil, clear, exit, or a hero's name").to_string()],
        "door stuck" => vec![tr("DOOR STUCK. DOOR STUCK. Please, I beg you.").to_string()],
        "nerf bebop" => vec![tr("The hook has been noted. Nothing will change.").to_string()],
        "sigil" | "grimoire" | "book" => vec![tr("A mod manager walks into the addons folder. It loads first.").to_string()],
        "gamer" | "rgb" => vec![tr("RGB on. Frame rate up 12 percent, says nobody.").to_string()],
        _ => match deadlock_core::locker::roster_hero(line.trim()) {
            Some(hero) => vec![fill(tr("{}: a fine pick. The patch notes disagree."), &[hero.name])],
            None => vec![fill(tr("Unknown command: {}"), &[line.trim()])],
        },
    }
}

fn submit(st: &mut State) {
    let Some(toy) = st.shell.tips.toy.as_mut() else { return };
    let line = toy.input.text.clone();
    toy.input = TextEdit::new("");
    let lower = line.trim().to_ascii_lowercase();
    if lower == "exit" || lower == "quit" || lower == "bye" {
        st.shell.tips.toy = None;
        return;
    }
    if lower == "clear" || lower == "cls" {
        toy.lines.clear();
        return;
    }
    toy.lines.push(format!("> {}", line.trim()));
    toy.lines.extend(answer(&line));
    let keep = toy.lines.len().saturating_sub(12);
    toy.lines.drain(..keep);
}

pub fn toy(s: &State, u: &Ui) -> Option<El> {
    let toy = s.shell.tips.toy.as_ref()?;
    let close = |st: &mut State| st.shell.tips.toy = None;
    let mut screen = col().w_full().min_h(180.0).padding(12.0).gap(4.0).radius(crate::ui::r(6.0)).fill(Color::BLACK.alpha(0.85));
    for (index, line) in toy.lines.iter().enumerate() {
        screen = screen.child(text(line.clone()).key(format!("toy-line-{index}")).font(MONO).color(u.teal).wrap());
    }
    screen = screen.child(
        rune::text_input(&toy.input, MONO)
            .key("toy-input")
            .label(tr("News computer"))
            .single_line()
            .w_full()
            .color(u.teal)
            .placeholder("> ", u.teal.alpha(0.5))
            .caret(u.teal, u.teal.alpha(0.22))
            .on_edit(|st: &mut State| &mut st.shell.tips.toy.as_mut().expect("toy open").input)
            .on_submit(|st: &mut State, _| submit(st))
            .autofocus(),
    );
    Some(
        u.dialog_shell()
            .key("news-computer")
            .role("dialog")
            .label(tr("News computer"))
            .w(480.0)
            .padding(20.0)
            .gap(12.0)
            .modal(true, close)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(text(tr("News computer")).font(BODY_MED).color(u.ink).grow(1.0))
                    .child(u.icon_button(&icons::X, tr("Close")).key("toy-close").cursor(rune::Cursor::Pointer).on_press(close)),
            )
            .child(screen),
    )
}

/// Grimoire's joke names, rolled once a run at 7 percent each, kept out of translation.
const MEMES: [(Screen, &str); 8] = [
    (Screen::Browse, "Mod Shopping Spree"),
    (Screen::Locker, "Fashion Week"),
    (Screen::Crosshair, "Aimbot (Legal)"),
    (Screen::Autoexec, "Hacker Voice: I'm In"),
    (Screen::Stats, "Losing Streak"),
    (Screen::Conflicts, "They Are Fighting"),
    (Screen::Profiles, "Alt Accounts"),
    (Screen::Servers, "Friend Simulator"),
];

/// A sidebar item's joke tooltip, when the roll picked it.
pub fn meme(s: &State, screen: Screen) -> Option<&'static str> {
    s.shell.tips.memes.as_ref()?.iter().find(|(m, _)| *m == screen).map(|(_, words)| *words)
}

fn roll(seed: u64, salt: u64, percent: u64) -> bool {
    let mixed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(salt.wrapping_mul(1_442_695_040_888_963_407));
    (mixed >> 33) % 100 < percent
}

/// Once a run: roll the joke tooltips and, at 5 percent, the window title.
fn rare_title(st: &mut State, tx: &mut Tx) -> bool {
    if st.shell.tips.titled || !st.shell.w4.eggs {
        return false;
    }
    st.shell.tips.titled = true;
    let seed = deadlock_core::library_facts::random_seed();
    st.shell.tips.memes = Some(
        MEMES
            .iter()
            .enumerate()
            .filter(|(i, _)| roll(seed, *i as u64 + 1, 7))
            .map(|(_, m)| *m)
            .collect(),
    );
    if roll(seed, 99, 5) {
        tx.set_title("DeadGame");
    }
    true
}

// --- seasonal themes ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Season {
    Halloween,
    Christmas,
    NewYear,
    LunarNewYear,
    Easter,
}

impl Season {
    pub fn id(self) -> &'static str {
        match self {
            Season::Halloween => "halloween",
            Season::Christmas => "christmas",
            Season::NewYear => "newyear",
            Season::LunarNewYear => "lunarnewyear",
            Season::Easter => "easter",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Season::Halloween => "Halloween",
            Season::Christmas => "Christmas",
            Season::NewYear => "New Year's Eve",
            Season::LunarNewYear => "Lunar New Year",
            Season::Easter => "Easter",
        }
    }

    fn tint(self, u: &Ui) -> Color {
        match self {
            Season::Halloween => rune::color::hex("#f2851f").unwrap_or(u.amber),
            Season::Christmas => rune::color::hex("#db383d").unwrap_or(u.rose),
            Season::NewYear => rune::color::hex("#f2c74d").unwrap_or(u.amber),
            Season::LunarNewYear => rune::color::hex("#e6292e").unwrap_or(u.rose),
            Season::Easter => u.teal,
        }
    }
}

/// Days since 1970-01-01 for a calendar date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Easter Sunday (the anonymous Gregorian computus), as days since 1970.
pub fn easter(year: i64) -> i64 {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    days_from_civil(year, month as u32, day as u32)
}

/// Lunar New Year's day, 2025 to 2040 (outside the table the season is off).
fn lunar_new_year(year: i64) -> Option<i64> {
    const DAYS: [(u32, u32); 16] = [
        (1, 29), (2, 17), (2, 6), (1, 26), (2, 13), (2, 3), (1, 23), (2, 11),
        (1, 31), (2, 19), (2, 8), (1, 28), (2, 15), (2, 4), (1, 24), (2, 12),
    ];
    let (m, d) = *DAYS.get(usize::try_from(year - 2025).ok()?)?;
    Some(days_from_civil(year, m, d))
}

/// Every season window around `today` (days since 1970): (season, first day, last day, the
/// festival day). Windows of this year and the next and the last, so one that crosses New Year
/// is found from either side.
fn windows(year: i64) -> Vec<(Season, i64, i64, i64)> {
    let mut out = Vec::new();
    for y in [year - 1, year, year + 1] {
        out.push((Season::Halloween, days_from_civil(y, 10, 1), days_from_civil(y, 11, 1), days_from_civil(y, 10, 31)));
        out.push((Season::Christmas, days_from_civil(y, 12, 1), days_from_civil(y, 12, 26), days_from_civil(y, 12, 25)));
        out.push((Season::NewYear, days_from_civil(y, 12, 27), days_from_civil(y + 1, 1, 1), days_from_civil(y + 1, 1, 1)));
        if let Some(day) = lunar_new_year(y) {
            out.push((Season::LunarNewYear, day - 21, day + 14, day));
        }
        let sunday = easter(y);
        out.push((Season::Easter, sunday - 30, sunday + 1, sunday));
    }
    out
}

/// The season in its window on `today`; overlapping windows go to the closer festival (DMM).
pub fn season_on(today: i64) -> Option<(Season, i64)> {
    let year = crate::screens::settings::log::civil_year(today);
    windows(year)
        .into_iter()
        .filter(|(_, from, to, _)| (*from..=*to).contains(&today))
        .min_by_key(|(_, _, _, festival)| (festival - today).abs())
        .map(|(season, _, to, _)| (season, to))
}

/// The next season to start after `today`.
pub fn next_season(today: i64) -> Option<(Season, i64)> {
    let year = crate::screens::settings::log::civil_year(today);
    windows(year)
        .into_iter()
        .filter(|(_, from, _, _)| *from > today)
        .min_by_key(|(_, from, _, _)| *from)
        .map(|(season, from, _, _)| (season, from))
}

pub fn today() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0)
}

fn dismiss_key(season: Season, today: i64) -> String {
    format!("{}:{}", season.id(), crate::screens::settings::log::civil_year(today))
}

/// The season showing now: the switch is on and this year's season was not turned off.
pub fn active_season(today: i64) -> Option<Season> {
    let o = options::get();
    if !o.seasonal {
        return None;
    }
    let (season, _) = season_on(today)?;
    (o.seasonal_dismissed.as_deref() != Some(dismiss_key(season, today).as_str())).then_some(season)
}

/// The season's mark at the foot of the sidebar; pressing it plays the season's little toy.
pub fn season_mark(s: &State, u: &Ui) -> Option<El> {
    let season = s.shell.tips.scene_season.or_else(|| s.shell.w4.seasonal.then(|| active_season(today())).flatten())?;
    let tint = season.tint(u);
    let collapsed = s.shell.w4.collapsed;
    let mut mark = row()
        .key("season-mark")
        .w_full()
        .h(crate::ui::CONTROL_L)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(tint.alpha(0.10))
        .border(1.0, tint.alpha(0.35))
        .hover(tint.alpha(0.18))
        .focusable(true)
        .role("button")
        .label(tr(season.name()))
        .help(tr(season.name()))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| season_toy(st, season))
        .child(rune::icon(&icons::SPARKLES, 14.0).color(tint).a11y_hidden());
    if collapsed {
        mark = mark.px(0.0).center();
    } else {
        mark = mark.child(text(tr(season.name())).font(SMALL).color(u.ink).grow(1.0));
    }
    Some(mark)
}

/// One small thing a season does when its mark is pressed.
fn season_toy(st: &mut State, season: Season) {
    let line = match season {
        Season::Halloween => tr("Boo!").to_string(),
        Season::Christmas => {
            st.shell.tips.lights = !st.shell.tips.lights;
            tr(if st.shell.tips.lights { "Tree lights on." } else { "Tree lights off." }).to_string()
        }
        Season::NewYear => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let left = 86_400 - secs % 86_400;
            fill(tr("Rocket launched. {} h {} min to midnight (UTC)."), &[&(left / 3_600).to_string(), &(left % 3_600 / 60).to_string()])
        }
        Season::LunarNewYear => {
            const BLESSINGS: [&str; 4] = [
                "A red envelope: good fortune in every match.",
                "A red envelope: may your mods always load first.",
                "A red envelope: health and a clean gameinfo.gi.",
                "A red envelope: may the patch notes be kind.",
            ];
            tr(BLESSINGS[(deadlock_core::library_facts::random_seed() % 4) as usize]).to_string()
        }
        Season::Easter => {
            st.shell.tips.eggs_found = (st.shell.tips.eggs_found + 1).min(5);
            fill(tr("{} of 5 eggs found."), &[&st.shell.tips.eggs_found.to_string()])
        }
    };
    crate::notify::push(st, crate::notify::Notice::info(tr(season.name())).detail(line));
}

/// Settings > Appearance: the switch, where the season stands, and "Turn off until next time".
pub fn season_line(today: i64) -> String {
    let o = options::get();
    match season_on(today) {
        Some((season, _)) if o.seasonal_dismissed.as_deref() == Some(dismiss_key(season, today).as_str()) => {
            fill(tr("{} is off until it comes round again."), &[tr(season.name())])
        }
        Some((season, until)) => fill(tr("{} is on until {}."), &[tr(season.name()), &date(until)]),
        None => match next_season(today) {
            Some((season, from)) => fill(tr("Next up: {} from {}."), &[tr(season.name()), &date(from)]),
            None => String::new(),
        },
    }
}

fn date(days: i64) -> String {
    let (y, m, d) = crate::screens::settings::log::civil_date(days);
    format!("{y}-{m:02}-{d:02}")
}

/// Turn this year's season off (DMM's dismiss), or bring it back.
pub fn toggle_dismiss(st: &mut State) {
    let today = today();
    let Some((season, _)) = season_on(today) else { return };
    let key = dismiss_key(season, today);
    options::edit(st, |o| {
        o.seasonal_dismissed = if o.seasonal_dismissed.as_deref() == Some(key.as_str()) { None } else { Some(key) };
    });
}

// --- seasonal effects -----------------------------------------------------------------------

/// Effects step at 15 frames a second: smooth enough for drifting snow, a quarter of the
/// redraws of full motion, and only while the season shows and the switch is on.
const EFFECT_STEP: Duration = Duration::from_millis(66);
/// Particles on screen at once.
const PARTICLES: usize = 28;

/// The season the effects draw, cached for a minute.
fn effect_season(st: &mut State) -> Option<Season> {
    if let Some(season) = st.shell.tips.scene_season {
        return Some(season);
    }
    if !st.shell.w4.seasonal {
        return None;
    }
    match st.shell.tips.season_cache {
        Some((at, season)) if at.elapsed() < Duration::from_secs(60) => season,
        _ => {
            let season = active_season(today());
            st.shell.tips.season_cache = Some((Instant::now(), season));
            season
        }
    }
}

fn step_effects(st: &mut State, tx: &mut Tx) -> bool {
    if !st.shell.w4.effects || effect_season(st).is_none() || rune::is_headless() {
        return false;
    }
    let now = Instant::now();
    let due = st.shell.tips.effect_at.is_none_or(|at| now.duration_since(at) >= EFFECT_STEP);
    if due {
        st.shell.tips.effect_t += EFFECT_STEP.as_secs_f32();
        st.shell.tips.effect_at = Some(now);
    }
    tx.wake_after(EFFECT_STEP);
    due
}

/// A cheap repeatable number in 0..1 for particle `i` and salt `k`.
fn noise(i: usize, k: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(2_654_435_761) ^ k.wrapping_mul(40_503);
    x ^= x >> 15;
    x = x.wrapping_mul(2_246_822_519);
    x ^= x >> 13;
    (x % 10_000) as f32 / 10_000.0
}

/// Where particle `i` is at time `t` in a `w` by `h` window, its size and its alpha. Snow and
/// leaves fall, sparks and lanterns rise, Easter's petals drift.
pub fn particle(season: Season, i: usize, t: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let speed = 18.0 + noise(i, 1) * 34.0;
    let size = 3.0 + noise(i, 2) * 5.0;
    let x0 = noise(i, 3) * w;
    let travel = h + 40.0;
    let along = (noise(i, 4) * travel + t * speed) % travel;
    let sway = (t * (0.6 + noise(i, 5)) + noise(i, 6) * std::f32::consts::TAU).sin() * 14.0;
    let (y, alpha) = match season {
        Season::Christmas | Season::Halloween => (along - 20.0, 0.75),
        Season::NewYear | Season::LunarNewYear => (h - along + 20.0, 0.8 - along / travel * 0.6),
        Season::Easter => (along - 20.0, 0.55),
    };
    (x0 + sway, y, size, alpha.clamp(0.0, 1.0))
}

/// The effect layer over the pages, drawn from the current time step. No input: it never takes
/// a press.
pub fn effects(s: &State, u: &Ui, window: (f32, f32)) -> Option<El> {
    if !s.shell.w4.effects {
        return None;
    }
    let season = s.shell.tips.scene_season.or_else(|| s.shell.w4.seasonal.then(|| active_season(today())).flatten())?;
    let tint = season.tint(u);
    let color = match season {
        Season::Christmas => Color::WHITE,
        Season::Easter => tint.mix(Color::WHITE, 0.5),
        _ => tint,
    };
    let t = s.shell.tips.effect_t;
    let mut layer = rune::el().key("season-effects").absolute(0.0, 0.0).w(window.0).h(window.1).a11y_hidden();
    for i in 0..PARTICLES {
        let (x, y, size, alpha) = particle(season, i, t, window.0, window.1);
        let lantern = season == Season::LunarNewYear;
        layer = layer.child(
            rune::el()
                .absolute(x, y)
                .size(size, if lantern { size * 1.4 } else { size })
                .radius(if lantern { size * 0.3 } else { size / 2.0 })
                .fill(color.alpha(alpha * 0.8)),
        );
    }
    Some(layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easter_lands_on_known_sundays() {
        assert_eq!(easter(2026), days_from_civil(2026, 4, 5));
        assert_eq!(easter(2027), days_from_civil(2027, 3, 28));
        assert_eq!(easter(2024), days_from_civil(2024, 3, 31));
    }

    #[test]
    fn seasons_follow_the_calendar() {
        assert_eq!(season_on(days_from_civil(2026, 10, 20)).map(|s| s.0), Some(Season::Halloween));
        assert_eq!(season_on(days_from_civil(2026, 12, 24)).map(|s| s.0), Some(Season::Christmas));
        assert_eq!(season_on(days_from_civil(2026, 12, 30)).map(|s| s.0), Some(Season::NewYear));
        assert_eq!(season_on(days_from_civil(2027, 1, 1)).map(|s| s.0), Some(Season::NewYear));
        assert_eq!(season_on(days_from_civil(2027, 2, 10)).map(|s| s.0), Some(Season::LunarNewYear));
        assert_eq!(season_on(days_from_civil(2026, 7, 1)), None);
        assert_eq!(next_season(days_from_civil(2026, 7, 1)).map(|s| s.0), Some(Season::Halloween));
        // Lunar New Year 2027 (Feb 6) and Easter 2027 (Mar 28) overlap in early March: the
        // closer festival wins.
        assert_eq!(season_on(days_from_civil(2027, 2, 19)).map(|s| s.0), Some(Season::LunarNewYear));
        assert_eq!(season_on(days_from_civil(2027, 3, 20)).map(|s| s.0), Some(Season::Easter));
    }

    #[test]
    fn particles_stay_near_the_window() {
        for season in [Season::Halloween, Season::Christmas, Season::NewYear, Season::LunarNewYear, Season::Easter] {
            for i in 0..PARTICLES {
                for t in [0.0, 3.3, 100.0] {
                    let (x, y, size, alpha) = particle(season, i, t, 800.0, 600.0);
                    assert!((-20.0..=820.0).contains(&x) && (-25.0..=625.0).contains(&y), "{season:?} {x} {y}");
                    assert!(size >= 3.0 && (0.0..=1.0).contains(&alpha));
                }
            }
        }
    }

    #[test]
    fn the_news_computer_answers() {
        assert!(answer("help")[0].contains("door stuck"));
        assert!(answer("Haze")[0].starts_with("Haze"));
        assert!(answer("xyzzy")[0].contains("xyzzy"));
        assert!(answer("  ").is_empty());
    }

    #[test]
    fn every_tip_has_words_and_an_anchor() {
        for hint in &HINTS {
            assert!(!hint.title.is_empty() && !hint.body.is_empty() && !hint.anchor.is_empty());
        }
    }
}
