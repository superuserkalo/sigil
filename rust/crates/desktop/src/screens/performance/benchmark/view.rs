//! The Benchmark page: setup, the running pass, a result, history and compare.

use std::collections::BTreeMap;

use deadlock_core::benchmark::compare::{Comparison, Verdict};
use deadlock_core::benchmark::metrics::{Bin, Summary};
use deadlock_core::benchmark::store::{self, Outcome, RunRecord, run_label};
use deadlock_core::benchmark::system;
use deadlock_core::benchmark::{CostEnd, DemoSource, Display, Length, Side, Test, Variant};
use rune::{Color, El, Shape, TextStyle, Tx, canvas, col, el, icon, row, spacer, text};

use super::{Page, Pick, Shared, Tab, tr};
use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::fill;
use crate::screens::subpage;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, PAGE_TOP, PAGE_X, SMALL, Ui};

const BIG: TextStyle = TextStyle::new(56.0, 600.0).lh(60.0).tracking(-1.2).features("'tnum' 1");
const VALUE: TextStyle = TextStyle::new(20.0, 600.0).lh(26.0).features("'tnum' 1");
const TNUM: TextStyle = TextStyle::new(13.0, 420.0).lh(20.0).features("'tnum' 1");
const TNUM_MED: TextStyle = TextStyle::new(13.0, 560.0).lh(20.0).features("'tnum' 1");
const TNUM_SMALL: TextStyle = TextStyle::new(11.0, 430.0).lh(14.0).features("'tnum' 1");
const GRAPH_H: f32 = 180.0;
const GRAPH_VB: f32 = 1000.0;
const AXIS_W: f32 = 44.0;

fn fps(v: f64) -> String {
    if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

fn ms(v: f64) -> String {
    format!("{v:.1} ms")
}

fn opt(v: Option<f64>, f: fn(f64) -> String) -> String {
    v.map(f).unwrap_or_else(|| "-".into())
}

fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// "21 Nov, 18:02" from an RFC 3339 time.
fn when(at: &str) -> String {
    deadlock_core::benchmark::local_when(at)
}

fn line(words: impl Into<String>, color: Color) -> El {
    text(words.into()).font(SMALL).color(color).wrap()
}

fn card(key: &str, u: &Ui) -> El {
    col().key(key.to_string()).w_full().padding(20.0).gap(14.0).radius(crate::ui::r(4.0)).border(1.0, u.hair)
}

fn variant_label(s: &State, run: &RunRecord, v: &Variant) -> String {
    match v {
        Variant::Vanilla => tr("Vanilla").to_string(),
        Variant::AsIs => match run.spec.test.sides() {
            Some((_, b)) => tr(b).to_string(),
            None => tr("As it is").to_string(),
        },
        Variant::Without(slots) if slots.len() == 1 => match &run.spec.test {
            Test::OneMod { name, .. } => fill(tr("{} off"), &[name]),
            _ => fill(tr("{} off"), &[&super::mod_name(s, slots[0])]),
        },
        Variant::Without(slots) => fill(tr("{} mods off"), &[&slots.len().to_string()]),
    }
}

// --- the page ---------------------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let mut out = col().key("bench-page").w_full().px(PAGE_X).pt(PAGE_TOP).pb(48.0).gap(24.0).child(back(u));
    if let Some(running) = &page.running {
        let shared = running.shared.lock().map(|s| s.clone()).ok();
        out = out.child(u.page_head("", tr("Benchmark"), tr("Running. Leave Deadlock alone until it closes by itself."), None));
        if let Some(shared) = shared {
            out = out.child(running_view(s, u, &shared));
        }
        return out;
    }
    if let Some(run) = page.showing.as_ref().and_then(|id| page.runs.iter().find(|r| &r.spec.id == id)) {
        return out.child(result_view(s, u, run));
    }
    out = out.child(u.page_head(
        "",
        tr("Benchmark"),
        tr("Plays the same replay section as fast as your PC can, and measures every frame. Compare mods, configs and drivers the same way every time."),
        None,
    ));
    out = out.child(tabs(s, u));
    if let Some((words, error)) = &page.message {
        out = out.child(line(words.clone(), if *error { u.rose } else { u.muted }));
    }
    match page.tab {
        Tab::Setup => out.child(setup(s, u)),
        Tab::History => out.child(history(s, u)),
    }
}

fn back(u: &Ui) -> El {
    row().w_full().h(32.0).mb(-16.0).center_y().child(
        row()
            .key("bench-back")
            .h(28.0)
            .pl(6.0)
            .pr(8.0)
            .ml(-6.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(4.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(tr("Back to Performance"))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                if st.performance.benchmark.showing.take().is_none() {
                    super::close(st);
                }
            })
            .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
            .child(text(tr("Performance")).font(BODY).color(u.muted).a11y_hidden()),
    )
}

fn tabs(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let count = page.runs.len();
    let items = vec![
        subpage::segment(u, tr("New run"), page.tab == Tab::Setup)
            .key("bench-tab-setup")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.performance.benchmark.tab = Tab::Setup),
        subpage::segment(u, tr("History"), page.tab == Tab::History)
            .key("bench-tab-history")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.performance.benchmark.tab = Tab::History)
            .child(text(count.to_string()).font(SMALL).color(u.muted).pl(6.0).a11y_hidden()),
    ];
    row().w_full().child(subpage::segmented(u, items).role("radiogroup").label(tr("Benchmark view")))
}

// --- setup ------------------------------------------------------------------------------------

fn radio(u: &Ui, on: bool) -> El {
    el().size(16.0, 16.0)
        .radius(8.0)
        // Orange stays for the one launch action; a picked option is ink.
        .border(if on { 5.0 } else { 1.5 }, if on { u.ink } else { u.hair_strong })
        .a11y_hidden()
}

fn option_row(u: &Ui, key: String, on: bool, label: String) -> El {
    row()
        .key(key)
        .w_full()
        .min_h(44.0)
        .px(12.0)
        .py(4.0)
        .gap(12.0)
        .center_y()
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("radio")
        .checked(on)
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(radio(u, on))
}

fn list_box(key: &str, u: &Ui) -> El {
    col().key(key.to_string()).w_full().radius(crate::ui::r(4.0)).clip_rounded(crate::ui::r(4.0)).border(1.0, u.hair)
}

fn setup(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let mut c = col().key("bench-setup").w_full().gap(24.0);
    if page.pick != Pick::Play {
        c = c.child(replays(s, u));
    }
    c = c.child(tests(s, u));
    if page.pick == Pick::OneMod {
        c = c.child(one_mod(s, u));
    }
    if page.pick != Pick::Play {
        c = c.child(options(s, u));
    }
    c.child(summary(s, u))
}

