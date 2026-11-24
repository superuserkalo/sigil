//! A config's page: what it changes, its author switches, the in-game video
//! settings it was tuned for, what Sigil leaves out, and where it comes from.
//!
//! Read only. Applying and turning off are dialogs on the list page; the convar
//! editor is `editor`, the GameBanana page is `hosted`.

use super::*;

/// The big numbers in the summary, in tabular figures.
const BIG_NUM: rune::TextStyle =
    rune::TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.2).features("'tnum' 1");

// --- detail -----------------------------------------------------------------

fn back(u: &Ui) -> El {
    row().w_full().h(32.0).center_y().child(
        row()
            .key("perf-back")
            .h(28.0)
            .pl(6.0)
            .pr(8.0)
            .ml(-6.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label(tr("Back to Performance"))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.performance.open = None)
            .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
            .child(text(tr("Performance")).font(BODY).color(u.muted).a11y_hidden()),
    )
}

pub(super) fn page(s: &State, u: &Ui, config: &Config) -> El {
    let options = options_for(s, config);
    let id = config.id().to_string();
    let is_on = applied_id(s).as_deref() == Some(config.id());
    let busy = s.performance.write_job.is_running();
    let foreign = matches!(applied(s), Some(Applied::Foreign));

    // Turn off (only when on), Edit settings, then Apply in a fixed slot at the right edge:
    // its label changes in place, so the other buttons never move.
    let mut actions = row().gap(8.0).center_y();
    if is_on {
        actions = actions.child(
            u.icon_action(&icons::POWER, tr("Turn off"), ActionKind::Secondary)
                .key("perf-detail-off")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if !st.performance.write_job.is_running() {
                        st.performance.dialog = Some(Dialog::TurnOff);
                    }
                }),
        );
    }
    actions = actions.child(edit_button(u, config));
    let changed = pending(s, config);
    let newer = is_on && update_for(s).is_some();
    let applied_now = is_on && !changed && !newer;
    let label = if applied_now {
        tr("Applied")
    } else if is_on && changed {
        tr("Apply changes")
    } else if newer {
        tr("Update")
    } else {
        tr("Apply")
    };
    let blocked = busy || foreign || applied_now;
    let apply_id = id.clone();
    let mut apply = row()
        .key("perf-detail-apply")
        .w(132.0)
        .h(32.0)
        .gap(6.0)
        .center()
        .radius(crate::ui::r(4.0))
        .fill(if applied_now { u.hover } else { u.accent })
        .opacity(if busy || foreign { 0.4 } else { 1.0 })
        .focusable(!blocked)
        .role("button")
        .disabled(blocked)
        .label(label.to_string());
    if applied_now {
        apply = apply.child(el().size(8.0, 8.0).radius(1.0).fill(crate::ui::switch_green()).a11y_hidden());
    }
    apply = apply.child(
        text(label.to_string())
            .font(BODY_MED)
            .color(if applied_now { u.prose } else { u.accent_ink })
            .a11y_hidden(),
    );
    if !blocked {
        apply = apply.cursor(rune::Cursor::Pointer).on_press(move |st: &mut State| {
            let blocked = st.performance.write_job.is_running()
                || matches!(applied(st), Some(Applied::Foreign));
            if !blocked {
                st.performance.dialog = Some(Dialog::Apply(apply_id.clone()));
            }
        });
    }
    actions = actions.child(apply);

    let meta = dots(&[
        tr(config.def.tier.label()).to_string(),
        config.def.author.to_string(),
        display_version(config),
        short_date(&config.date),
    ]);
    let head = col()
        .w_full()
        .gap(4.0)
        .child(
            row()
                .w_full()
                .gap(16.0)
                .items_end()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(text(config.name()).font(DISPLAY).color(u.ink).truncate())
                        .child(text(meta).font(SMALL).color(u.muted).truncate())
                        .children(super::expected_line(s, config).map(|line| {
                            text(line).key("perf-detail-expected").font(SMALL_NUM).color(u.prose).truncate()
                        })),
                )
                .child(actions),
        )
        // The message slot is always there, so "Applied" or an error never pushes the page.
        .child(match &s.performance.message {
            Some((line, error)) => row().w_full().h(20.0).center_y().child(
                text(line.clone())
                    .font(SMALL)
                    .color(if *error { u.rose } else { u.muted })
                    .truncate(),
            ),
            None => row().w_full().h(20.0),
        });

    let mut page = super::page("perf-detail-page")
        .gap(24.0)
        .child(col().w_full().gap(4.0).child(back(u)).child(head));
    page = page.child(summary(u, config, &options));
    if !config.body.section_ops.is_empty() {
        page = page.child(engine(u, config, &options));
    }
    for group in [Group::Visibility, Group::Camera, Group::Devtools] {
        if config.switchable().any(|opt| opt.group == group) {
            page = page.child(switches(u, config, &options, group));
        }
    }
    if !config.videos.is_empty() {
        page = page.child(video_section(s, u, config, &options));
    }
    if let Some(custom) = super::custom::detail_section(s, u, config) {
        page = page.child(custom);
    }
    page.child(left_out(s, u, config)).child(source(u, config))
}

