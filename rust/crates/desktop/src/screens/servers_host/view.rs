//! The Host tab's page and its two confirm dialogs. Reads `HostState` only; every press calls
//! an action in `super`.

use rune::{Color, El, Key, KeyEvent, ScrollSpec, Tx, col, el, icon, row, spacer, text};

use super::{
    Busy, Confirm, Firewall, HostState, NEED_BYTES, Phase, Setup, connect_line, gigabytes,
};
use crate::app::State;
use crate::icons;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, SMALL, Ui};

/// The page width the Host sections keep, so lines stay readable on a wide window.
const MAX_W: f32 = 920.0;
/// Console lines drawn at once; the rest stay in memory for Copy.
const CONSOLE_DRAWN: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Done,
    Todo,
    Warn,
    Busy,
    /// Waits for an earlier step.
    Blocked,
    /// Advice, not a step.
    Info,
}

pub fn view(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let mut page = col().key("host-page").w_full().max_w(MAX_W).gap(24.0);
    if !host.windows {
        return page.child(not_windows(u));
    }
    page = page.child(intro(u));
    let setup = match &host.setup {
        Some(Ok(setup)) => Some(setup),
        _ => None,
    };
    if let Some(setup) = setup {
        if setup.outdated() {
            page = page.child(outdated_banner(u, host, setup));
        }
        if setup.deadworks_outdated() {
            page = page.child(deadworks_banner(u, host, setup));
        }
    }
    if let Some(error) = &host.error {
        page = page.child(error_line(u, error));
    }
    page = page.child(setup_section(s, u));
    page = page.child(server_section(s, u));
    page = page.child(reach_section(s, u));
    page = page.child(plugins_section(s, u));
    page
}

// ---------------------------------------------------------------------------------------------
// Header pieces
// ---------------------------------------------------------------------------------------------