fn replays(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let choose = u
        .icon_action(&icons::FOLDER_OPEN, tr("Choose a file"), ActionKind::Ghost)
        .key("bench-pick-file")
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(|st: &mut State, tx: &mut Tx| super::pick_file(st, tx));
    let refresh = u
        .icon_button(&icons::REFRESH_CW, tr("Look for replays again"))
        .key("bench-demos-refresh")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            st.performance.benchmark.demos = None;
            super::start_reads(st);
        });
    let head = row().w_full().center_y().gap(8.0).child(u.section(tr("Replay"))).child(spacer()).child(choose).child(refresh);
    let mut c = col().w_full().gap(10.0).child(head);
    let Some(demos) = page.demos.as_ref() else {
        return c.child(line(tr("Looking for replays in the game folder and in Stats."), u.muted));
    };
    if demos.is_empty() {
        return c.child(
            col()
                .key("bench-no-replays")
                .w_full()
                .padding(20.0)
                .gap(6.0)
                .radius(crate::ui::r(4.0))
                .border(1.0, u.hair)
                .child(text(tr("No replays yet")).font(BODY_MED).color(u.ink))
                .child(line(tr("Watch any match in Deadlock once and its replay lands in the game's replays folder, where Sigil finds it. Or choose a .dem file."), u.muted)),
        );
    }
    let build = super::current_build(s);
    let chosen = super::chosen_demo(s).map(|d| d.path.clone());
    let mut list = list_box("bench-replays", u);
    for (i, d) in demos.iter().take(6).enumerate() {
        let on = chosen.as_ref() == Some(&d.path);
        let mut meta: Vec<String> = Vec::new();
        if let Some(map) = &d.map {
            meta.push(map.clone());
        }
        if let Some(sec) = d.seconds {
            meta.push(fill(tr("{} min"), &[&format!("{:.0}", sec / 60.0)]));
        }
        if let Some(b) = d.build {
            meta.push(fill(tr("build {}"), &[&b.to_string()]));
        }
        meta.push(format!("{:.0} MB", d.bytes as f64 / 1_000_000.0));
        let source = match d.source {
            DemoSource::Game => tr("In the game's replays"),
            DemoSource::Sigil => tr("From Stats"),
            DemoSource::Picked => tr("Chosen file"),
        };
        let other = d.other_build(build.as_deref());
        let path = d.path.clone();
        let mut r = option_row(u, format!("bench-demo-{i}"), on, fill(tr("Match {}"), &[&d.label]))
            .on_press(move |st: &mut State| st.performance.benchmark.chosen = Some(path.clone()))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(fill(tr("Match {}"), &[&d.label])).font(BODY_MED).color(u.ink).truncate())
                    .child(text(meta.join("  ·  ")).font(SMALL).color(u.muted).truncate()),
            );
        if other {
            r = r.child(text(tr("Other build, may not play")).font(SMALL).color(u.amber));
        }
        r = r.child(text(source).font(SMALL).color(u.muted).w(150.0).align(rune::Align::End));
        if i > 0 {
            r = r.border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        }
        list = list.child(r);
    }
    c = c.child(list);
    c.child(line(tr("Replays play only on the game build they were recorded on. After a patch, watch a new match and history starts a new line."), u.muted))
}

fn tests(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let mods = super::enabled_mods(s).len();
    let mut list = list_box("bench-tests", u);
    for (i, pick) in Pick::ALL.iter().enumerate() {
        let pick = *pick;
        let on = page.pick == pick;
        let mut detail = tr(pick.detail()).to_string();
        if pick == Pick::Cost && mods >= 2 {
            detail = format!("{detail} {}", fill(tr("About {} launches for {} mods."), &[&deadlock_core::benchmark::CostSearch::estimate(mods).to_string(), &mods.to_string()]));
        }
        let mut r = option_row(u, format!("bench-test-{i}"), on, tr(pick.title()).to_string())
            .on_press(move |st: &mut State| st.performance.benchmark.pick = pick)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(tr(pick.title())).font(BODY_MED).color(u.ink))
                    .child(text(detail).font(SMALL).color(u.muted).wrap()),
            );
        if i > 0 {
            r = r.border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        }
        list = list.child(r);
    }
    col().w_full().gap(10.0).child(u.section(tr("Test"))).child(list)
}

fn one_mod(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let mods = super::enabled_mods(s);
    let mut chips = row().key("bench-one-mod").w_full().gap(6.0).flex_wrap();
    if mods.is_empty() {
        return col().w_full().gap(10.0).child(u.section(tr("Mod"))).child(line(tr("No mods are switched on."), u.muted));
    }
    for (slot, name) in mods {
        let on = page.one_mod == Some(slot);
        chips = chips.child(
            row()
                .key(format!("bench-mod-{slot}"))
                .h(28.0)
                .px(12.0)
                .gap(6.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .border(1.0, if on { u.ink } else { u.hair_strong })
                .fill(if on { u.selected } else { Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("radio")
                .checked(on)
                .label(name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.performance.benchmark.one_mod = Some(slot))
                .child(text(name).font(BODY).color(u.ink).a11y_hidden())
                .child(text(format!("pak{slot:02}")).font(TNUM_SMALL).color(u.muted).a11y_hidden()),
        );
    }
    col().w_full().gap(10.0).child(u.section(tr("Mod"))).child(chips)
}

fn options(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    let lengths: Vec<El> = Length::ALL
        .iter()
        .map(|l| {
            let l = *l;
            subpage::segment(u, tr(l.label()), page.length == l)
                .key(format!("bench-length-{l:?}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.performance.benchmark.length = l)
        })
        .collect();
    let n = super::passes(page);
    let passes: Vec<El> = (1..=deadlock_core::benchmark::MAX_PASSES)
        .map(|p| {
            subpage::segment(u, &p.to_string(), n == p)
                .key(format!("bench-passes-{p}"))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.performance.benchmark.passes = p)
        })
        .collect();
    let displays: Vec<El> = Display::ALL
        .iter()
        .map(|d| {
            let d = *d;
            let label = match d {
                Display::Game => tr("Game settings").to_string(),
                Display::Fixed { height, .. } => format!("{height}p"),
            };
            subpage::segment(u, &label, page.display == d)
                .key(format!("bench-display-{}", match d { Display::Game => 0, Display::Fixed { height, .. } => height }))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.performance.benchmark.display = d)
        })
        .collect();
    let warm = !page.no_warmup;
    let group = |label: &str, control: El| col().gap(8.0).child(text(tr(label).to_string()).font(SMALL).color(u.muted)).child(control);
    row()
        .key("bench-options")
        .w_full()
        .gap(32.0)
        .items_start()
        .flex_wrap()
        .child(group("Section", subpage::segmented(u, lengths).role("radiogroup").label(tr("Section"))))
        .child(group("Window", subpage::segmented(u, displays).role("radiogroup").label(tr("Window"))))
        .child(group(if page.pick == Pick::Cost { "Base passes" } else { "Passes per side" }, subpage::segmented(u, passes).role("radiogroup").label(tr("Passes"))))
        .child(group(
            "Warm-up pass",
            row()
                .h(32.0)
                .gap(10.0)
                .center_y()
                .child(
                    u.toggle(warm)
                        .key("bench-warmup")
                        .label(tr("Warm-up pass"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.performance.benchmark.no_warmup = !st.performance.benchmark.no_warmup),
                )
                .child(text(tr(if warm { "First pass fills the shader cache" } else { "Off" })).font(SMALL).color(u.muted)),
        ))
}

fn summary(s: &State, u: &Ui) -> El {
    let mut c = card("bench-summary", u);
    let spec = super::spec(s);
    let blockers = super::blockers(s);
    let run = u.icon_action(&icons::PLAY, tr("Run benchmark"), ActionKind::Primary).key("bench-run").cursor(rune::Cursor::Pointer);
    match &spec {
        Ok(spec) => {
            let launches = deadlock_core::benchmark::launches(spec);
            let head = if spec.test == Test::PlaySession {
                tr("Starts Deadlock with the frame log on. Play a match, then quit the game.").to_string()
            } else {
                fill(tr("Starts Deadlock {} times, about {} minutes."), &[&launches.to_string(), &super::minutes(spec).to_string()])
            };
            let mut rows = col().gap(4.0);
            for change in super::changes(spec, true) {
                rows = rows.child(
                    row()
                        .gap(8.0)
                        .items_start()
                        .child(el().size(4.0, 4.0).mt(8.0).radius(2.0).fill(u.faint).a11y_hidden())
                        .child(text(change).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).wrap()),
                );
            }
            c = c.child(text(head).font(H2).color(u.ink)).child(rows);
        }
        Err(why) => c = c.child(text(tr(why)).font(H2).color(u.ink)),
    }
    for b in &blockers {
        c = c.child(
            row()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::TRIANGLE_ALERT, 14.0).color(u.amber).a11y_hidden())
                .child(text(b.clone()).font(SMALL).color(u.amber).wrap()),
        );
    }
    let ready = spec.is_ok() && blockers.is_empty();
    let machine = s
        .performance
        .benchmark
        .machine
        .as_ref()
        .map(|m| {
            let mut parts = Vec::new();
            if let Some(cpu) = &m.cpu {
                parts.push(cpu.clone());
            }
            if !m.gpu_line().is_empty() {
                parts.push(m.gpu_line());
            }
            parts.join("  ·  ")
        })
        .unwrap_or_default();
    let mut foot = row().w_full().gap(12.0).center_y().child(text(machine).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).truncate());
    foot = foot.child(if ready { run.on_press(super::start) } else { run.opacity(0.4) });
    c.child(u.divider()).child(foot)
}