fn section_head(u: &Ui, title: &str, line: &str, trailing: Option<El>) -> El {
    let mut left = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(title.to_string()).font(H2).color(u.ink));
    if !line.is_empty() {
        left = left.child(text(line.to_string()).font(SMALL).color(u.muted).wrap());
    }
    let mut r = row().w_full().gap(16.0).items_end().pb(8.0).child(left);
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

/// A section head: a 32 row with the title, its count, and a fixed slot for the bulk action,
/// then the description in prose at a readable width.
fn group_head(u: &Ui, title: &str, count: &str, line: &str, trailing: Option<El>) -> El {
    let mut slot = row().w(112.0).h(28.0).center_y().justify_end();
    if let Some(t) = trailing {
        slot = slot.child(t);
    }
    let mut c = col().w_full().gap(4.0).pb(8.0).child(
        row()
            .w_full()
            .h(32.0)
            .gap(8.0)
            .center_y()
            .child(text(title.to_string()).font(H2).color(u.ink))
            .child(text(count.to_string()).font(crate::ui::SMALL_NUM).color(u.muted))
            .child(spacer())
            .child(slot),
    );
    if !line.is_empty() {
        c = c.child(text(line.to_string()).font(BODY).color(u.prose).wrap().max_w(640.0));
    }
    c
}

/// One switch: the whole row toggles it. Label and convar, the author's value, the toggle.
fn switch_row(u: &Ui, key: String, label: &str, convar: &str, value: String, on: bool, press: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key)
        .w_full()
        .h(44.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .hover(u.hover)
        .focusable(true)
        .role("switch")
        .checked(on)
        .label(label.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(press)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(0.0)
                .child(text(label.to_string()).font(BODY).color(u.ink).truncate().a11y_hidden())
                .child(text(convar.to_string()).font(MONO).color(u.muted).truncate().a11y_hidden()),
        )
        .child(
            text(value)
                .font(crate::ui::SMALL_NUM)
                .color(u.muted)
                .w(160.0)
                .align(rune::Align::End)
                .truncate()
                .a11y_hidden(),
        )
        .child(u.toggle_mark(on).a11y_hidden())
}

/// The four numbers that say what an apply writes.
fn summary(u: &Ui, config: &Config, options: &Options) -> El {
    let on_switches = config
        .switchable()
        .filter(|opt| options.opt_ins.contains(&opt.key))
        .count();
    let left_out = config.body.held.len() + config.ignored.len();
    let first = std::cell::Cell::new(true);
    let stat = |value: String, label: &str| {
        let mut c = col()
            .w(176.0)
            .h(56.0)
            .gap(2.0)
            .justify_center()
            .child(text(value).font(BIG_NUM).color(u.ink))
            .child(text(label.to_string()).font(SMALL).color(u.muted).truncate());
        if !first.replace(false) {
            c = c.pl(16.0).border_sides([0.0, 0.0, 0.0, 1.0], u.hair);
        }
        c
    };
    col()
        .w_full()
        .gap(12.0)
        .child(text(config.def.note).font(BODY).color(u.prose).wrap().max_w(640.0))
        .child(
            row()
                .w_full()
                .gap(16.0)
                .child(stat(
                    config.body_convars().count().to_string(),
                    tr("Performance settings"),
                ))
                .child(stat(
                    if options.engine {
                        config.body.section_ops.len()
                    } else {
                        0
                    }
                    .to_string(),
                    tr("Engine section edits"),
                ))
                .child(stat(
                    format!("{on_switches} of {}", config.switchable().count()),
                    tr("Author switches on"),
                ))
                .child(stat(left_out.to_string(), tr("Left out"))),
        )
        .child(
            text(fill(
                tr("Applying writes {} settings to gameinfo.gi. Turn off puts every stock value back."),
                &[&config.setting_count(options).to_string()],
            ))
            .font(SMALL)
            .color(u.muted),
        )
}