fn intro(u: &Ui) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(
            text(tr("Run a Deadworks server on this PC for you and your friends. Sigil sets it up, starts and stops it, and shows its console."))
                .font(BODY)
                .color(u.prose)
                .wrap(),
        )
        .child(
            text(tr("Hosting needs Windows, about 35 GB of free space, the .NET 10 runtime and a Steam account that owns Deadlock. You sign in to Steam in SteamCMD's own window; Sigil never sees your password."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        )
}

fn not_windows(u: &Ui) -> El {
    let mut list = col().w_full().gap(6.0).py(4.0);
    for line in [
        tr("Windows 10 or 11, 64-bit"),
        tr("About 35 GB of free space for the server files"),
        tr("The .NET 10 runtime (x64) from Microsoft"),
        tr("A Steam account that owns Deadlock, signed in inside SteamCMD's own window"),
    ] {
        list = list.child(bullet(u, line));
    }
    col()
        .key("host-not-windows")
        .w_full()
        .gap(12.0)
        .py(24.0)
        .child(text(tr("Hosting needs Windows")).font(H1).color(u.ink))
        .child(
            text(tr("The Deadlock dedicated server and Deadworks run on Windows only, so Sigil can host from a Windows PC. Joining servers works here as usual."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        )
        .child(text(tr("What hosting needs")).font(H2).color(u.ink).pt(8.0))
        .child(list)
        .child(
            text(tr("On Linux, the Deadworks guide shows a Docker image that runs the Windows server under Wine."))
                .font(SMALL)
                .color(u.faint)
                .wrap(),
        )
        .child(
            row().child(
                u.icon_action(&icons::EXTERNAL_LINK, tr("Open the Deadworks guide"), ActionKind::Secondary)
                    .key("host-guide")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| open_url(st, tx, deadlock_core::servers::DEADWORKS_HOST_GUIDE_URL)),
            ),
        )
}

fn open_url(st: &mut State, tx: &mut Tx, url: &str) {
    if let Err(error) = tx.open_url(url) {
        st.status = fill(
            tr("The page could not be opened: {}"),
            &[&error.to_string()],
        );
    }
}

fn banner(u: &Ui, key: &str, line: String, action: El) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .gap(12.0)
        .px(14.0)
        .py(10.0)
        .center_y()
        .radius(crate::ui::r(10.0))
        .fill(u.amber.alpha(0.07))
        .border(1.0, u.amber.alpha(0.30))
        .child(
            icon(&icons::TRIANGLE_ALERT, 16.0)
                .color(u.amber)
                .a11y_hidden(),
        )
        .child(
            text(line)
                .font(BODY)
                .color(u.ink)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(action)
}

fn outdated_banner(u: &Ui, host: &HostState, setup: &Setup) -> El {
    let line = fill(
        tr(
            "Server files are older than your game (server build {}, game build {}). Players on the new build cannot join until you update.",
        ),
        &[
            setup.server_build.as_deref().unwrap_or("?"),
            setup.game_build.as_deref().unwrap_or("?"),
        ],
    );
    let blocked = host.running() || host.steamcmd.is_some();
    banner(
        u,
        "host-outdated",
        line,
        u.icon_action(&icons::REFRESH_CW, tr("Update"), ActionKind::Secondary)
            .key("host-update-files")
            .opacity(if blocked { 0.5 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| {
                if !st.servers.host.running() && st.servers.host.steamcmd.is_none() {
                    st.servers.host.confirm = Some(Confirm::ServerFiles { update: true });
                }
            }),
    )
}

fn deadworks_banner(u: &Ui, host: &HostState, setup: &Setup) -> El {
    let line = match setup.deadworks.as_deref() {
        Some(found) if !found.is_empty() => fill(
            tr("Deadworks {} is in the server folder. Sigil's pinned release is {}."),
            &[found, &setup.deadworks_pinned],
        ),
        _ => fill(
            tr(
                "The Deadworks in the server folder was not installed by Sigil. Sigil's pinned release is {}.",
            ),
            &[&setup.deadworks_pinned],
        ),
    };
    let label = fill(tr("Install {}"), &[&setup.deadworks_pinned]);
    banner(
        u,
        "host-deadworks-outdated",
        line,
        u.action(&label, ActionKind::Secondary)
            .key("host-update-deadworks")
            .opacity(if host.running() || host.busy.is_some() {
                0.5
            } else {
                1.0
            })
            .cursor(rune::Cursor::Pointer)
            .on_press(super::install_deadworks),
    )
}

fn error_line(u: &Ui, error: &str) -> El {
    row()
        .key("host-error")
        .w_full()
        .gap(10.0)
        .items_start()
        .child(
            icon(&icons::CIRCLE_X, 16.0)
                .color(u.rose)
                .mt(2.0)
                .a11y_hidden(),
        )
        .child(
            text(error.to_string())
                .font(BODY)
                .color(u.rose)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(
            u.icon_button(&icons::X, tr("Dismiss"))
                .key("host-error-dismiss")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| st.servers.host.error = None),
        )
}

fn section_head(u: &Ui, title: &str, meta: &str, trailing: Option<El>) -> El {
    let mut r = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(text(title.to_string()).font(H2).color(u.ink))
        .child(text(meta.to_string()).font(SMALL).color(u.faint))
        .child(spacer());
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

fn bullet(u: &Ui, line: &str) -> El {
    row()
        .w_full()
        .gap(10.0)
        .items_start()
        .child(
            el().size(4.0, 4.0)
                .radius(2.0)
                .fill(u.faint)
                .mt(8.0)
                .a11y_hidden(),
        )
        .child(
            text(line.to_string())
                .font(BODY)
                .color(u.prose)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

// ---------------------------------------------------------------------------------------------
// Setup checklist
// ---------------------------------------------------------------------------------------------

fn step_mark(u: &Ui, step: Step) -> El {
    let (glyph, color) = match step {
        Step::Done => (&icons::CIRCLE_CHECK, u.teal),
        Step::Warn => (&icons::TRIANGLE_ALERT, u.amber),
        Step::Busy => (&icons::LOADER, u.accent),
        Step::Todo => (&icons::CIRCLE_X, u.faint),
        Step::Blocked => (&icons::LOCK, u.faint),
        Step::Info => (&icons::GLOBE, u.faint),
    };
    el().size(20.0, 20.0)
        .mt(1.0)
        .center()
        .child(icon(glyph, 16.0).color(color).a11y_hidden())
}

fn step_row(u: &Ui, key: &str, step: Step, title: &str, detail: El, action: Option<El>) -> El {
    let mut r = row()
        .key(key.to_string())
        .w_full()
        .gap(12.0)
        .px(16.0)
        .py(12.0)
        .items_start()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(step_mark(u, step))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(title.to_string())
                        .font(BODY_MED)
                        .color(if step == Step::Blocked {
                            u.muted
                        } else {
                            u.ink
                        }),
                )
                .child(detail),
        );
    if let Some(action) = action {
        r = r.child(action);
    }
    r
}

fn detail(u: &Ui, line: String) -> El {
    text(line).font(SMALL).color(u.muted).wrap().w_full()
}

fn step_action(u: &Ui, key: &str, label: &str, kind: ActionKind, enabled: bool) -> El {
    u.action(label, kind)
        .key(key.to_string())
        .opacity(if enabled { 1.0 } else { 0.45 })
        .cursor(if enabled {
            rune::Cursor::Pointer
        } else {
            rune::Cursor::Default
        })
}

fn setup_section(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let checking = host.check_job.is_running();
    let again = || {
        checking.then(|| {
            row()
                .gap(6.0)
                .center_y()
                .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
                .child(text(tr("Checking this PC")).font(SMALL).color(u.faint))
        })
    };
    let setup = match &host.setup {
        None => {
            return col()
                .key("host-setup")
                .w_full()
                .gap(10.0)
                .child(section_head(u, tr("Setup"), "", None))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(icon(&icons::LOADER, 14.0).color(u.faint).a11y_hidden())
                        .child(text(tr("Checking this PC")).font(BODY).color(u.muted)),
                );
        }
        Some(Err(error)) => {
            return col()
                .key("host-setup")
                .w_full()
                .gap(10.0)
                .child(section_head(u, tr("Setup"), "", None))
                .child(text(error.clone()).font(BODY).color(u.rose).wrap());
        }
        Some(Ok(setup)) => setup,
    };
    let done = [
        setup.space_ok(),
        setup.dotnet.is_some(),
        setup.steamcmd,
        setup.server_build.is_some(),
        setup.deadworks.is_some(),
    ]
    .iter()
    .filter(|ok| **ok)
    .count();
    let meta = fill(tr("{} of {} done"), &[&done.to_string(), "5"]);
    let busy = host.busy;
    let locked = host.running() || busy.is_some() || host.steamcmd.is_some();

    let mut panel = u.panel().key("host-checklist").w_full();

    // 1. Folder and free space.
    let dir = host
        .server_dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let space = match setup.free_bytes {
        Some(free) if setup.space_ok() => fill(
            tr("{} free here. A new server needs about {}."),
            &[&gigabytes(free), &gigabytes(NEED_BYTES)],
        ),
        Some(free) => fill(
            tr(
                "Only {} free here. The server files need about {}: pick a folder on a drive with more room.",
            ),
            &[&gigabytes(free), &gigabytes(NEED_BYTES)],
        ),
        None => tr("Free space could not be read.").to_string(),
    };
    panel = panel.child(step_row(
        u,
        "host-step-folder",
        if setup.space_ok() {
            Step::Done
        } else {
            Step::Warn
        },
        tr("Server folder"),
        col()
            .w_full()
            .gap(2.0)
            .child(text(dir).font(MONO).color(u.prose).w_full().wrap_anywhere())
            .child(detail(u, space)),
        Some(
            step_action(
                u,
                "host-folder",
                tr("Change"),
                ActionKind::Secondary,
                !locked,
            )
            .on_press_tx(super::choose_folder),
        ),
    ));

    // 2. .NET 10.
    let dotnet = match &setup.dotnet {
        Some(version) => step_row(
            u,
            "host-step-dotnet",
            Step::Done,
            tr(".NET 10 runtime"),
            detail(u, fill(tr("Found {}."), &[version])),
            None,
        ),
        None => step_row(
            u,
            "host-step-dotnet",
            Step::Todo,
            tr(".NET 10 runtime"),
            detail(u, tr("Not found. Install the .NET 10 Runtime (x64) from Microsoft, then press Check again.").to_string()),
            Some(
                u.icon_action(&icons::EXTERNAL_LINK, tr("Get .NET 10"), ActionKind::Secondary)
                    .key("host-dotnet")
                    .cursor(rune::Cursor::Pointer)
                    .on_press_tx(|st: &mut State, tx: &mut Tx| open_url(st, tx, super::DOTNET_URL)),
            ),
        ),
    };
    panel = panel.child(dotnet);

    // 3. SteamCMD.
    let steamcmd = if busy == Some(Busy::SteamCmd) {
        step_row(
            u,
            "host-step-steamcmd",
            Step::Busy,
            tr("SteamCMD"),
            detail(u, tr("Downloading SteamCMD from Valve...").to_string()),
            None,
        )
    } else if setup.steamcmd {
        step_row(
            u,
            "host-step-steamcmd",
            Step::Done,
            tr("SteamCMD"),
            detail(
                u,
                tr("Valve's command-line Steam client, kept in Sigil's app data.").to_string(),
            ),
            None,
        )
    } else {
        let can = setup.space_ok() && !locked;
        step_row(
            u,
            "host-step-steamcmd",
            Step::Todo,
            tr("SteamCMD"),
            detail(u, tr("Valve's command-line Steam client. It downloads the server files. About 4 MB from Valve.").to_string()),
            Some(
                step_action(u, "host-steamcmd", tr("Install SteamCMD"), ActionKind::Secondary, can)
                    .on_press(super::install_steamcmd),
            ),
        )
    };
    panel = panel.child(steamcmd);

    // 4. Server files.
    let files = if host.steamcmd.is_some() {
        let mut body = col().w_full().gap(4.0).child(detail(
            u,
            tr("SteamCMD is open in its own window. Sign in there with your Steam account; Steam Guard asks there too. Close the window when it says the download is done.").to_string(),
        ));
        if let Some(line) = &host.steamcmd_status {
            body = body.child(
                text(line.clone())
                    .font(MONO)
                    .color(u.prose)
                    .w_full()
                    .truncate(),
            );
        }
        step_row(
            u,
            "host-step-files",
            Step::Busy,
            tr("Server files"),
            body,
            None,
        )
    } else if let Some(build) = &setup.server_build {
        step_row(
            u,
            "host-step-files",
            if setup.outdated() {
                Step::Warn
            } else {
                Step::Done
            },
            tr("Server files"),
            detail(u, fill(tr("Build {}, app 1422450."), &[build])),
            Some(
                step_action(u, "host-files", tr("Update"), ActionKind::Ghost, !locked).on_press(
                    |st: &mut State| {
                        if !st.servers.host.running() && st.servers.host.busy.is_none() {
                            st.servers.host.confirm = Some(Confirm::ServerFiles { update: true });
                        }
                    },
                ),
            ),
        )
    } else if !setup.steamcmd {
        step_row(
            u,
            "host-step-files",
            Step::Blocked,
            tr("Server files"),
            detail(u, tr("Install SteamCMD first.").to_string()),
            None,
        )
    } else {
        let can = setup.space_ok() && !locked;
        step_row(
            u,
            "host-step-files",
            Step::Todo,
            tr("Server files"),
            detail(u, tr("Not downloaded yet. SteamCMD opens in its own window, you sign in there, and it downloads about 35 GB.").to_string()),
            Some(
                step_action(u, "host-files", tr("Download"), ActionKind::Primary, can).on_press(
                    |st: &mut State| {
                        if st.servers.host.busy.is_none() && st.servers.host.steamcmd.is_none() {
                            st.servers.host.confirm = Some(Confirm::ServerFiles { update: false });
                        }
                    },
                ),
            ),
        )
    };
    panel = panel.child(files);

    // 5. Deadworks.
    let deadworks = if busy == Some(Busy::Deadworks) {
        step_row(
            u,
            "host-step-deadworks",
            Step::Busy,
            tr("Deadworks"),
            detail(
                u,
                fill(
                    tr("Downloading Deadworks {} from GitHub and checking its hash..."),
                    &[&setup.deadworks_pinned],
                ),
            ),
            None,
        )
    } else if let Some(version) = &setup.deadworks {
        step_row(
            u,
            "host-step-deadworks",
            if setup.deadworks_behind {
                Step::Warn
            } else {
                Step::Done
            },
            tr("Deadworks"),
            detail(
                u,
                if version.is_empty() {
                    tr("Installed outside Sigil. The plugin framework the server runs.").to_string()
                } else {
                    fill(
                        tr("Version {}, the plugin framework the server runs."),
                        &[version],
                    )
                },
            ),
            None,
        )
    } else if setup.server_build.is_none() {
        step_row(
            u,
            "host-step-deadworks",
            Step::Blocked,
            tr("Deadworks"),
            detail(
                u,
                tr("Download the server files first. Deadworks goes on top of them.").to_string(),
            ),
            None,
        )
    } else {
        let label = fill(tr("Install {}"), &[&setup.deadworks_pinned]);
        step_row(
            u,
            "host-step-deadworks",
            Step::Todo,
            tr("Deadworks"),
            detail(u, tr("The plugin framework the server runs (MIT). Sigil downloads its pinned release from GitHub and checks the hash before unpacking.").to_string()),
            Some(step_action(u, "host-deadworks", &label, ActionKind::Primary, !locked).on_press(super::install_deadworks)),
        )
    };
    panel = panel.child(deadworks.border_sides([0.0, 0.0, 0.0, 0.0], Color::TRANSPARENT));

    col()
        .key("host-setup")
        .w_full()
        .gap(10.0)
        .child(section_head(u, tr("Setup"), &meta, again()))
        .child(panel)
}

// ---------------------------------------------------------------------------------------------
// Server card
// ---------------------------------------------------------------------------------------------

fn phase_label(u: &Ui, phase: &Phase) -> (&'static str, Color, bool) {
    match phase {
        Phase::Stopped => ("Stopped", u.faint, false),
        Phase::Starting => ("Starting", u.amber, true),
        Phase::Running => ("Running", u.teal, true),
        Phase::Stopping => ("Stopping", u.amber, true),
        Phase::Exited(_) => ("Stopped", u.amber, false),
        Phase::Failed(_) => ("Did not start", u.rose, false),
    }
}

/// The server's state at the right of the Servers tab bar, while the Host tab is open.
pub fn tab_status(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    if !host.windows {
        return el();
    }
    let (label, color, _) = phase_label(u, &host.phase);
    let line = if host.phase == Phase::Running {
        fill(tr("Running on port {}"), &[&host.port().to_string()])
    } else {
        tr(label).to_string()
    };
    row()
        .key("host-tab-status")
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .child(el().size(6.0, 6.0).radius(3.0).fill(color).a11y_hidden())
        .child(text(line).font(SMALL).color(u.muted))
}

fn field(u: &Ui, label: &str, key: &str, locked: bool, input: El) -> El {
    col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(6.0)
        .child(text(label.to_string()).font(SMALL).color(u.muted))
        .child(
            row()
                .key(key.to_string())
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .opacity(if locked { 0.6 } else { 1.0 })
                .child(input.grow(1.0).min_w(0.0)),
        )
}

macro_rules! input {
    ($u:expr, $edit:expr, $locked:expr, $label:expr, $placeholder:expr, $style:expr, $field:ident) => {{
        let e = rune::text_input($edit, $style)
            .label($label)
            .single_line()
            .color($u.ink)
            .placeholder($placeholder.to_string(), $u.faint)
            .caret($u.accent, $u.accent.alpha(0.20));
        if $locked {
            e
        } else {
            e.on_edit(|st: &mut State| &mut st.servers.host.$field)
        }
    }};
}

fn server_section(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let ready = host.ready();
    let (label, color, live) = phase_label(u, &host.phase);
    let status = row()
        .key("host-status")
        .gap(6.0)
        .center_y()
        .role("status")
        .label(tr(label))
        .child(el().size(8.0, 8.0).radius(4.0).fill(color).a11y_hidden())
        .child(
            text(tr(label))
                .font(SMALL)
                .color(if live { u.ink } else { u.muted })
                .a11y_hidden(),
        );
    let button = match host.phase {
        Phase::Running => u
            .icon_action(&icons::SQUARE_STOP, tr("Stop"), ActionKind::Danger)
            .key("host-stop")
            .cursor(rune::Cursor::Pointer)
            .on_press(super::stop),
        Phase::Starting => u
            .icon_action(&icons::LOADER, tr("Starting"), ActionKind::Secondary)
            .key("host-stop")
            .opacity(0.6),
        Phase::Stopping => u
            .icon_action(&icons::LOADER, tr("Stopping"), ActionKind::Secondary)
            .key("host-stop")
            .opacity(0.6),
        _ => {
            let can = ready && host.busy.is_none() && host.steamcmd.is_none();
            u.icon_action(&icons::PLAY, tr("Start server"), ActionKind::Primary)
                .key("host-start")
                .opacity(if can { 1.0 } else { 0.45 })
                .cursor(if can {
                    rune::Cursor::Pointer
                } else {
                    rune::Cursor::Default
                })
                .on_press(super::start)
        }
    };
    let head = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(text(tr("Server")).font(H2).color(u.ink))
        .child(status)
        .child(spacer())
        .child(button);

    let locked = host.running();
    let fields = col()
        .w_full()
        .gap(12.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .child(field(
                    u,
                    tr("Server name"),
                    "host-name",
                    locked,
                    input!(
                        u,
                        &host.name,
                        locked,
                        tr("Server name"),
                        tr("My Deadlock server"),
                        BODY,
                        name
                    ),
                ))
                .child(field(
                    u,
                    tr("Map"),
                    "host-map",
                    locked,
                    row()
                        .gap(4.0)
                        .center_y()
                        .child(
                            input!(u, &host.map, locked, tr("Map"), "dl_midtown", MONO, map)
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(
                            el().size(24.0, 24.0)
                                .center()
                                .radius(crate::ui::r(6.0))
                                .hover(u.hover)
                                .focusable(!locked)
                                .role("button")
                                .label(tr("Pick a map"))
                                .cursor(if locked {
                                    rune::Cursor::Default
                                } else {
                                    rune::Cursor::Pointer
                                })
                                .key("host-map-pick")
                                .child(
                                    icon(&icons::CHEVRON_DOWN, 14.0)
                                        .color(u.muted)
                                        .a11y_hidden(),
                                )
                                .on_press(|st: &mut State| {
                                    if !st.servers.host.running() {
                                        st.servers.host.map_menu = !st.servers.host.map_menu;
                                    }
                                }),
                        ),
                )),
        )
        .child(
            row()
                .w_full()
                .gap(12.0)
                .child(field(
                    u,
                    tr("Port"),
                    "host-port",
                    locked,
                    input!(u, &host.port, locked, tr("Port"), "27067", MONO, port),
                ))
                .child(field(
                    u,
                    tr("Max players"),
                    "host-max",
                    locked,
                    input!(
                        u,
                        &host.max_players,
                        locked,
                        tr("Max players"),
                        tr("Server default"),
                        MONO,
                        max_players
                    ),
                )),
        )
        .child(
            row()
                .w_full()
                .gap(12.0)
                .child(field(
                    u,
                    tr("Password"),
                    "host-password",
                    locked,
                    input!(
                        u,
                        &host.password,
                        locked,
                        tr("Password"),
                        tr("None: anyone can join"),
                        BODY,
                        password
                    )
                    .secure(),
                ))
                .child(field(
                    u,
                    tr("Extra launch options"),
                    "host-extra",
                    locked,
                    input!(
                        u,
                        &host.extra,
                        locked,
                        tr("Extra launch options"),
                        "+sv_cheats 0",
                        MONO,
                        extra
                    ),
                )),
        );

    let mut card = col()
        .key("host-server")
        .w_full()
        .gap(16.0)
        .padding(16.0)
        .radius(crate::ui::r(10.0))
        .border(1.0, u.hair)
        .child(head);
    if !ready {
        card = card.child(
            text(tr("Finish the setup above to start the server."))
                .font(BODY)
                .color(u.muted)
                .wrap(),
        );
    }
    match &host.phase {
        Phase::Failed(error) => {
            card = card.child(text(error.clone()).font(BODY).color(u.rose).wrap());
        }
        Phase::Exited(line) => {
            let said = if line.is_empty() {
                tr("The server stopped on its own. Its last lines are in the console below.")
                    .to_string()
            } else {
                fill(tr("The server stopped on its own: {}"), &[line])
            };
            card = card.child(text(said).font(BODY).color(u.amber).wrap());
        }
        _ => {}
    }
    card = card.child(fields);
    let map = host.map.text.trim();
    if let Some(item) = host.content_for(map).filter(|_| !locked) {
        // The file name at either separator, so a Windows path reads right anywhere.
        let path = item.vpk.to_string_lossy();
        let file = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string();
        card = card.child(
            row()
                .key("host-content")
                .w_full()
                .gap(10.0)
                .items_start()
                .child(icon(&icons::PACKAGE, 16.0).color(u.accent).mt(2.0).a11y_hidden())
                .child(
                    text(fill(
                        tr("Custom map {}: Start copies {} into the server folder first, then loads it."),
                        &[map, &file],
                    ))
                    .font(SMALL)
                    .color(u.prose)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
                ),
        );
    }
    if locked {
        card = card.child(
            text(tr("Stop the server to change its settings."))
                .font(SMALL)
                .color(u.faint),
        );
    }
    card.child(console(s, u))
}

fn console(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let count = host.console.len();
    let all: String = host.console.iter().cloned().collect::<Vec<_>>().join("\n");
    let head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(text(tr("Console")).font(BODY_MED).color(u.ink))
        .child(
            text(fill(tr("{} lines"), &[&count.to_string()]))
                .font(SMALL)
                .color(u.faint),
        )
        .child(spacer())
        .child(
            u.icon_button(&icons::COPY, tr("Copy the console"))
                .key("host-console-copy")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    tx.copy(all.clone());
                    st.status = tr("Console copied.").to_string();
                }),
        );
    let mut lines = col().w_full().gap(0.0).padding(12.0);
    if host.console.is_empty() {
        lines = lines.child(
            text(tr("The server's output shows here once it starts."))
                .font(MONO)
                .color(u.faint),
        );
    } else {
        let skip = count.saturating_sub(CONSOLE_DRAWN);
        for line in host.console.iter().skip(skip) {
            let tone = console_tone(u, line);
            lines = lines.child(
                text(line.clone())
                    .font(MONO)
                    .color(tone)
                    .w_full()
                    .wrap_anywhere(),
            );
        }
    }
    let running = host.phase == Phase::Running;
    let command = row()
        .key("host-command")
        .w_full()
        .h(32.0)
        .pl(10.0)
        .pr(2.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .opacity(if running { 1.0 } else { 0.5 })
        .child(text(">").font(MONO).color(u.faint).a11y_hidden())
        .child({
            let input = rune::text_input(&host.command, MONO)
                .key("host-command-input")
                .label(tr("Console command"))
                .single_line()
                .color(u.ink)
                .placeholder(
                    tr("Type a console command, like status").to_string(),
                    u.faint,
                )
                .caret(u.accent, u.accent.alpha(0.20))
                .grow(1.0)
                .min_w(0.0);
            if running {
                input
                    .on_edit(|st: &mut State| &mut st.servers.host.command)
                    .on_submit(|st: &mut State, _| super::send_command(st))
            } else {
                input
            }
        })
        .child(
            u.action(tr("Send"), ActionKind::Ghost)
                .key("host-command-send")
                .cursor(if running {
                    rune::Cursor::Pointer
                } else {
                    rune::Cursor::Default
                })
                .on_press(super::send_command),
        );
    col()
        .w_full()
        .gap(8.0)
        .child(head)
        .child(
            col()
                .key("host-console")
                .w_full()
                .h(260.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(lines),
        )
        .child(command)
        .child(
            text(tr("Type status to list the players on the server."))
                .font(SMALL)
                .color(u.faint),
        )
}

fn console_tone(u: &Ui, line: &str) -> Color {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("exception") || lower.contains("failed") {
        u.rose
    } else if lower.contains("warn") {
        u.amber
    } else {
        u.prose
    }
}

// ---------------------------------------------------------------------------------------------
// Let friends join: firewall, LAN, router
// ---------------------------------------------------------------------------------------------

fn reach_section(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let port = host.port();
    let running = host.phase == Phase::Running;

    // The firewall rule.
    let checking = host.firewall_job.is_running();
    let (step, line) = match (&host.firewall, checking) {
        (_, true) => (Step::Busy, tr("Checking Windows Firewall...").to_string()),
        (Firewall::Present, _) => (
            Step::Done,
            fill(
                tr("Windows Firewall lets TCP and UDP port {} in."),
                &[&port.to_string()],
            ),
        ),
        (Firewall::Partial, _) => (
            Step::Warn,
            fill(
                tr(
                    "Only part of the rule for port {} is there. Add it again to open both TCP and UDP.",
                ),
                &[&port.to_string()],
            ),
        ),
        (Firewall::Missing, _) => (
            Step::Todo,
            fill(
                tr(
                    "No rule for port {} yet. Other PCs cannot reach the server until there is one.",
                ),
                &[&port.to_string()],
            ),
        ),
        (Firewall::Unknown, _) => (Step::Todo, tr("Not checked yet.").to_string()),
    };
    let needs_rule = matches!(
        host.firewall,
        Firewall::Missing | Firewall::Partial | Firewall::Unknown
    );
    let mut firewall_body = col().w_full().gap(2.0).child(detail(u, line));
    if needs_rule {
        firewall_body = firewall_body.child(
            text(tr(
                "Adding the rule shows Windows' admin prompt. Sigil changes nothing else.",
            ))
            .font(SMALL)
            .color(u.faint)
            .wrap()
            .w_full(),
        );
    }
    let firewall = step_row(
        u,
        "host-firewall",
        step,
        tr("Firewall"),
        firewall_body,
        if needs_rule {
            Some(
                u.icon_action(
                    &icons::SHIELD,
                    tr("Add firewall rule"),
                    ActionKind::Secondary,
                )
                .key("host-firewall-add")
                .opacity(if checking { 0.5 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if !st.servers.host.firewall_job.is_running() {
                        st.servers.host.confirm = Some(Confirm::Firewall);
                    }
                }),
            )
        } else if matches!(host.firewall, Firewall::Present) {
            // Close the port again when hosting is done; the same admin prompt.
            Some(
                u.action(tr("Remove rule"), ActionKind::Ghost)
                    .key("host-firewall-remove")
                    .opacity(if checking { 0.5 } else { 1.0 })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(super::remove_firewall),
            )
        } else {
            None
        },
    );

    // This PC and the local network.
    let mut lan = col().w_full().gap(6.0);
    if host.lan.is_empty() {
        lan = lan.child(detail(
            u,
            tr("No local network address found. Is this PC connected?").to_string(),
        ));
    }
    for ip in &host.lan {
        let line = connect_line(ip, port);
        let copy = line.clone();
        lan = lan.child(
            row()
                .key(format!("host-lan-{ip}"))
                .w_full()
                .h(32.0)
                .pl(10.0)
                .pr(2.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(
                    text(line)
                        .font(MONO)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    u.icon_button(&icons::COPY, tr("Copy the connect line"))
                        .key(format!("host-copy-{ip}"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            st.status = tr("Connect line copied.").to_string();
                        }),
                ),
        );
    }
    let join = u
        .icon_action(
            &icons::LOG_IN,
            tr("Join from this PC"),
            ActionKind::Secondary,
        )
        .key("host-join")
        .opacity(if running { 1.0 } else { 0.45 })
        .cursor(if running {
            rune::Cursor::Pointer
        } else {
            rune::Cursor::Default
        })
        .on_press(|st: &mut State| {
            if st.servers.host.phase == Phase::Running {
                super::join_here(st);
            }
        });
    let mut lan_body = col().w_full().gap(8.0);
    let custom = host
        .live_map
        .as_deref()
        .filter(|map| running && host.is_custom(map));
    if let Some(map) = custom {
        lan_body = lan_body.child(
            row()
                .key("host-invite-map")
                .w_full()
                .gap(10.0)
                .items_start()
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.accent.alpha(0.07))
                .border(1.0, u.accent.alpha(0.25))
                .child(icon(&icons::GLOBE, 16.0).color(u.accent).mt(2.0).a11y_hidden())
                .child(
                    text(fill(
                        tr("The server runs your custom map {}. Friends who use Sigil get the map offered when they join. Everyone else installs the map first, then types the line below in Deadlock's console (F7)."),
                        &[map],
                    ))
                    .font(SMALL)
                    .color(u.ink)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
                ),
        );
    } else {
        lan_body = lan_body.child(detail(
            u,
            tr("Friends on the same network type this in Deadlock's console (F7), or use Connect on the Servers tab.").to_string(),
        ));
    }
    let lan_body = lan_body.child(lan);
    let lan_row = step_row(
        u,
        "host-lan",
        if running { Step::Done } else { Step::Blocked },
        tr("Same network"),
        lan_body,
        Some(join),
    );

    // Outside the network: the router.
    let mut steps = col().w_full().gap(4.0);
    let target = host
        .lan
        .first()
        .cloned()
        .unwrap_or_else(|| tr("this PC").to_string());
    for (i, line) in [
        tr("Open your router's settings page. Its address is often on a sticker on the router, like 192.168.1.1."),
        &fill(tr("Forward TCP and UDP port {} to {}."), &[&port.to_string(), &target]),
        &fill(tr("Your public IP is on the router's status page. Friends join with connect <public IP>:{}."), &[&port.to_string()]),
    ]
    .into_iter()
    .enumerate()
    {
        steps = steps.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text(format!("{}.", i + 1)).font(SMALL).color(u.faint).w(14.0))
                .child(text(line.to_string()).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).wrap()),
        );
    }
    let router = step_row(
        u,
        "host-router",
        Step::Info,
        tr("Friends outside your network"),
        col()
            .w_full()
            .gap(6.0)
            .child(detail(u, tr("Your router has to pass the port on to this PC. Sigil does not change router settings or look up your public IP.").to_string()))
            .child(steps),
        None,
    )
    .border_sides([0.0, 0.0, 0.0, 0.0], Color::TRANSPARENT);

    col()
        .key("host-reach")
        .w_full()
        .gap(10.0)
        .child(section_head(u, tr("Let friends join"), "", None))
        .child(
            u.panel()
                .w_full()
                .child(firewall)
                .child(lan_row)
                .child(router),
        )
}

// ---------------------------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------------------------

fn plugins_section(s: &State, u: &Ui) -> El {
    let host = &s.servers.host;
    let mut body = col().key("host-plugins").w_full().gap(10.0);
    let meta = match &host.plugins {
        Some(Ok(list)) => fill(
            tr("{} of {} on"),
            &[
                &list.iter().filter(|p| p.on).count().to_string(),
                &list.len().to_string(),
            ],
        ),
        _ => String::new(),
    };
    body = body.child(section_head(u, tr("Plugins"), &meta, None)).child(
        text(tr("Plugins are .NET code that runs on this PC with the server's rights. Turn on only plugins you trust. A change applies on the next start; Deadworks reloads a replaced plugin while it runs."))
            .font(SMALL)
            .color(u.muted)
            .wrap()
            .w_full(),
    );
    let list = match &host.plugins {
        None => text(tr("Plugins show here once Deadworks is installed."))
            .font(BODY)
            .color(u.faint),
        Some(Err(error)) => text(error.clone()).font(BODY).color(u.rose).wrap(),
        Some(Ok(list)) if list.is_empty() => text(tr("No plugins in the server's plugins folder."))
            .font(BODY)
            .color(u.faint),
        Some(Ok(list)) => {
            let busy = host.plugin_job.is_running();
            let mut panel = u.panel().w_full();
            let last = list.len().saturating_sub(1);
            for (i, plugin) in list.iter().enumerate() {
                let name = plugin.name.clone();
                let on = plugin.on;
                panel = panel.child(
                    row()
                        .key(format!("host-plugin-{}", plugin.name))
                        .w_full()
                        .h(44.0)
                        .px(16.0)
                        .gap(12.0)
                        .center_y()
                        .border_sides([0.0, 0.0, if i == last { 0.0 } else { 1.0 }, 0.0], u.hair)
                        .child(icon(&icons::PACKAGE, 16.0).color(u.faint).a11y_hidden())
                        .child(
                            text(plugin.name.clone())
                                .font(BODY_MED)
                                .color(u.ink)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(
                            text(if on { tr("On") } else { tr("Off") })
                                .font(SMALL)
                                .color(u.faint),
                        )
                        .child(
                            u.toggle(on)
                                .key(format!("host-plugin-toggle-{}", plugin.name))
                                .label(fill(tr("Turn {} on or off"), &[&plugin.name]))
                                .opacity(if busy { 0.5 } else { 1.0 })
                                .cursor(rune::Cursor::Pointer)
                                .on_press(move |st: &mut State| {
                                    super::set_plugin(st, name.clone(), !on)
                                }),
                        ),
                );
            }
            panel
        }
    };
    body.child(list)
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

fn close(st: &mut State) {
    st.servers.host.confirm = None;
}

fn shell(u: &Ui, key: &str, title: &str, body: El) -> El {
    u.dialog_shell()
        .key(key.to_string())
        .role("dialog")
        .label(title.to_string())
        .w(500.0)
        .padding(24.0)
        .gap(12.0)
        .modal(true, close)
        .on_key(|st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(text(title.to_string()).font(H1).color(u.ink).wrap())
        .child(body)
}

fn buttons(u: &Ui, confirm: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(spacer())
        .child(
            u.action(tr("Cancel"), ActionKind::Ghost)
                .key("host-dialog-cancel")
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        )
        .child(confirm)
}

/// The open confirm dialog on the Host tab, if any.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    let host = &s.servers.host;
    match host.confirm.as_ref()? {
        Confirm::ServerFiles { update } => Some(files_dialog(s, u, *update)),
        Confirm::Firewall => Some(firewall_dialog(u, host.port())),
    }
}

fn files_dialog(s: &State, u: &Ui, update: bool) -> El {
    let host = &s.servers.host;
    let title = if update {
        tr("Update the server files")
    } else {
        tr("Download the server files")
    };
    let dir = host
        .server_dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let mut body = col()
        .w_full()
        .gap(10.0)
        .child(
            text(if update {
                tr("SteamCMD opens in its own window and brings Deadlock's dedicated server (app 1422450) up to the newest build in:")
            } else {
                tr("SteamCMD opens in its own window and downloads Deadlock's dedicated server (app 1422450, about 35 GB) into:")
            })
            .font(BODY)
            .color(u.prose)
            .wrap(),
        )
        .child(text(dir).font(MONO).color(u.ink).wrap_anywhere().w_full())
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .child(icon(&icons::KEY_ROUND, 16.0).color(u.accent).mt(2.0).a11y_hidden())
                .child(
                    text(tr("Sign in there with a Steam account that owns Deadlock. SteamCMD asks for the password and any Steam Guard code in its own window. Sigil never sees, asks for or keeps them."))
                        .font(BODY)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                ),
        )
        .child(text(tr("Steam account name")).font(BODY_MED).color(u.ink).pt(4.0))
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
                    rune::text_input(&host.steam_user, BODY)
                        .key("host-steam-user")
                        .label(tr("Steam account name"))
                        .single_line()
                        .color(u.ink)
                        .placeholder(tr("The name you sign in with, not your profile name").to_string(), u.faint)
                        .caret(u.accent, u.accent.alpha(0.20))
                        .on_edit(|st: &mut State| &mut st.servers.host.steam_user)
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    if let Some(error) = &host.error {
        body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
    }
    body = body.child(
        text(tr(
            "Run it again after every Deadlock patch so friends on the new build can join.",
        ))
        .font(SMALL)
        .color(u.faint)
        .wrap(),
    );
    body = body.child(buttons(
        u,
        u.icon_action(&icons::TERMINAL, tr("Open SteamCMD"), ActionKind::Primary)
            .key("host-open-steamcmd")
            .cursor(rune::Cursor::Pointer)
            .on_press(super::open_steamcmd),
    ));
    shell(u, "host-files-dialog", title, body)
}

fn firewall_dialog(u: &Ui, port: u16) -> El {
    let body = col()
        .w_full()
        .gap(10.0)
        .child(
            text(fill(
                tr("Sigil asks Windows to let incoming TCP and UDP on port {} reach the server. Nothing else changes."),
                &[&port.to_string()],
            ))
            .font(BODY)
            .color(u.prose)
            .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .padding(12.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .child(icon(&icons::SHIELD, 16.0).color(u.amber).mt(2.0).a11y_hidden())
                .child(
                    text(tr("Windows shows its admin prompt next. Press Yes there to add the rule; No leaves the firewall as it is."))
                        .font(BODY)
                        .color(u.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                ),
        )
        .child(buttons(
            u,
            u.icon_action(&icons::SHIELD, tr("Add rule"), ActionKind::Primary)
                .key("host-firewall-confirm")
                .cursor(rune::Cursor::Pointer)
                .on_press(super::add_firewall),
        ));
    shell(u, "host-firewall-dialog", tr("Add a firewall rule"), body)
}

// ---------------------------------------------------------------------------------------------
// Map picker
// ---------------------------------------------------------------------------------------------

fn map_row(u: &Ui, name: &str, detail: &str, on: bool) -> El {
    let pick = name.to_string();
    row()
        .key(format!("host-map-item-{name}"))
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .focusable(true)
        .role("menuitem")
        .checked(on)
        .label(name.to_string())
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| {
            st.servers.host.map = rune::TextEdit::new(pick.clone());
            st.servers.host.map_menu = false;
        })
        .child(
            text(name.to_string())
                .font(MONO)
                .color(if on { u.ink } else { u.prose })
                .grow(1.0)
                .min_w(0.0)
                .truncate()
                .a11y_hidden(),
        )
        .child(
            text(detail.to_string())
                .font(SMALL)
                .color(u.faint)
                .a11y_hidden(),
        )
        .child(if on {
            icon(&icons::CHECK, 14.0).color(u.muted).a11y_hidden()
        } else {
            el().size(14.0, 14.0)
        })
}

fn menu_head(u: &Ui, label: &str) -> El {
    row()
        .w_full()
        .h(28.0)
        .px(10.0)
        .center_y()
        .child(text(label.to_string()).font(SMALL).color(u.faint))
}

/// The map menu under the Map field: the stock maps, then the player's custom maps.
pub fn overlay(s: &State, u: &Ui) -> Option<El> {
    let host = &s.servers.host;
    if !host.map_menu || host.running() || s.screen != crate::model::Screen::Servers {
        return None;
    }
    let current = host.map.text.trim();
    let mut items = col().w_full().child(menu_head(u, tr("Deadlock's maps")));
    for name in super::bridge::STOCK_MAPS {
        items = items.child(map_row(u, name, "", *name == current));
    }
    items = items.child(menu_head(u, tr("Your custom maps")));
    let custom = host.custom_maps();
    if custom.is_empty() {
        items = items.child(
            text(tr("None installed. Get maps on the Maps tab."))
                .font(SMALL)
                .color(u.muted)
                .px(10.0)
                .py(6.0),
        );
    }
    for name in &custom {
        let detail = if host.content_for(name).is_some() {
            tr("Copied in at Start")
        } else if host.on_server(name) {
            tr("On the server")
        } else {
            tr("Installed")
        };
        items = items.child(map_row(u, name, detail, name == current));
    }
    Some(
        col()
            .key("host-map-menu")
            .role("menu")
            .w(300.0)
            .padding(4.0)
            .fill(u.raised)
            .radius(crate::ui::r(10.0))
            .border(1.0, u.hair_strong)
            .shadow(12.0, 32.0, Color::BLACK.alpha(0.5))
            .anchored("host-map", rune::Side::Bottom, rune::Align::End, 6.0)
            .modal(true, |st: &mut State| st.servers.host.map_menu = false)
            .on_key(|st: &mut State, ev: &KeyEvent, _| {
                if ev.key == Key::Escape {
                    st.servers.host.map_menu = false;
                    true
                } else {
                    false
                }
            })
            .child(
                col()
                    .w_full()
                    .max_h(360.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(items),
            ),
    )
}