// --- running ----------------------------------------------------------------------------------

fn running_view(s: &State, u: &Ui, shared: &Shared) -> El {
    let run = &shared.record;
    let total = match &run.cost {
        Some(cost) => shared.plan.len() + run.passes.iter().filter(|p| !p.warmup).count().max(1) + cost.next().map(|_| 1).unwrap_or(0),
        None => shared.plan.len(),
    }
    .max(shared.at as usize);
    let current = shared.current.as_ref();
    let title = current
        .map(|p| if p.warmup { tr("Warm-up").to_string() } else { variant_label(s, run, &p.variant) })
        .unwrap_or_default();
    let elapsed = clock(shared.started.elapsed().as_secs());
    let pass_secs = match &shared.step {
        super::Step::Following(live) => Some(live.elapsed.as_secs()),
        _ => None,
    };
    let mut step = row()
        .gap(10.0)
        .center_y()
        .child(icon(&icons::LOADER, 14.0).color(u.accent).a11y_hidden())
        .child(text(tr(shared.step.label())).font(BODY_MED).color(u.ink));
    if let Some(secs) = pass_secs {
        step = step.child(text(clock(secs)).font(TNUM).color(u.muted));
    }
    let mut head = card("bench-live", u)
        .child(
            row()
                .w_full()
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(text(fill(tr("Pass {} of {}"), &[&shared.at.to_string(), &total.to_string()])).font(SMALL).color(u.muted))
                        .child(text(title).font(H1).color(u.ink)),
                )
                .child(col().gap(2.0).child(text(elapsed).font(VALUE).color(u.ink).align(rune::Align::End)).child(text(tr("so far")).font(SMALL).color(u.muted).align(rune::Align::End))),
        )
        .child(step);
    if let super::Step::Following(live) = &shared.step
        && let Some(last) = &live.last_line
    {
        head = head.child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(4.0))
                .fill(u.well)
                .child(text(last.clone()).font(MONO).color(u.muted).truncate().grow(1.0).min_w(0.0)),
        );
    }
    // The strip: every pass, done ones with their FPS.
    let mut strip = row().key("bench-strip").w_full().gap(6.0).flex_wrap();
    for n in 1..=total as u32 {
        let done = run.passes.iter().find(|p| p.index == n);
        let planned = shared.plan.get(n as usize - 1);
        let is_now = n == shared.at;
        let label = match (done, planned) {
            (Some(p), _) if p.warmup => tr("Warm-up").to_string(),
            (Some(p), _) => variant_label(s, run, &p.variant),
            (None, Some(p)) if p.warmup => tr("Warm-up").to_string(),
            (None, Some(p)) => variant_label(s, run, &p.variant),
            (None, None) if is_now => title_of(s, run, current),
            _ => tr("Next").to_string(),
        };
        let value = match done {
            Some(p) if p.ok() => fps(p.summary.as_ref().map(|s| s.avg_fps).unwrap_or(0.0)),
            Some(_) => tr("failed").to_string(),
            None if is_now => tr("now").to_string(),
            None => String::new(),
        };
        let warm = done.map(|p| p.warmup).or(planned.map(|p| p.warmup)).unwrap_or(false);
        strip = strip.child(
            col()
                .key(format!("bench-strip-{n}"))
                .w(112.0)
                .h(52.0)
                .px(10.0)
                .py(8.0)
                .gap(2.0)
                .radius(crate::ui::r(4.0))
                .border(1.0, if is_now { u.accent } else { u.hair })
                .fill(if done.is_some() { u.well } else { Color::TRANSPARENT })
                .opacity(if done.is_none() && !is_now { 0.55 } else { 1.0 })
                .child(text(label).font(SMALL).color(if warm { u.muted } else { u.muted }).truncate())
                .child(text(value).font(TNUM_MED).color(if is_now { u.accent } else { u.ink })),
        );
    }
    let mut c = col().key("bench-running").w_full().gap(16.0).child(head).child(strip);
    if let Some(cmp) = run.comparison() {
        c = c.child(line(
            fill(tr("So far: {} {} FPS, {} {} FPS."), &[tr(&cmp.a.label), &fps(cmp.a.summary.avg_fps), &super::lower(tr(&cmp.b.label)), &fps(cmp.b.summary.avg_fps)]),
            u.muted,
        ));
    }
    c.child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(line(tr("Stop closes Deadlock and puts the cfg, your mods and autoexec.cfg back. Passes done so far are kept."), u.muted).grow(1.0).min_w(0.0))
            .child(
                u.icon_action(&icons::SQUARE_STOP, tr("Stop"), ActionKind::Secondary)
                    .key("bench-stop")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(super::stop),
            ),
    )
}