fn engine(u: &Ui, config: &Config, options: &Options) -> El {
    let mut sections: Vec<String> = Vec::new();
    for op in &config.body.section_ops {
        let name = op.path.first().cloned().unwrap_or_default();
        if !sections.contains(&name) {
            sections.push(name);
        }
    }
    let named = match sections.len() {
        0 => String::new(),
        1..=3 => sections.join(", "),
        n => format!("{} and {} more", sections[..3].join(", "), n - 3),
    };
    let id = config.id().to_string();
    let engine_on = options.engine;
    let count = config.body.section_ops.len();
    col()
        .w_full()
        .child(group_head(
            u,
            tr("Engine section edits"),
            "",
            tr("Edits outside ConVars. If matchmaking warns about unverified game files, turn these off."),
            None,
        ))
        .child(
            col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair).child(
                switch_row(
                    u,
                    "perf-engine".to_string(),
                    tr("Include engine section edits"),
                    &fill(tr("{} edits in {}"), &[&count.to_string(), &named]),
                    String::new(),
                    engine_on,
                    move |st: &mut State| edit(st, &id, |options| options.engine = !engine_on),
                ),
            ),
        )
}

/// Change the switches for a config, starting from what it shows now.
fn edit(st: &mut State, id: &str, change: impl FnOnce(&mut Options)) {
    let Some(config) = config(st, id) else {
        return;
    };
    let mut options = options_for(st, config);
    change(&mut options);
    st.performance.options.insert(id.to_string(), options);
}

fn group_line(group: Group) -> &'static str {
    match group {
        Group::Visibility => {
            tr("Glows, outlines, health bars and tracers the author changed. On by default, as the author ships them.")
        }
        Group::Camera => {
            tr("Field of view and camera angles the author changed. On by default, as the author ships them.")
        }
        Group::Devtools => tr("Hideout testing tools and debug drawing. Off unless you turn them on."),
    }
}

fn switches(u: &Ui, config: &Config, options: &Options, group: Group) -> El {
    let items: Vec<_> = config
        .switchable()
        .filter(|opt| opt.group == group)
        .collect();
    let on = items
        .iter()
        .filter(|opt| options.opt_ins.contains(&opt.key))
        .count();
    let id = config.id().to_string();
    let keys: Vec<String> = items.iter().map(|opt| opt.key.clone()).collect();
    let all_on = on == items.len();
    let bulk = u
        .action(
            if all_on {
                tr("Turn all off")
            } else {
                tr("Turn all on")
            },
            ActionKind::Ghost,
        )
        .key(format!("perf-bulk-{}", group.id()))
        // The label's right edge meets the switches' right edge.
        .mr(-12.0)
        .cursor(rune::Cursor::Pointer)
    .on_press(move |st: &mut State| {
        edit(st, &id, |options| {
            for key in &keys {
                if all_on {
                    options.opt_ins.remove(key);
                } else {
                    options.opt_ins.insert(key.clone());
                }
            }
        })
    });
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    for opt in items {
        let key = opt.key.clone();
        let id = config.id().to_string();
        let is_on = options.opt_ins.contains(&opt.key);
        let values = match &opt.stock {
            Some(stock) => fill(
                tr("Author {} · game {}"),
                &[&video_value(&opt.value), &video_value(stock)],
            ),
            None => fill(tr("Author {}"), &[&video_value(&opt.value)]),
        };
        let label = performance::classify::opt_in_label(&opt.key).unwrap_or(&opt.key);
        let unknown = config.convar_state(&opt.key) == ConvarState::Unknown;
        rows = rows.child(switch_row(
            u,
            format!("perf-opt-{}", opt.key),
            label,
            &opt.key,
            if unknown {
                format!("{values} · may do nothing")
            } else {
                values
            },
            is_on,
            move |st: &mut State| {
                edit(st, &id, |options| {
                    if !options.opt_ins.remove(&key) {
                        options.opt_ins.insert(key.clone());
                    }
                })
            },
        ));
    }
    col()
        .w_full()
        .child(group_head(
            u,
            tr(group.label()),
            &fill(tr("{} of {} on"), &[&on.to_string(), &rows_count(config, group).to_string()]),
            group_line(group),
            Some(bulk),
        ))
        .child(rows)
}

fn rows_count(config: &Config, group: Group) -> usize {
    config.switchable().filter(|opt| opt.group == group).count()
}