fn title_of(s: &State, run: &RunRecord, p: Option<&deadlock_core::benchmark::PassPlan>) -> String {
    p.map(|p| variant_label(s, run, &p.variant)).unwrap_or_default()
}

// --- result -----------------------------------------------------------------------------------

fn result_view(s: &State, u: &Ui, run: &RunRecord) -> El {
    let id = run.spec.id.clone();
    let mut sub: Vec<String> = Vec::new();
    if let Some(d) = &run.spec.demo {
        sub.push(fill(tr("Match {}"), &[&d.label]));
        if let Some(map) = &d.map {
            sub.push(map.clone());
        }
        sub.push(tr(run.spec.length.label()).to_string());
    }
    sub.push(when(&run.spec.created_at));
    let (a, b, c, d) = (id.clone(), id.clone(), id.clone(), id.clone());
    // A card needs a measured pass: a run that failed before one has nothing to draw.
    let measured = run.passes.iter().any(|p| !p.warmup && p.summary.is_some());
    let mut actions = row().gap(8.0).center_y();
    if measured {
        actions = actions.child(
            u.icon_action(&icons::DOWNLOAD, tr("Save card"), ActionKind::Secondary)
                .key("bench-card")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| super::save_card(st, tx, &a)),
        );
    }
    let actions = actions
        .child(u.action(tr("JSON"), ActionKind::Ghost).key("bench-json").cursor(rune::Cursor::Pointer).on_press_tx(move |st: &mut State, tx: &mut Tx| super::export_json(st, tx, &b)))
        .child(u.action(tr("CSV"), ActionKind::Ghost).key("bench-csv").cursor(rune::Cursor::Pointer).on_press_tx(move |st: &mut State, tx: &mut Tx| super::export_csv(st, tx, &c)))
        .child(u.icon_button(&icons::FOLDER_OPEN, tr("Open logs")).key("bench-logs").cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| super::open_logs(st, &d)))
        .child(
            u.icon_button(&icons::TRASH_2, tr("Delete this run"))
                .key("bench-delete")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.performance.benchmark.deleting = Some(id.clone())),
        );
    let mut out = col()
        .key("bench-result")
        .w_full()
        .gap(24.0)
        .child(u.page_head("", tr(run.spec.test.label()), &sub.join("  ·  "), Some(actions)));
    if s.performance.benchmark.deleting.as_deref() == Some(run.spec.id.as_str()) {
        out = out.child(delete_confirm(u, &run.spec.id));
    }
    if let Some((words, error)) = &s.performance.benchmark.message {
        out = out.child(line(words.clone(), if *error { u.rose } else { u.muted }));
    }
    match &run.outcome {
        Some(Outcome::Failed(why)) if run.headline().is_none() => {
            return out.child(
                card("bench-failed", u)
                    .child(row().gap(8.0).center_y().child(icon(&icons::CIRCLE_X, 16.0).color(u.rose).a11y_hidden()).child(text(tr("The run did not measure")).font(H2).color(u.ink)))
                    .child(line(why.clone(), u.muted))
                    .child(line(tr("Nothing is left changed. The cfg, your mods and autoexec.cfg are as they were."), u.muted)),
            )
            .child(passes_table(s, u, run));
        }
        Some(Outcome::Stopped) => out = out.child(line(tr("Stopped before the end. The result uses the passes that finished."), u.amber)),
        Some(Outcome::Failed(why)) => out = out.child(line(fill(tr("Ended early: {}"), &[why]), u.amber)),
        _ => {}
    }
    if let Some(head) = run.headline() {
        out = out.child(headline(u, &head.summary, head.spread, head.passes));
    }
    if let Some(cmp) = run.comparison() {
        out = out.child(comparison(u, &cmp, None));
    }
    if run.cost.is_some() {
        out = out.child(cost(s, u, run));
    }
    out = out.child(graph_card(s, u, run));
    out.child(passes_table(s, u, run)).child(ran_on(u, run)).child(notes(s, u, run))
}

/// Notes on a finished run: appended, never rewritten.
fn notes(s: &State, u: &Ui, run: &RunRecord) -> El {
    let mut c = col().key("bench-notes").w_full().gap(10.0).child(u.section(tr("Notes")));
    for note in &run.notes {
        c = c.child(
            row()
                .w_full()
                .gap(16.0)
                .items_start()
                .child(text(when(&note.at)).font(TNUM).color(u.muted).w(110.0))
                .child(text(note.text.clone()).font(BODY).color(u.prose).grow(1.0).min_w(0.0).wrap()),
        );
    }
    let id = run.spec.id.clone();
    let enter_id = id.clone();
    let field = rune::text_input(&s.performance.benchmark.note, BODY)
        .key("bench-note-field")
        .single_line()
        .label(tr("Add a note"))
        .grow(1.0)
        .min_w(0.0)
        .color(u.ink)
        .placeholder(tr("Add a note: what you changed, what you noticed"), u.faint)
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(|st: &mut State| &mut st.performance.benchmark.note))
        .on_submit(move |st: &mut State, _tx: &mut Tx| super::add_note(st, enter_id.clone()));
    c.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(row().h(32.0).px(10.0).grow(1.0).min_w(0.0).center_y().radius(crate::ui::r(4.0)).fill(u.well).border(1.0, u.hair).child(field))
            .child(u.action(tr("Add note"), ActionKind::Secondary).key("bench-note-add").cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| super::add_note(st, id.clone()))),
    )
}

fn delete_confirm(u: &Ui, id: &str) -> El {
    let id = id.to_string();
    row()
        .key("bench-delete-confirm")
        .w_full()
        .gap(12.0)
        .center_y()
        .padding(12.0)
        .radius(crate::ui::r(4.0))
        .border(1.0, u.rose.alpha(0.3))
        .child(text(tr("Delete this run and its frames?")).font(BODY).color(u.ink).grow(1.0))
        .child(u.action(tr("Keep"), ActionKind::Ghost).key("bench-delete-no").cursor(rune::Cursor::Pointer).on_press(|st: &mut State| st.performance.benchmark.deleting = None))
        .child(u.action(tr("Delete"), ActionKind::Danger).key("bench-delete-yes").cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| super::delete(st, id.clone())))
}

fn stat(u: &Ui, value: String, label: &str) -> El {
    col().w(120.0).gap(2.0).child(text(value).font(VALUE).color(u.ink)).child(text(tr(label).to_string()).font(SMALL).color(u.muted))
}

fn headline(u: &Ui, s: &Summary, spread: Option<(f64, f64)>, passes: usize) -> El {
    let spread_line = match spread {
        Some((lo, hi)) if passes > 1 => fill(tr("median of {} passes, {} to {}"), &[&passes.to_string(), &fps(lo), &fps(hi)]),
        _ => tr("one pass").to_string(),
    };
    row()
        .key("bench-headline")
        .w_full()
        .gap(40.0)
        .items_end()
        .flex_wrap()
        .child(
            col()
                .gap(0.0)
                .child(row().gap(8.0).items_end().child(text(fps(s.avg_fps)).font(BIG).color(u.ink)).child(text("FPS").font(H2).color(u.muted).pb(8.0)))
                .child(text(spread_line).font(SMALL).color(u.muted)),
        )
        .child(
            row()
                .gap(8.0)
                .pb(18.0)
                .flex_wrap()
                .child(stat(u, opt(s.low_1, fps), "1% low"))
                .child(stat(u, opt(s.low_01, fps), "0.1% low"))
                .child(stat(u, opt(s.p50_ms, ms), "Median frame"))
                .child(stat(u, opt(s.p99_ms, ms), "p99 frame"))
                .child(stat(u, s.stutters.map(|n| n.to_string()).unwrap_or_else(|| "-".into()), "Stutters")),
        )
}

fn verdict_line(cmp: &Comparison) -> (String, Verdict) {
    let pct = format!("{:.1}", (cmp.gap_share * 100.0).abs());
    let gap = format!("{:+.1}", cmp.gap_fps);
    let (a, b) = (super::lower(tr(&cmp.a.label)), tr(&cmp.b.label).to_string());
    let words = match cmp.verdict {
        Verdict::Faster => fill(tr("{} is {}% faster than {} ({} FPS)."), &[&b, &pct, &a, &gap]),
        Verdict::Slower => fill(tr("{} is {}% slower than {} ({} FPS)."), &[&b, &pct, &a, &gap]),
        Verdict::Same => fill(tr("{} and {} are the same, inside the noise."), &[&b, &a]),
    };
    (words, cmp.verdict)
}

/// The comparison card. `changed` lists what differs between two runs (history's compare).
fn comparison(u: &Ui, cmp: &Comparison, changed: Option<El>) -> El {
    let (words, verdict) = verdict_line(cmp);
    let tone = match verdict {
        Verdict::Faster => u.teal,
        Verdict::Slower => u.rose,
        Verdict::Same => u.muted,
    };
    let mut noise = fill(tr("Noise: plus or minus {} FPS."), &[&format!("{:.1}", cmp.noise_fps)]);
    if cmp.low_confidence {
        noise = format!("{noise} {}", tr("One pass on a side, so the noise is estimated. Run more passes to be sure."));
    }
    let mut table = col().w_full().gap(0.0).child(
        row()
            .w_full()
            .h(28.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(text(tr("Metric")).font(SMALL).color(u.muted).grow(1.0))
            .child(text(tr(&cmp.a.label).to_string()).font(SMALL).color(u.muted).w(120.0).align(rune::Align::End))
            .child(text(tr(&cmp.b.label).to_string()).font(SMALL).color(u.muted).w(120.0).align(rune::Align::End))
            .child(text(tr("Change")).font(SMALL).color(u.muted).w(100.0).align(rune::Align::End)),
    );
    for m in &cmp.metrics {
        let (label, format): (&str, fn(f64) -> String) = match m.key.as_str() {
            "avgFps" => ("Average FPS", fps),
            "low1" => ("1% low", fps),
            "low01" => ("0.1% low", fps),
            "p50" => ("Median frame", ms),
            "p95" => ("p95 frame", ms),
            "p99" => ("p99 frame", ms),
            _ => ("Stutters", |v| format!("{v:.0}")),
        };
        if m.a.is_none() && m.b.is_none() {
            continue;
        }
        let change = m.change.map(|c| format!("{:+.1}%", c * 100.0)).unwrap_or_else(|| "-".into());
        let color = match m.b_better() {
            Some(true) if verdict != Verdict::Same => u.teal,
            Some(false) if verdict != Verdict::Same => u.rose,
            _ => u.muted,
        };
        table = table.child(
            row()
                .w_full()
                .h(32.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(tr(label).to_string()).font(BODY).color(u.prose).grow(1.0))
                .child(text(opt(m.a, format)).font(TNUM).color(u.prose).w(120.0).align(rune::Align::End))
                .child(text(opt(m.b, format)).font(TNUM_MED).color(u.ink).w(120.0).align(rune::Align::End))
                .child(text(change).font(TNUM).color(color).w(100.0).align(rune::Align::End)),
        );
    }
    let mut c = card("bench-compare", u)
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(el().w(4.0).h(24.0).radius(crate::ui::r(2.0)).fill(tone).a11y_hidden())
                .child(text(words).font(H1).color(u.ink).wrap().grow(1.0).min_w(0.0)),
        )
        .child(line(noise, u.muted))
        .child(table);
    if let Some(changed) = changed {
        c = c.child(changed);
    }
    c
}