fn video_section(s: &State, u: &Ui, config: &Config, options: &Options) -> El {
    let set = config
        .video(options.video.as_deref())
        .unwrap_or(&config.videos[0]);
    let status = status(s);
    let record = status
        .and_then(|status| status.video.as_ref())
        .filter(|record| record.config == config.id());
    let mut trailing = row().gap(8.0).center_y();
    if config.videos.len() > 1 {
        let segments = config
            .videos
            .iter()
            .map(|variant| {
                let id = config.id().to_string();
                let file = variant.file.clone();
                subpage::segment(u, &variant.label, variant.file == set.file)
                    .key(format!(
                        "perf-video-{}",
                        variant.label.to_lowercase().replace(' ', "-")
                    ))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        let file = file.clone();
                        edit(st, &id, move |options| options.video = Some(file))
                    })
            })
            .collect();
        trailing = trailing.child(subpage::segmented(u, segments));
    }

    let mut grid = row().w_full().gap(32.0).items_start();
    let half = set.settings.len().div_ceil(2);
    for column in set.settings.chunks(half.max(1)) {
        let mut c = col()
            .grow(1.0)
            .min_w(0.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
        for (key, value) in column {
            let label = video::label(key)
                .map(str::to_string)
                .unwrap_or_else(|| key.clone());
            c = c.child(
                row()
                    .w_full()
                    .h(36.0)
                    .gap(12.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                    .child(
                        text(label)
                            .font(if video::label(key).is_some() {
                                BODY
                            } else {
                                MONO
                            })
                            .color(u.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(
                        text(if key == "mat_viewportscale" {
                            value
                                .parse::<f64>()
                                .map(|v| format!("{}%", (v * 100.0).round()))
                                .unwrap_or_else(|_| value.clone())
                        } else {
                            capitalize(&video_value(value))
                        })
                        .font(BODY)
                        .color(u.muted),
                    ),
            );
        }
        grid = grid.child(c);
    }

    let id = config.id().to_string();
    let action: El = match (record, status.and_then(|st| st.video_path.as_ref())) {
        (Some(record), _) => {
            let kept = status.map(|st| st.video_kept).unwrap_or(0);
            let changed = record.written.len().saturating_sub(kept);
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(if changed > 0 {
                        fill(tr("Written to video.txt. You changed {} of them in game since."), &[&changed.to_string()])
                    } else {
                        tr("Written to video.txt.").to_string()
                    })
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0),
                )
                .child(
                    u.action(tr("Keep these values"), ActionKind::Secondary)
                        .key("perf-video-keep")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.performance.dialog = Some(Dialog::KeepVideo)),
                )
                .child(
                    u.action(tr("Restore my video settings"), ActionKind::Secondary)
                        .key("perf-video-restore")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| st.performance.dialog = Some(Dialog::RestoreVideo)),
                )
        }
        (None, Some(_)) => row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                text(tr("Set these in game under Settings > Video, or let Sigil write them."))
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0),
            )
            .child(
                u.action(tr("Write to video.txt"), ActionKind::Secondary)
                    .key("perf-video-write")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if !st.performance.write_job.is_running() {
                            st.performance.dialog = Some(Dialog::WriteVideo(id.clone()));
                        }
                    }),
            ),
        (None, None) => text(tr("Set these in game under Settings > Video. Sigil can write them once the game has created its video.txt: start Deadlock once."))
            .font(SMALL)
            .color(u.muted)
            .wrap(),
    };

    col()
        .w_full()
        .child(section_head(
            u,
            tr("Recommended in-game settings"),
            tr("The author tuned the config for these graphics settings. Your resolution, window mode, V-Sync and frame cap are not part of it."),
            Some(trailing),
        ))
        .child(grid)
        .child(el().h(12.0))
        .child(action)
}

fn video_value(value: &str) -> String {
    match value {
        "true" => "on".into(),
        "false" => "off".into(),
        other => match other.parse::<f64>() {
            Ok(number) if number.fract() == 0.0 => format!("{}", number as i64),
            Ok(number) => format!("{number}"),
            Err(_) => other.to_string(),
        },
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What the author sets that Sigil does not write, and why.
fn left_out(s: &State, u: &Ui, config: &Config) -> El {
    let mut rows = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair);
    let line = |key: String, reason: String, code: bool| {
        row()
            .w_full()
            .min_h(44.0)
            .py(10.0)
            .gap(16.0)
            .items_start()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(
                text(key)
                    .font(if code { MONO } else { BODY })
                    .color(u.ink)
                    .w(300.0)
                    .truncate(),
            )
            .child(
                text(reason)
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
            )
    };
    let mut any = false;
    if !config.ignored.is_empty() {
        any = true;
        let limit = if s.performance.show_all_ignored {
            config.ignored.len()
        } else {
            6
        };
        let shown: Vec<&str> = config
            .ignored
            .iter()
            .take(limit)
            .map(String::as_str)
            .collect();
        let more = config.ignored.len().saturating_sub(shown.len());
        let mut names = col().w(300.0).gap(4.0);
        for name in shown {
            names = names.child(text(name.to_string()).font(MONO).color(u.ink).truncate());
        }
        if more > 0 {
            names = names.child(
                text(fill(tr("Show {} more"), &[&more.to_string()]))
                    .font(SMALL)
                    .color(u.muted)
                    .key("perf-ignored-more")
                    .role("button")
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.performance.show_all_ignored = true),
            );
        }
        rows = rows.child(
            row()
                .w_full()
                .py(10.0)
                .gap(16.0)
                .items_start()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(names)
                .child(
                    text(format!(
                        "{} setting{} the game no longer reads from gameinfo.gi. Writing {} would do nothing.",
                        config.ignored.len(),
                        if config.ignored.len() == 1 { "" } else { "s" },
                        if config.ignored.len() == 1 { "it" } else { "them" }
                    ))
                    .font(SMALL)
                    .color(u.muted)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
                ),
        );
    }
    for held in &config.body.held {
        any = true;
        let key = if held.place.ends_with("setting") || held.place.ends_with("settings") {
            format!("{} section, {}", held.key, held.place)
        } else {
            held.key.clone()
        };
        rows = rows.child(line(key, held.reason.clone(), true));
    }
    if !config.unknown.is_empty() {
        any = true;
        rows = rows.child(line(
            format!("{} unlisted settings", config.unknown.len()),
            tr("Not in the game's current list of settings. They are written as the author ships them, but may do nothing.").into(),
            false,
        ));
    }
    if !any {
        rows = rows.child(line(
            tr("Nothing").into(),
            tr("Every setting in this config is applied.").into(),
            false,
        ));
    }
    let line = if s
        .performance
        .catalog
        .as_ref()
        .is_some_and(|c| c.game_convars)
    {
        tr("Checked against Valve's list of settings for the current game build.")
    } else {
        tr("Valve's list of settings is not downloaded yet, so settings the game ignores are not marked.")
    };
    col()
        .w_full()
        .child(section_head(u, tr("Left out"), line, None))
        .child(rows)
}

fn source(u: &Ui, config: &Config) -> El {
    let link = |label: String, url: String, key: &str| {
        row()
            .key(key.to_string())
            .gap(6.0)
            .center_y()
            .role("link")
            .focusable(true)
            .label(label.clone())
            .cursor(rune::Cursor::Pointer)
            .on_press(move |_: &mut State| open_url(&url))
            .child(text(label).font(BODY).color(u.ink).truncate())
            .child(
                icon(&icons::EXTERNAL_LINK, 12.0)
                    .color(u.muted)
                    .a11y_hidden(),
            )
    };
    let short = &config.commit[..7.min(config.commit.len())];
    col()
        .w_full()
        .child(section_head(u, tr("Source"), "", None))
        .child(
            col()
                .w_full()
                .border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
                .child(subpage::kv(u, tr("Author"), subpage::val(u, config.def.author)))
                .child(subpage::kv(
                    u,
                    tr("Project"),
                    if config.source.id == "mod" {
                        // A mod's own gameinfo: its GameBanana page, when it came from there.
                        let url = config.link();
                        if url.is_empty() {
                            subpage::val(u, &config.version)
                        } else {
                            link(config.version.clone(), url, "perf-link-repo")
                        }
                    } else if let Some(hosted) = config.def.gamebanana {
                        link(
                            fill(tr("GameBanana (mod {})"), &[&hosted.mod_id.to_string()]),
                            config.link(),
                            "perf-link-repo",
                        )
                    } else {
                        link(
                            format!("{} ({})", config.source.repo, config.source.license),
                            config.source.url(),
                            "perf-link-repo",
                        )
                    },
                ))
                .child(subpage::kv(
                    u,
                    tr("File"),
                    link(config.path.clone(), config.link(), "perf-link-file"),
                ))
                .child(subpage::kv(
                    u,
                    tr("Version"),
                    subpage::val(
                        u,
                        dots(&[
                            version_label(&config.version),
                            format!("commit {short}"),
                            short_date(&config.date),
                        ]),
                    ),
                )),
        )
}