fn cost(s: &State, u: &Ui, run: &RunRecord) -> El {
    let Some(search) = &run.cost else { return el() };
    let mut c = card("bench-cost", u);
    let name = |slot: u16| {
        run.settings.mods.iter().find(|m| m.0 == slot).map(|m| m.1.clone()).unwrap_or_else(|| super::mod_name(s, slot))
    };
    c = c.child(match &search.end {
        Some(CostEnd::Found { slot }) => {
            let gain = search.groups().into_iter().find(|g| g.0 == vec![*slot]).map(|g| g.1);
            col()
                .gap(4.0)
                .child(text(fill(tr("{} costs the most"), &[&name(*slot)])).font(H1).color(u.ink))
                .child(line(
                    match gain {
                        Some(g) => fill(tr("Switching it off gained {} FPS on its own. pak{} in Installed."), &[&format!("{g:.1}"), &format!("{slot:02}")]),
                        None => fill(tr("pak{} in Installed."), &[&format!("{slot:02}")]),
                    },
                    u.muted,
                ))
        }
        Some(CostEnd::NothingMeasurable) => col()
            .gap(4.0)
            .child(text(tr("No mod stands out")).font(H1).color(u.ink))
            .child(line(tr("No group of mods cost more than the noise. Your mods are light, or their cost is spread evenly."), u.muted)),
        None => col().child(text(tr("The search did not finish")).font(H1).color(u.ink)),
    });
    let mut table = col().w_full();
    for (slots, gain) in search.groups() {
        let names: Vec<String> = slots.iter().map(|s| name(*s)).collect();
        let label = if names.len() > 3 { fill(tr("{} and {} more"), &[&names[..2].join(", "), &(names.len() - 2).to_string()]) } else { names.join(", ") };
        let share = if search.levels.first().and_then(|l| l.base).unwrap_or(0.0) > 0.0 { (gain / search.levels[0].base.unwrap_or(1.0) * 100.0).max(0.0) } else { 0.0 };
        table = table.child(
            row()
                .w_full()
                .h(34.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(text(if slots.len() == 1 { tr("1 mod").to_string() } else { fill(tr("{} mods"), &[&slots.len().to_string()]) }).font(TNUM_SMALL).color(u.muted).w(56.0))
                .child(text(label).font(BODY).color(u.prose).grow(1.0).min_w(0.0).truncate())
                .child(
                    row().w(120.0).h(6.0).radius(crate::ui::r(3.0)).fill(u.hair).child(el().w((share.min(20.0) / 20.0 * 120.0) as f32).h_full().radius(crate::ui::r(3.0)).fill(if gain > 0.0 { u.rose } else { u.faint })),
                )
                .child(text(format!("{:+.1} FPS", gain)).font(TNUM).color(u.ink).w(90.0).align(rune::Align::End)),
        );
    }
    c.child(text(tr("FPS gained by switching each group off")).font(SMALL).color(u.muted)).child(table)
}

/// Frame time across the section: bins of each side's first measured pass.
fn graph_card(s: &State, u: &Ui, run: &RunRecord) -> El {
    let frames = super::graph_frames(s, run);
    let mut c = card("bench-graph", u);
    let two = frames.len() > 1;
    let mut head = row().w_full().gap(16.0).center_y().child(text(tr("Frame time")).font(H2).color(u.ink)).child(spacer());
    if two && let Some((a, b)) = run.spec.test.sides() {
        for (label, color) in [(a, u.muted), (b, u.accent)] {
            head = head.child(row().gap(6.0).center_y().child(el().size(8.0, 8.0).radius(crate::ui::r(2.0)).fill(color).a11y_hidden()).child(text(tr(label).to_string()).font(SMALL).color(u.muted)));
        }
    }
    c = c.child(head);
    if frames.is_empty() {
        let why = match run.source() {
            Some(deadlock_core::benchmark::session::FrameSource::SummaryOnly) => tr("The game wrote only its summary line for this run, so there is no frame graph. Average FPS and percentiles come from that line."),
            _ => tr("No frames were stored for this run."),
        };
        return c.child(line(why, u.muted));
    }
    let series: Vec<(Color, Vec<Bin>)> = frames
        .iter()
        .map(|(side, f)| (if two && *side == Side::A { u.muted } else { u.accent }, super::bins(f)))
        .collect();
    c = c.child(graph(u, &series));
    let note = match run.source() {
        Some(src) => fill(tr("Each point is about {} seconds. Spikes are the slowest frame in that slice. Source: {}."), &[&format!("{:.1}", run.spec.length.seconds().unwrap_or(90) as f64 / 300.0), &super::lower(tr(src.label()))]),
        None => String::new(),
    };
    c.child(line(note, u.muted))
}

fn graph(u: &Ui, series: &[(Color, Vec<Bin>)]) -> El {
    let top = series
        .iter()
        .flat_map(|(_, bins)| bins.iter().map(|b| b.worst_ms.min(1000.0 / b.fps * 3.0)))
        .fold(1.0f64, f64::max)
        * 1.1;
    let step = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0].into_iter().find(|s| top / s <= 5.0).unwrap_or(100.0);
    let y = |v: f64| GRAPH_H - (v / top) as f32 * (GRAPH_H - 8.0);
    let mut shape = Shape::new().view_box(GRAPH_VB, GRAPH_H);
    let mut labels = el().w(AXIS_W).h(GRAPH_H);
    let mut tick = step;
    while tick < top {
        let yy = y(tick);
        shape = shape.line([0.0, yy], [GRAPH_VB, yy], 1.0, rune::LineCap::Butt, u.over.alpha(0.05));
        labels = labels.child(text(format!("{tick:.0} ms")).font(TNUM_SMALL).color(u.muted).absolute(0.0, yy - 7.0));
        tick += step;
    }
    shape = shape.line([0.0, GRAPH_H - 0.5], [GRAPH_VB, GRAPH_H - 0.5], 1.0, rune::LineCap::Butt, u.hair_strong);
    let last = series.len().saturating_sub(1);
    for (i, (color, bins)) in series.iter().enumerate() {
        if bins.len() < 2 {
            continue;
        }
        let total = bins.last().map(|b| b.at).unwrap_or(1.0).max(1e-6);
        let x = |b: &Bin| (b.at / total) as f32 * GRAPH_VB;
        let points: Vec<[f32; 2]> = bins.iter().map(|b| [x(b), y(1000.0 / b.fps)]).collect();
        if i == last {
            // Spikes: the slowest frame of each slice, where it stands out.
            for b in bins {
                if b.worst_ms > 1000.0 / b.fps * 1.8 {
                    shape = shape.line([x(b), y(1000.0 / b.fps)], [x(b), y(b.worst_ms.min(top))], 1.5, rune::LineCap::Round, color.alpha(0.45));
                }
            }
            let mut area = points.clone();
            area.push([GRAPH_VB, GRAPH_H]);
            area.push([0.0, GRAPH_H]);
            shape = shape.fill(area, color.alpha(0.10));
        }
        shape = shape.stroke(points, if i == last { 1.75 } else { 1.25 }, if i == last { *color } else { color.alpha(0.8) });
    }
    row().key("bench-graph-plot").w_full().child(labels).child(canvas(shape).grow(1.0).min_w(0.0).h(GRAPH_H)).role("img").label(tr("Frame time across the section"))
}

fn passes_table(s: &State, u: &Ui, run: &RunRecord) -> El {
    let mut t = col().key("bench-passes").w_full().gap(10.0).child(u.section(tr("Passes")));
    let mut rows = col().w_full();
    for p in &run.passes {
        let sm = p.summary.clone().unwrap_or_default();
        let label = if p.warmup { tr("Warm-up").to_string() } else { variant_label(s, run, &p.variant) };
        let mut r = row()
            .key(format!("bench-pass-{}", p.index))
            .w_full()
            .min_h(34.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .opacity(if p.warmup { 0.6 } else { 1.0 })
            .child(text(p.index.to_string()).font(TNUM_SMALL).color(u.muted).w(20.0))
            .child(text(label).font(BODY).color(u.prose).w(170.0).truncate());
        if let Some(error) = &p.error {
            r = r.child(text(error.clone()).font(SMALL).color(u.rose).grow(1.0).min_w(0.0).wrap());
        } else {
            r = r
                .child(text(format!("{} FPS", fps(sm.avg_fps))).font(TNUM_MED).color(u.ink).w(90.0).align(rune::Align::End))
                .child(text(opt(sm.low_1, fps)).font(TNUM).color(u.prose).w(70.0).align(rune::Align::End))
                .child(text(opt(sm.p99_ms, ms)).font(TNUM).color(u.prose).w(80.0).align(rune::Align::End))
                .child(text(fill(tr("{} frames"), &[&sm.frames.to_string()])).font(TNUM_SMALL).color(u.muted).grow(1.0).align(rune::Align::End))
                .child(text(p.fps_cap.filter(|c| *c > 0.0).map(|c| fill(tr("capped at {}"), &[&fps(c)])).unwrap_or_default()).font(SMALL).color(u.amber).w(100.0).align(rune::Align::End));
        }
        rows = rows.child(r);
    }
    let head = row()
        .w_full()
        .h(24.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(el().w(20.0))
        .child(text(tr("Pass")).font(SMALL).color(u.muted).w(170.0))
        .child(text(tr("Average")).font(SMALL).color(u.muted).w(90.0).align(rune::Align::End))
        .child(text(tr("1% low")).font(SMALL).color(u.muted).w(70.0).align(rune::Align::End))
        .child(text(tr("p99")).font(SMALL).color(u.muted).w(80.0).align(rune::Align::End))
        .child(spacer())
        .child(el().w(100.0));
    t = t.child(col().w_full().child(head).child(rows));
    t.child(line(tr("The warm-up pass fills the shader cache and is left out of the result."), u.muted))
}

fn ran_on(u: &Ui, run: &RunRecord) -> El {
    let m = &run.machine;
    let st = &run.settings;
    let mut left = col().grow(1.0).min_w(260.0);
    let mut right = col().grow(1.0).min_w(260.0);
    let kv = |k: &str, v: String| u.kv(tr(k), &v, u.prose);
    left = left
        .child(kv("Processor", m.cpu.clone().unwrap_or_else(|| "-".into())))
        .child(kv("Graphics", m.gpu.clone().unwrap_or_else(|| "-".into())))
        .child(kv("Driver", [m.driver.clone(), m.driver_date.clone()].into_iter().flatten().collect::<Vec<_>>().join("  ·  ")))
        .child(kv("Memory", m.ram_mb.map(|r| format!("{:.0} GB", r as f64 / 1024.0)).unwrap_or_else(|| "-".into())))
        .child(kv("System", m.os.clone()));
    let res = match (st.resolution(), st.window_mode()) {
        (Some(r), Some(w)) => format!("{r}  ·  {}", tr(w)),
        (Some(r), None) => r,
        _ => "-".into(),
    };
    let cap = run.passes.iter().find_map(|p| p.fps_cap).map(|c| if c > 0.0 { fps(c) } else { tr("none").to_string() }).unwrap_or_else(|| "-".into());
    right = right
        .child(kv("Game build", st.build.clone().unwrap_or_else(|| "-".into())))
        .child(kv("Resolution", res))
        .child(kv("Video settings", fill(tr("{} lines, fingerprint {}"), &[&st.video.len().to_string(), &st.video_fingerprint()])))
        .child(kv("Config", st.performance_config.clone().unwrap_or_else(|| tr("Stock").to_string())))
        .child(kv("Profile", st.profile.clone().unwrap_or_else(|| "-".into())))
        .child(kv("Mods on", fill(tr("{}, fingerprint {}"), &[&st.mods.len().to_string(), &st.mods_fingerprint()])))
        .child(kv("Frame cap", cap));
    col().key("bench-ran-on").w_full().gap(10.0).child(u.section(tr("Ran on"))).child(row().w_full().gap(32.0).items_start().flex_wrap().child(left).child(right))
}

// --- history ----------------------------------------------------------------------------------

fn history(s: &State, u: &Ui) -> El {
    let page = &s.performance.benchmark;
    if !page.runs_loaded {
        return line(tr("Reading stored runs."), u.muted);
    }
    if page.runs.is_empty() {
        return u.empty_state("", tr("No runs yet"), tr("Every run is kept here, with the machine and settings it ran on."), None);
    }
    let mut c = col().key("bench-history").w_full().gap(24.0);
    if let Some(compare) = compare_panel(u, page) {
        c = c.child(compare);
    } else {
        c = c.child(line(tr("Tick two runs to compare them: before and after a driver, a config, a profile or a mod."), u.muted));
    }
    let mut series: BTreeMap<String, Vec<&RunRecord>> = BTreeMap::new();
    for run in &page.runs {
        series.entry(run.series_key()).or_default().push(run);
    }
    let mut groups: Vec<(String, Vec<&RunRecord>)> = series.into_iter().collect();
    groups.sort_by(|a, b| b.1[0].spec.id.cmp(&a.1[0].spec.id));
    for (key, runs) in groups {
        c = c.child(series_group(s, u, &key, &runs));
    }
    c
}

fn compare_panel(u: &Ui, page: &Page) -> Option<El> {
    if page.picked.len() != 2 {
        return None;
    }
    let a = page.runs.iter().find(|r| r.spec.id == page.picked[0])?;
    let b = page.runs.iter().find(|r| r.spec.id == page.picked[1])?;
    let cmp = store::compare_runs(a, b)?;
    let mut diffs: Vec<(String, String, String)> = Vec::new();
    let mut add = |k: &str, va: String, vb: String| {
        if va != vb {
            diffs.push((k.to_string(), va, vb));
        }
    };
    add(tr("Game build"), a.settings.build.clone().unwrap_or_default(), b.settings.build.clone().unwrap_or_default());
    add(tr("Driver"), a.machine.gpu_line(), b.machine.gpu_line());
    add(tr("Config"), a.settings.performance_config.clone().unwrap_or_default(), b.settings.performance_config.clone().unwrap_or_default());
    add(tr("Profile"), a.settings.profile.clone().unwrap_or_default(), b.settings.profile.clone().unwrap_or_default());
    for (key, va, vb) in system::video_diff(&a.settings, &b.settings) {
        add(&format!("setting.{key}"), va.unwrap_or_else(|| "-".into()), vb.unwrap_or_else(|| "-".into()));
    }
    let off: Vec<&String> = a.settings.mods.iter().filter(|m| !b.settings.mods.contains(m)).map(|m| &m.1).collect();
    let on: Vec<&String> = b.settings.mods.iter().filter(|m| !a.settings.mods.contains(m)).map(|m| &m.1).collect();
    for name in off {
        add(&fill(tr("Mod {}"), &[name]), tr("on").to_string(), tr("off").to_string());
    }
    for name in on {
        add(&fill(tr("Mod {}"), &[name]), tr("off").to_string(), tr("on").to_string());
    }
    let mut changed = col().w_full().gap(4.0).child(text(tr("What changed")).font(BODY_MED).color(u.ink));
    if diffs.is_empty() {
        changed = changed.child(line(tr("Same build, driver, settings and mods. The gap is the game or the PC on the day."), u.muted));
    }
    for (k, va, vb) in diffs {
        changed = changed.child(
            row()
                .w_full()
                .gap(12.0)
                .child(text(k).font(MONO).color(u.muted).w(220.0).truncate())
                .child(text(va).font(MONO).color(u.muted).grow(1.0).min_w(0.0).truncate())
                .child(icon(&icons::CHEVRON_RIGHT, 12.0).color(u.muted).a11y_hidden())
                .child(text(vb).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate()),
        );
    }
    if a.series_key() != b.series_key() {
        changed = changed.child(line(tr("These runs played different replays or builds, so the numbers are not like for like."), u.amber));
    }
    Some(
        col()
            .w_full()
            .gap(8.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(u.section(tr("Compare")))
                    .child(spacer())
                    .child(u.action(tr("Clear"), ActionKind::Ghost).key("bench-compare-clear").cursor(rune::Cursor::Pointer).on_press(|st: &mut State| st.performance.benchmark.picked.clear())),
            )
            .child(comparison(u, &cmp, Some(changed))),
    )
}

fn series_group(s: &State, u: &Ui, key: &str, runs: &[&RunRecord]) -> El {
    let first = runs[0];
    let title = match &first.spec.demo {
        Some(d) => fill(tr("Match {} on build {}"), &[&d.label, first.settings.build.as_deref().unwrap_or("?")]),
        None => tr("Play sessions").to_string(),
    };
    let mut c = col().key(format!("bench-series-{key}")).w_full().gap(10.0).child(
        row()
            .w_full()
            .center_y()
            .child(text(title).font(H2).color(u.ink))
            .child(spacer())
            .child(text(fill(tr("{} runs"), &[&runs.len().to_string()])).font(SMALL).color(u.muted)),
    );
    let trend: Vec<&RunRecord> = runs.iter().rev().copied().filter(|r| r.cost.is_none() && r.headline().is_some()).collect();
    if trend.len() >= 2 {
        c = c.child(trend_chart(u, &trend));
    }
    let mut list = list_box(&format!("bench-runs-{key}"), u);
    for (i, run) in runs.iter().enumerate() {
        let id = run.spec.id.clone();
        let picked = s.performance.benchmark.picked.contains(&id);
        let head = run.headline();
        let value = head.as_ref().map(|h| format!("{} FPS", fps(h.summary.avg_fps))).unwrap_or_else(|| tr("no result").to_string());
        let low = head.as_ref().and_then(|h| h.summary.low_1).map(|v| fill(tr("1% low {}"), &[&fps(v)])).unwrap_or_default();
        let extra = match run.comparison() {
            Some(cmp) => verdict_line(&cmp).0,
            None => run.machine.driver.clone().map(|d| fill(tr("driver {}"), &[&d])).unwrap_or_default(),
        };
        let pick_id = id.clone();
        let mut r = row()
            .key(format!("bench-run-{id}"))
            .w_full()
            .h(44.0)
            .px(12.0)
            .gap(12.0)
            .center_y()
            .fill(if picked { u.selected } else { Color::TRANSPARENT })
            .hover(u.hover)
            .child(
                el().size(28.0, 28.0)
                    .center()
                    .radius(crate::ui::r(4.0))
                    .hover(u.hover)
                    .focusable(true)
                    .role("checkbox")
                    .checked(picked)
                    .label(tr("Compare this run"))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::toggle_pick(st, pick_id.clone()))
                    .child(icon(if picked { &icons::SQUARE_CHECK } else { &icons::SQUARE }, 16.0).color(if picked { u.ink } else { u.muted }).a11y_hidden()),
            )
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .h_full()
                    .gap(12.0)
                    .center_y()
                    .focusable(true)
                    .role("button")
                    .label(run_label(run))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| super::show(st, id.clone()))
                    .child(text(when(&run.spec.created_at)).font(TNUM).color(u.muted).w(110.0))
                    .child(text(run_label(run)).font(BODY_MED).color(u.ink).w(200.0).truncate())
                    .child(text(value).font(TNUM_MED).color(u.ink).w(80.0).align(rune::Align::End))
                    .child(text(low).font(TNUM).color(u.muted).w(90.0).align(rune::Align::End))
                    .child(text(extra).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).truncate())
                    .child(icon(&icons::CHEVRON_RIGHT, 14.0).color(u.muted).a11y_hidden()),
            );
        if i > 0 {
            r = r.border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        }
        list = list.child(r);
    }
    c.child(list)
}

/// Average FPS over time, with a tick where the driver or the build changed.
fn trend_chart(u: &Ui, runs: &[&RunRecord]) -> El {
    const H: f32 = 96.0;
    let values: Vec<f64> = runs.iter().filter_map(|r| r.headline().map(|h| h.summary.avg_fps)).collect();
    let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let pad = ((hi - lo) * 0.25).max(2.0);
    let (lo, hi) = (lo - pad, hi + pad);
    let n = values.len();
    let x = |i: usize| if n > 1 { 12.0 + i as f32 / (n - 1) as f32 * (GRAPH_VB - 24.0) } else { GRAPH_VB / 2.0 };
    let y = |v: f64| 8.0 + ((hi - v) / (hi - lo)) as f32 * (H - 16.0);
    let mut shape = Shape::new().view_box(GRAPH_VB, H);
    let mut notes: Vec<String> = Vec::new();
    for i in 1..n {
        let (a, b) = (runs[i - 1], runs[i]);
        let driver = a.machine.driver != b.machine.driver;
        let build = a.settings.build != b.settings.build;
        if driver || build {
            let xx = (x(i - 1) + x(i)) / 2.0;
            let mut yy = 0.0;
            while yy < H {
                shape = shape.line([xx, yy], [xx, (yy + 4.0).min(H)], 1.0, rune::LineCap::Butt, u.amber.alpha(0.6));
                yy += 8.0;
            }
            if driver {
                notes.push(fill(tr("Driver {} from {}"), &[b.machine.driver.as_deref().unwrap_or("?"), &when(&b.spec.created_at)]));
            }
            if build {
                notes.push(fill(tr("Build {} from {}"), &[b.settings.build.as_deref().unwrap_or("?"), &when(&b.spec.created_at)]));
            }
        }
    }
    let points: Vec<[f32; 2]> = values.iter().enumerate().map(|(i, v)| [x(i), y(*v)]).collect();
    shape = shape.stroke(points.clone(), 1.5, u.accent);
    for p in &points {
        shape = shape.fill(circle(*p, 3.0), u.accent);
    }
    let mut c = col().w_full().gap(6.0).child(
        row()
            .w_full()
            .child(
                col()
                    .w(AXIS_W)
                    .h(H)
                    .child(text(fps(hi - pad)).font(TNUM_SMALL).color(u.muted))
                    .child(spacer())
                    .child(text(fps(lo + pad)).font(TNUM_SMALL).color(u.muted)),
            )
            .child(canvas(shape).grow(1.0).min_w(0.0).h(H))
            .role("img")
            .label(tr("Average FPS over time")),
    );
    for note in notes {
        c = c.child(
            row()
                .pl(AXIS_W)
                .gap(8.0)
                .center_y()
                .child(el().w(10.0).h(2.0).fill(u.amber.alpha(0.7)).a11y_hidden())
                .child(text(note).font(SMALL).color(u.muted)),
        );
    }
    c
}

fn circle(center: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    (0..12)
        .map(|i| {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
        })
        .collect()
}
