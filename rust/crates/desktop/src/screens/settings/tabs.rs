//! The categories added in round 3: General's start and language rows, Integrations, the
//! match sharing part of Privacy, Network, Logs and Experimental, plus the actions behind them.
//! Row helpers and `say` come from the parent module; every title and line they get passes
//! through `i18n::tr`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use deadlock_core::steam_users::{self, SteamUser};
use rune::{El, TextEdit, Tx, col, el, row, text};

use super::discord::{self, Activity, Status};
use super::i18n::{self, Lang, fill, tr};
use super::log::{self, Level};
use super::sharing;
use super::{
    Confirm, LIST_GAP, ROW_H, button, detail, disabled, field, say, section, segmented, setting_row,
    stacked_row, status_word, toggle_row, trailing,
};
use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::prefs::{self, Channel, DateFormat, FLAGS};
use crate::ui::{ActionKind, BODY, BODY_MED, MONO, SMALL, Ui};

// --- general ---------------------------------------------------------------------

pub fn general(s: &State, u: &Ui) -> El {
    let langs: Vec<(Lang, &str)> = Lang::ALL
        .iter()
        .map(|l| (*l, if *l == Lang::System { "System" } else { l.native() }))
        .collect();
    let dates: Vec<(DateFormat, &str)> = DateFormat::ALL.iter().map(|f| (*f, f.label())).collect();
    let startup: Vec<(Screen, &str)> = prefs::STARTUP_SCREENS
        .iter()
        .filter(|screen| s.prefs.page_on(**screen))
        .map(|screen| (*screen, screen.title()))
        .collect();
    let launch: Vec<(prefs::OnLaunch, &str)> =
        prefs::OnLaunch::ALL.iter().map(|o| (*o, o.label())).collect();
    let autostart_on = s.settings.autostart_on.unwrap_or(false);
    let lang_line = match s.prefs.language {
        Lang::System => fill(
            tr("Follows your computer: {}."),
            &[s.prefs.language.resolve().native()],
        ),
        _ => tr("English is the source; other languages fall back to it.").to_string(),
    };

    col()
        .w_full()
        .gap(LIST_GAP)
        .child(section(
            u,
            "Language and dates",
            vec![
                setting_row(
                    u,
                    "settings-language",
                    "Language",
                    detail(u, &lang_line),
                    super::segmented_w(u, "settings-language-pick", &langs, s.prefs.language, |st, lang| {
                        prefs::edit(st, |p| p.language = lang);
                        i18n::set(lang);
                    }, super::CONTROL_W),
                ),
                setting_row(
                    u,
                    "settings-date-format",
                    "Dates",
                    detail(u, ""),
                    super::segmented_w(u, "settings-date-pick", &dates, s.prefs.date_format, |st, f| {
                        prefs::edit(st, |p| p.date_format = f)
                    }, super::CONTROL_W),
                ),
            ],
        ))
        .child(section(
            u,
            "Starting Sigil",
            vec![
                setting_row(
                    u,
                    "settings-startup",
                    "Open on",
                    detail(u, "The page Sigil shows when it starts."),
                    super::segmented_w(u, "settings-startup-pick", &startup, s.prefs.startup, |st, screen| {
                        prefs::edit(st, |p| p.startup = screen)
                    }, super::CONTROL_W),
                ),
                toggle_row(
                    u,
                    "settings-autostart",
                    "Start with your computer",
                    "",
                    autostart_on,
                    toggle_autostart,
                ),
                disabled(
                    toggle_row(
                        u,
                        "settings-start-minimized",
                        "Start minimized",
                        "When it opens at sign-in, Sigil waits in the Dock or taskbar.",
                        s.prefs.start_minimized,
                        |st| {
                            prefs::edit(st, |p| p.start_minimized = !p.start_minimized);
                            if st.settings.autostart_on == Some(true) {
                                apply_autostart(st, true);
                            }
                        },
                    ),
                    !autostart_on,
                ),
            ],
        ))
        .child(section(
            u,
            "Launching the game",
            vec![
                setting_row(
                    u,
                    "settings-on-launch",
                    "When the game starts",
                    detail(u, "What Sigil does once Steam starts Deadlock."),
                    super::segmented_w(u, "settings-on-launch-pick", &launch, s.prefs.on_launch, |st, o| {
                        prefs::edit(st, |p| p.on_launch = o)
                    }, super::CONTROL_W),
                ),
                toggle_row(
                    u,
                    "settings-confirm-launch",
                    "Ask before launching",
                    "Show a confirm with your mod count before Steam starts the game.",
                    s.prefs.confirm_launch,
                    |st| prefs::edit(st, |p| p.confirm_launch = !p.confirm_launch),
                ),
                toggle_row(
                    u,
                    "settings-single-launch",
                    "One launch button",
                    "A single Launch button in the sidebar, with a switch between modded and vanilla.",
                    s.prefs.single_launch,
                    |st| prefs::edit(st, |p| p.single_launch = !p.single_launch),
                ),
            ],
        ))
}

fn toggle_autostart(st: &mut State) {
    let on = !st.settings.autostart_on.unwrap_or(false);
    apply_autostart(st, on);
}

fn apply_autostart(st: &mut State, on: bool) {
    if st.demo || rune::is_headless() {
        st.settings.autostart_on = Some(on);
        return;
    }
    match super::autostart::set(on, st.prefs.start_minimized) {
        Ok(()) => {
            st.settings.autostart_on = Some(super::autostart::is_enabled());
            log::info(format!("Start with the computer: {on}"));
        }
        Err(error) => {
            log::error(format!("Start with the computer failed: {error}"));
            say(st, fill(tr("The login item could not be changed: {}"), &[&error]), true);
        }
    }
}

/// The sidebar's single launch control, when "One launch button" is on.
pub fn single_launch(s: &State, u: &Ui) -> El {
    let vanilla = s.settings.launch_vanilla_pick;
    let label = if vanilla { "Launch vanilla" } else { "Launch modded" };
    row()
        .key("launch-single")
        .w_full()
        .h(36.0)
        .gap(4.0)
        .child(crate::app::launch_lock(s,
            row()
                .key("launch-single-go")
                .grow(1.0)
                .h_full()
                .px(12.0)
                .gap(10.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.accent)
                .hover(rune::Color::WHITE.alpha(0.10))
                .focusable(true)
                .role("button")
                .label(tr(label))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                    crate::app::launch_from_settings(st, tx, vanilla)
                })
                .child(rune::icon(&icons::PLAY, 14.0).color(u.accent_ink).a11y_hidden())
                .child(text(tr(label)).font(BODY_MED).color(u.accent_ink).grow(1.0).truncate()),
        ))
        .child(
            u.icon_button(&icons::REFRESH_CW, tr("Switch between modded and vanilla"))
                .key("launch-single-swap")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.settings.launch_vanilla_pick = !st.settings.launch_vanilla_pick
                }),
        )
}

// --- integrations ----------------------------------------------------------------

pub fn integrations(s: &State, u: &Ui) -> El {
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(discord_section(s, u))
        .child(steam_section(s, u))
        .child(gamebanana_section(s, u))
        .child(links_section(u))
        .child(section(
            u,
            "Sharing",
            vec![
                setting_row(
                    u,
                    "settings-share-profiles",
                    "Profile share codes",
                    detail(u, "Send a mod setup to a friend as a code, from Profiles."),
                    button(u, "settings-share-profiles-open", "Open Profiles", ActionKind::Secondary)
                        .on_press(|st: &mut State| {
                            super::close(st);
                            st.go(Screen::Profiles);
                        }),
                ),
                setting_row(
                    u,
                    "settings-community",
                    "Community gallery",
                    detail(u, "Publishing and liking profiles needs a Sigil service that does not exist yet."),
                    status_word(u, "Not available", false),
                ),
            ],
        ))
}

/// "Install with Sigil" links (`crate::deeplink`): GameBanana's 1-Click button opens the mod
/// here, and the player still confirms the install.
fn links_section(u: &Ui) -> El {
    use crate::deeplink::{self, Registration};
    let on = deeplink::links_on();
    let problem = on && deeplink::registration() == Registration::Failed;
    let switch = u
        .toggle(on)
        .key("settings-links-switch")
        .label(tr("Open Install with Sigil links"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            let on = !deeplink::links_on();
            if let Err(error) = deeplink::set_links_on(on) {
                say(st, error, true);
            }
        });
    let mut controls = Vec::new();
    if problem {
        controls.push(status_word(u, "Not registered", true));
    }
    controls.push(switch);
    section(
        u,
        "Links",
        vec![setting_row(
            u,
            "settings-links",
            "Open Install with Sigil links",
            detail(u, "GameBanana's Sigil button opens the mod here. You confirm every install.").wrap(),
            trailing(controls),
        )],
    )
}

fn discord_section(s: &State, u: &Ui) -> El {
    let configured = discord::app_id().is_some();
    let on = s.prefs.discord && configured;
    let line = if !configured {
        "Not set up in this build: Sigil has no Discord application id yet."
    } else {
        "Shows what you do in Sigil on your Discord profile, through the Discord app on this computer."
    };
    let status = match (&s.settings.presence.status, on) {
        (_, false) => None,
        (Status::Connected, _) => Some(("Connected", false)),
        (Status::NotConfigured, _) => Some(("Not set up", true)),
        _ => Some(("Waiting for Discord", false)),
    };
    let mut switch = u
        .toggle(on)
        .key("settings-discord-switch")
        .label(tr("Discord Rich Presence"))
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            prefs::edit(st, |p| p.discord = !p.discord);
            if !st.prefs.discord {
                st.settings.presence.stop();
            }
        });
    if !configured {
        switch = disabled(switch, true);
    }
    let mut controls = Vec::new();
    if let Some((word, warn)) = status {
        controls.push(status_word(u, word, warn));
    }
    controls.push(switch);
    let preview = presence_activity(s);
    let preview_line = match &preview.state {
        Some(state) => format!("{} · {state}", preview.details),
        None => preview.details.clone(),
    };
    let mut rows = vec![setting_row(
        u,
        "settings-discord",
        "Discord Rich Presence",
        detail(u, line).wrap(),
        trailing(controls),
    )];
    if !configured {
        // Presence needs the application id; the lines can still be written.
        rows.extend(super::templates::rows(s, u, false));
        return section(u, "Discord", rows);
    }
    rows.push(disabled(
        toggle_row(
            u,
            "settings-discord-details",
            "Show what you are doing",
            "Off shows only that you are using Sigil.",
            s.prefs.discord_details,
            |st| prefs::edit(st, |p| p.discord_details = !p.discord_details),
        ),
        !on,
    ));
    rows.push(disabled(
        toggle_row(
            u,
            "settings-discord-mods",
            "Show your mod count",
            "Adds how many mods are on, and whether the game runs modded.",
            s.prefs.discord_mods,
            |st| prefs::edit(st, |p| p.discord_mods = !p.discord_mods),
        ),
        !on || !s.prefs.discord_details,
    ));
    rows.extend(super::templates::rows(s, u, on));
    rows.push(setting_row(
        u,
        "settings-discord-preview",
        "Discord shows",
        text(preview_line).font(MONO).color(u.muted).truncate(),
        el(),
    ));
    section(u, "Discord", rows)
}

/// What Sigil would show on Discord right now. English on purpose: friends see it, and
/// presence text conventionally stays in one language.
pub fn presence_activity(s: &State) -> Activity {
    let since = s.settings.session_start;
    if !s.prefs.discord_details {
        return Activity {
            details: "Using Sigil".into(),
            state: None,
            since,
        };
    }
    let mods = s.addons.enabled_count();
    let count = |n: usize| format!("{n} mod{} on", if n == 1 { "" } else { "s" });
    if s.settings.game_running
        && let Some(activity) = super::templates::game_activity(s, since)
    {
        return activity;
    }
    if s.settings.game_running {
        let vanilla =
            deadlock_core::launch::stash_status(&crate::data::data_dir()).is_some() || mods == 0;
        let state = match (vanilla, s.prefs.discord_mods) {
            (true, _) => "Vanilla".to_string(),
            (false, true) => format!("Modded, {}", count(mods)),
            (false, false) => "Modded".to_string(),
        };
        return Activity {
            details: "Playing Deadlock".into(),
            state: Some(state),
            since,
        };
    }
    let details = if s.settings.open {
        "Changing settings"
    } else {
        match s.screen {
            Screen::Browse => "Browsing mods",
            Screen::Installed | Screen::Updates => "Managing mods",
            Screen::Locker | Screen::Character => "In the Locker",
            Screen::Crosshair => "Designing a crosshair",
            Screen::Autoexec => "Editing the autoexec",
            Screen::Performance => "Tuning performance",
            Screen::Stats => "Checking stats",
            Screen::Conflicts => "Sorting out mod overlap",
            Screen::Profiles => "Managing profiles",
            Screen::Servers => "Browsing servers",
            Screen::Launch => "Getting ready to play",
            Screen::Downloads => "Downloading mods",
            _ => "Building a mod",
        }
    };
    if let Some(activity) = super::templates::app_activity(details, if s.prefs.discord_mods { mods } else { 0 }, since) {
        return activity;
    }
    Activity {
        details: details.into(),
        state: (s.prefs.discord_mods && mods > 0).then(|| count(mods)),
        since,
    }
}

/// Integrations' pointer to the one Steam section, which lives in Game.
fn steam_section(_s: &State, u: &Ui) -> El {
    section(
        u,
        "Steam",
        vec![setting_row(
            u,
            "settings-steam-elsewhere",
            "Steam account",
            detail(u, "The linked account and the Steam folder are in Game."),
            button(u, "settings-steam-open-game", "Open Game", ActionKind::Secondary)
                .on_press(|st: &mut State| super::select(st, super::Tab::Game)),
        )],
    )
}

/// The Steam accounts on this computer, for Game's Steam section.
pub fn steam_account_rows(s: &State, u: &Ui) -> Vec<El> {
    let users = s.settings.steam_users.as_deref().unwrap_or(&[]);
    let linked = s.prefs.steam_account;
    let mut rows = Vec::new();
    if users.is_empty() && linked.is_none() {
        rows.push(setting_row(
            u,
            "settings-steam-none",
            "Steam account",
            detail(u, "No Steam sign-in found on this computer. Stats takes a profile link instead."),
            button(u, "settings-steam-rescan", "Look again", ActionKind::Secondary)
                .on_press(read_steam_users),
        ));
    }
    let mut shown: Vec<(u32, String, bool)> = users
        .iter()
        .map(|user| (user.account_id, user.persona.clone(), user.most_recent))
        .collect();
    if let Some(id) = linked
        && !shown.iter().any(|(a, _, _)| *a == id)
    {
        shown.insert(0, (id, id.to_string(), false));
    }
    for (account, name, recent) in shown {
        let is_linked = linked == Some(account);
        let line = if is_linked {
            fill(tr("Linked. Stats opens on this account. Id {}."), &[&account.to_string()])
        } else if recent {
            fill(tr("Signed in to Steam most recently. Id {}."), &[&account.to_string()])
        } else {
            fill(tr("Id {}."), &[&account.to_string()])
        };
        let profile = steam_users::profile_url(account);
        let stats = format!("https://statlocker.gg/profile/{account}");
        let mut actions = vec![
            u.icon_button(&icons::EXTERNAL_LINK, tr("Open the Steam profile"))
                .key(format!("settings-steam-open-{account}"))
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &profile)),
        ];
        if is_linked {
            actions.insert(
                0,
                button(u, &format!("settings-steam-stats-{account}"), "Stats", ActionKind::Secondary)
                    .on_press(move |st: &mut State| {
                        super::close(st);
                        crate::data::load_stats_account(st, account as u64);
                        st.go(Screen::Stats);
                    }),
            );
            actions.insert(
                1,
                button(u, &format!("settings-steam-statlocker-{account}"), "Statlocker", ActionKind::Ghost)
                    .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &stats)),
            );
            actions.push(
                button(u, &format!("settings-steam-unlink-{account}"), "Unlink", ActionKind::Ghost)
                    .on_press(|st: &mut State| prefs::edit(st, |p| p.steam_account = None)),
            );
        } else {
            actions.insert(
                0,
                button(u, &format!("settings-steam-link-{account}"), "Link", ActionKind::Secondary)
                    .on_press(move |st: &mut State| {
                        prefs::edit(st, |p| p.steam_account = Some(account));
                        say(st, tr("Linked. Stats opens on this account."), false);
                    }),
            );
        }
        rows.push(setting_row(
            u,
            &format!("settings-steam-{account}"),
            &name,
            detail(u, &line),
            trailing(actions),
        ));
    }
    rows
}

pub fn read_steam_users(st: &mut State) {
    if st.demo {
        st.settings.steam_users = Some(vec![
            SteamUser {
                steam_id64: steam_users::account_to_id64(81_220_464),
                account_id: 81_220_464,
                persona: "Kalo".into(),
                most_recent: true,
            },
            SteamUser {
                steam_id64: steam_users::account_to_id64(40_312_977),
                account_id: 40_312_977,
                persona: "Smurf".into(),
                most_recent: false,
            },
        ]);
        return;
    }
    let mut roots = deadlock_core::discovery::default_steam_roots();
    if let Some(custom) = super::options::get().steam_root.clone() {
        roots.insert(0, PathBuf::from(custom));
    }
    if let Some(root) = st.game_root.as_ref().and_then(|g| g.ancestors().nth(3)) {
        roots.push(root.to_path_buf());
    }
    st.settings.steam_users = Some(steam_users::detect(&roots));
}

fn gamebanana_section(s: &State, u: &Ui) -> El {
    let row_el = match &s.prefs.gamebanana {
        Some((id, name)) => {
            let profile = format!("https://gamebanana.com/members/{id}");
            setting_row(
                u,
                "settings-gamebanana",
                if name.is_empty() { "GameBanana" } else { name },
                detail(u, &fill(tr("Linked GameBanana member {}."), &[&id.to_string()])),
                trailing(vec![
                    button(u, "settings-gamebanana-open", "Open profile", ActionKind::Secondary)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| open_url(st, tx, &profile)),
                    button(u, "settings-gamebanana-unlink", "Unlink", ActionKind::Ghost)
                        .on_press(|st: &mut State| prefs::edit(st, |p| p.gamebanana = None)),
                ]),
            )
        }
        None => {
            let typed = parse_member(&s.settings.gb_edit.text);
            let busy = s.settings.gb_job.is_running();
            stacked_row(
                u,
                "settings-gamebanana",
                "GameBanana account",
                "Paste your profile link to open your page and uploads from Sigil. Sigil only reads your public name.",
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        field(
                            rune::text_input(&s.settings.gb_edit, MONO)
                                .key("settings-gamebanana-input")
                                .label(tr("GameBanana profile link or member id"))
                                .single_line()
                                .grow(1.0)
                                .color(u.ink)
                                .placeholder("gamebanana.com/members/1382", u.faint)
                                .caret(u.accent, u.accent.alpha(0.22))
                                .on_edit(|st: &mut State| &mut st.settings.gb_edit),
                            u,
                        )
                        .grow(1.0)
                        .min_w(0.0),
                    )
                    .child(disabled(
                        button(
                            u,
                            "settings-gamebanana-link",
                            if busy { "Linking" } else { "Link" },
                            ActionKind::Secondary,
                        )
                        .on_press(link_gamebanana),
                        typed.is_none() || busy,
                    )),
            )
        }
    };
    section(u, "GameBanana", vec![row_el])
}

/// A member id from `1382`, `gamebanana.com/members/1382` or a full profile URL.
pub fn parse_member(input: &str) -> Option<u64> {
    let input = input.trim().trim_end_matches('/');
    let tail = match input.find("/members/") {
        Some(at) => &input[at + "/members/".len()..],
        None => input,
    };
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    let id = digits.parse::<u64>().ok().filter(|id| *id > 0)?;
    (digits.len() == tail.len() || tail[digits.len()..].starts_with(['/', '?', '#'])).then_some(id)
}

fn link_gamebanana(st: &mut State) {
    let Some(id) = parse_member(&st.settings.gb_edit.text) else {
        return say(st, tr("Paste a profile link like gamebanana.com/members/1382."), true);
    };
    if st.demo || rune::is_headless() {
        prefs::edit(st, |p| p.gamebanana = Some((id, String::new())));
        return;
    }
    let wake = st.wake.clone();
    st.settings.gb_job.start(wake, move || member_name(id).map(|name| (id, name)));
}

fn member_name(id: u64) -> Result<String, String> {
    let url = format!("https://gamebanana.com/apiv11/Member/{id}/ProfilePage");
    let mut response = deadlock_core::net::agent(Duration::from_secs(15))
        .get(&url)
        .header("User-Agent", "Sigil")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "No GameBanana member has that id.".to_string(),
            other => other.to_string(),
        })?;
    let value: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    Ok(value["_sName"].as_str().unwrap_or_default().to_string())
}

pub fn open_url(st: &mut State, tx: &mut Tx, url: &str) {
    if let Err(error) = tx.open_url(url) {
        say(st, format!("{error}"), true);
    }
}

// --- privacy: match sharing ------------------------------------------------------

pub fn match_sharing(s: &State, u: &Ui) -> El {
    let on = s.prefs.share_matches;
    let record = s.settings.sharing.record.as_ref();
    let total = record.map_or(0, |r| r.total_sent());
    let running = s.settings.sharing.job.is_running();
    let status = if s.prefs.offline {
        tr("Paused while offline mode is on.").to_string()
    } else if running {
        tr("Looking through Steam's cache").to_string()
    } else {
        match &s.settings.sharing.last {
            Some(Err(error)) => fill(tr("The last upload failed: {}"), &[error]),
            Some(Ok((found, sent))) => fill(
                tr("Last run: {} matches in Steam's cache, {} new sent. {} sent in all."),
                &[&found.to_string(), &sent.to_string(), &total.to_string()],
            ),
            None if total > 0 => fill(tr("{} matches sent so far."), &[&total.to_string()]),
            None => tr("Nothing sent yet.").to_string(),
        }
    };
    let minutes: Vec<(u32, &str)> = vec![(15, "15 min"), (30, "30 min"), (60, "1 h")];
    let mut rows = vec![
        toggle_row(
            u,
            "settings-share-matches",
            "Share match data with deadlock-api",
            "Helps the community stats sites see more matches. Off by default.",
            on,
            |st| {
                prefs::edit(st, |p| p.share_matches = !p.share_matches);
                if st.prefs.share_matches {
                    st.settings.sharing.last_run = None;
                }
            },
        ),
        stacked_row(
            u,
            "settings-share-what",
            "What is sent",
            "When the game shows you a match, Steam downloads its replay data and keeps a copy in its download cache. Sigil reads the web addresses of those copies and sends four numbers per match to api.deadlock-api.com: the match id, the replay server number, and the metadata or replay key. The submitter is recorded as Sigil. No Steam account, name, id, file or anything else leaves your computer, and Sigil never signs in to Steam.",
            el(),
        ),
        disabled(
            setting_row(
                u,
                "settings-share-every",
                "Look every",
                detail(u, "While Sigil is open."),
                segmented(u, "settings-share-every-pick", &minutes, s.prefs.share_minutes, |st, m| {
                    prefs::edit(st, |p| p.share_minutes = m)
                }),
            ),
            !on,
        ),
        setting_row(
            u,
            "settings-share-status",
            "Status",
            detail(u, &status),
            trailing(vec![
                disabled(
                    button(u, "settings-share-now", "Share now", ActionKind::Secondary)
                        .on_press(sharing::run_now),
                    !on || running || s.prefs.offline,
                ),
                disabled(
                    button(u, "settings-share-forget", "Forget", ActionKind::Ghost)
                        .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::ForgetSharing)),
                    record.is_none_or(|r| r.log.is_empty() && r.sent.is_empty()),
                ),
            ]),
        ),
    ];
    if let Some(record) = record {
        for (index, batch) in record.log.iter().take(8).enumerate() {
            let ids: Vec<String> = batch
                .matches
                .iter()
                .take(6)
                .map(|(id, kind)| format!("{id} {}", tr(kind)))
                .collect();
            let more = batch.matches.len().saturating_sub(6);
            let mut line = ids.join(", ");
            if more > 0 {
                line.push_str(&fill(tr(" and {} more"), &[&more.to_string()]));
            }
            let title = format!(
                "{} · {}",
                date_label(s, batch.at),
                match &batch.error {
                    Some(_) => tr("Failed").to_string(),
                    None => fill(tr("{} sent"), &[&batch.matches.len().to_string()]),
                }
            );
            rows.push(setting_row(
                u,
                &format!("settings-share-log-{index}"),
                &title,
                match &batch.error {
                    Some(error) => text(error.clone()).font(SMALL).color(u.rose).truncate(),
                    None => text(line).font(MONO).color(u.muted).truncate().selectable(true),
                },
                el(),
            ));
        }
    }
    section(u, "Match data sharing", rows)
}

/// An absolute date and time for a Unix second, in the chosen format (UTC).
pub fn date_label(s: &State, at: u64) -> String {
    let days = (at / 86_400) as i64;
    let (y, m, d) = civil(days);
    let rem = at % 86_400;
    format!(
        "{} {:02}:{:02}",
        s.prefs.date_format.format(y, m, d),
        rem / 3600,
        rem % 3600 / 60
    )
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

// --- network ---------------------------------------------------------------------

pub fn network(s: &State, u: &Ui) -> El {
    let updates: Vec<(u32, &str)> = vec![(0, "At startup"), (6, "Every 6 h"), (24, "Daily")];
    let catalog: Vec<(u32, &str)> = vec![(0, "Off"), (15, "15 min"), (30, "30 min"), (60, "1 h")];
    let proxy_typed = s.settings.proxy_edit.text.trim().to_string();
    let proxy_valid = proxy_typed.is_empty() || deadlock_core::net::validate_proxy(&proxy_typed).is_ok();
    let proxy_saved = s.prefs.proxy.clone().unwrap_or_default();
    let proxy_line = match (&s.prefs.proxy, proxy_valid) {
        (_, false) => deadlock_core::net::validate_proxy(&proxy_typed)
            .err()
            .map(|e| tr(&e).to_string())
            .unwrap_or_default(),
        (Some(p), _) => fill(tr("Using {} for Browse and mod downloads."), &[p]),
        (None, _) => tr("Browse and mod downloads connect directly.").to_string(),
    };
    // SOCKS5 (wave 4, DMM's third proxy kind): say where names resolve.
    let proxy_line = match proxy_scheme(&proxy_typed) {
        Some(ProxyScheme::Socks5) if proxy_valid => {
            format!("{proxy_line} {}", tr("SOCKS5: names resolve on this PC."))
        }
        Some(ProxyScheme::Socks5h) if proxy_valid => {
            format!("{proxy_line} {}", tr("SOCKS5h: names resolve on the proxy."))
        }
        _ => proxy_line,
    };
    let scheme = proxy_scheme(&proxy_typed).unwrap_or(ProxyScheme::Http);
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(section(
            u,
            "",
            vec![toggle_row(
                u,
                "settings-offline",
                "Offline mode",
                "Stops every connection Sigil makes. Installed mods and local tools keep working.",
                s.prefs.offline,
                |st| {
                    prefs::edit(st, |p| p.offline = !p.offline);
                    super::apply_offline(st.prefs.offline);
                    log::info(format!("Offline mode: {}", st.prefs.offline));
                },
            )],
        ))
        .child(disabled(
            section(
                u,
                "In the background",
                vec![
                    disabled(
                        setting_row(
                            u,
                            "settings-net-update-every",
                            "Update check",
                            detail(u, "How often, while Sigil is open."),
                            segmented(u, "settings-net-update-pick", &updates, s.prefs.update_hours, |st, h| {
                                prefs::edit(st, |p| p.update_hours = h)
                            }),
                        ),
                        !s.prefs.check_updates,
                    ),
                    setting_row(
                        u,
                        "settings-net-catalog",
                        "Refresh Browse",
                        detail(u, "Reloads the GameBanana page you are on, so new uploads appear."),
                        segmented(u, "settings-net-catalog-pick", &catalog, s.prefs.catalog_minutes, |st, m| {
                            prefs::edit(st, |p| p.catalog_minutes = m);
                            st.settings.last_catalog = Some(Instant::now());
                        }),
                    ),
                ],
            ),
            s.prefs.offline,
        ))
        .child(section(
            u,
            "Proxy",
            std::iter::once(stacked_row(
                u,
                "settings-proxy",
                "Proxy server",
                &proxy_line,
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(segmented(
                        u,
                        "settings-proxy-kind",
                        &[
                            (ProxyScheme::Http, "HTTP"),
                            (ProxyScheme::Https, "HTTPS"),
                            (ProxyScheme::Socks5, "SOCKS5"),
                            (ProxyScheme::Socks5h, "SOCKS5h"),
                        ],
                        scheme,
                        set_proxy_scheme,
                    ))
                    .child(
                        field(
                            rune::text_input(&s.settings.proxy_edit, MONO)
                                .key("settings-proxy-input")
                                .label(tr("Proxy address"))
                                .single_line()
                                .grow(1.0)
                                .color(u.ink)
                                .placeholder("http://127.0.0.1:7890", u.faint)
                                .caret(u.accent, u.accent.alpha(0.22))
                                .on_edit(|st: &mut State| &mut st.settings.proxy_edit),
                            u,
                        )
                        .grow(1.0)
                        .min_w(0.0),
                    )
                    .child(disabled(
                        button(u, "settings-proxy-save", "Save", ActionKind::Secondary).on_press(save_proxy),
                        !proxy_valid || proxy_typed == proxy_saved,
                    ))
                    .child(disabled(
                        button(u, "settings-proxy-clear", "Clear", ActionKind::Ghost).on_press(
                            |st: &mut State| {
                                st.settings.proxy_edit = TextEdit::new("");
                                save_proxy(st);
                            },
                        ),
                        s.prefs.proxy.is_none(),
                    )),
            ))
            .chain(super::netx::proxy_rows(s, u))
            .collect(),
        ))
}

/// The proxy kinds Settings offers: DMM's HTTP, HTTPS and SOCKS5, plus SOCKS5 with names
/// resolved on the proxy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
}

impl ProxyScheme {
    fn prefix(self) -> &'static str {
        match self {
            ProxyScheme::Http => "http://",
            ProxyScheme::Https => "https://",
            ProxyScheme::Socks5 => "socks5://",
            ProxyScheme::Socks5h => "socks5h://",
        }
    }
}

pub(crate) fn proxy_scheme(address: &str) -> Option<ProxyScheme> {
    let lower = address.trim().to_ascii_lowercase();
    [ProxyScheme::Socks5h, ProxyScheme::Socks5, ProxyScheme::Https, ProxyScheme::Http]
        .into_iter()
        .find(|scheme| lower.starts_with(scheme.prefix()))
}

/// Swap the typed address's scheme, keeping the host and port; an empty field gets the
/// scheme so the next keys type the host.
pub(crate) fn with_scheme(address: &str, scheme: ProxyScheme) -> String {
    let trimmed = address.trim();
    let rest = match trimmed.find("://") {
        Some(at) => &trimmed[at + 3..],
        None => trimmed,
    };
    format!("{}{rest}", scheme.prefix())
}

fn set_proxy_scheme(st: &mut State, scheme: ProxyScheme) {
    let next = with_scheme(&st.settings.proxy_edit.text, scheme);
    st.settings.proxy_edit = TextEdit::new(next);
}

fn save_proxy(st: &mut State) {
    let typed = st.settings.proxy_edit.text.trim().to_string();
    let value = (!typed.is_empty()).then_some(typed);
    match deadlock_core::net::set_proxy(value.clone()) {
        Ok(()) => {
            prefs::edit(st, |p| p.proxy = value.clone());
            say(
                st,
                if value.is_some() { tr("Proxy saved.") } else { tr("Proxy cleared.") },
                false,
            );
        }
        Err(error) => {
            let error = tr(&error).to_string();
            say(st, error, true)
        }
    }
}

// --- logs and support ------------------------------------------------------------

pub fn logs(s: &State, u: &Ui) -> El {
    let levels: Vec<(Level, &str)> = Level::ALL.iter().map(|l| (*l, l.label())).collect();
    let size = s.settings.log_size.unwrap_or(0);
    let log_dir = log::dir().display().to_string();
    let log_file = log::path().display().to_string();
    let has_log = size > 0;
    let (dumps, newest, folders) = s
        .settings
        .crash_dumps
        .clone()
        .unwrap_or((0, None, Vec::new()));
    let dump_line = if folders.is_empty() {
        tr("Appears once the game folder is found.").to_string()
    } else if dumps == 0 {
        tr("No crash dumps. The game writes one when it crashes.").to_string()
    } else {
        fill(tr("{} crash dumps from the game and Steam."), &[&dumps.to_string()])
    };
    let dump_folder = newest
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.display().to_string()))
        .or_else(|| folders.first().map(|p| p.display().to_string()))
        .unwrap_or_default();
    let newest_dump = newest.map(|p| p.display().to_string()).unwrap_or_default();
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(section(
            u,
            "",
            vec![
                setting_row(
                    u,
                    "settings-log-level",
                    "Log level",
                    detail(u, "Debug writes the most. Keys and tokens are never written."),
                    segmented(u, "settings-log-level-pick", &levels, s.prefs.log_level, |st, level| {
                        prefs::edit(st, |p| p.log_level = level);
                        log::set_level(level);
                        log::info(format!("Log level: {}", level.id()));
                    }),
                ),
                setting_row(
                    u,
                    "settings-log-file",
                    "Log file",
                    detail(
                        u,
                        &if has_log {
                            fill(tr("{} on this computer. Never uploaded."), &[&super::size_label(size)])
                        } else {
                            tr("Empty. Sigil writes here as you use it.").to_string()
                        },
                    ),
                    trailing(vec![
                        disabled(
                            button(u, "settings-log-open", "Open", ActionKind::Secondary).on_press(
                                move |st: &mut State| crate::data::reveal_path(st, log_file.clone()),
                            ),
                            !has_log,
                        ),
                        u.icon_button(&icons::FOLDER, tr("Open the logs folder"))
                            .key("settings-log-folder")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                let _ = std::fs::create_dir_all(&log_dir);
                                crate::data::reveal_path(st, log_dir.clone())
                            }),
                    ]),
                ),
                setting_row(
                    u,
                    "settings-log-copy",
                    "Copy the last 200 lines",
                    detail(u, "For a bug report. Keys and tokens are masked."),
                    trailing(vec![
                        disabled(
                            button(u, "settings-log-copy-run", "Copy", ActionKind::Secondary)
                                .on_press_tx(|st: &mut State, tx: &mut Tx| {
                                    tx.copy(log::tail(200));
                                    say(st, tr("Copied the last 200 lines."), false);
                                }),
                            !has_log,
                        ),
                        disabled(
                            button(u, "settings-log-clear", "Clear", ActionKind::Ghost)
                                .on_press(|st: &mut State| st.settings.confirm = Some(Confirm::ClearLogs)),
                            !has_log,
                        ),
                    ]),
                ),
            ]
            .into_iter()
            // Wave 4: Log source, Copy logs and Ask Sigil for help (DMM).
            .chain(super::logs_w4::rows(s, u))
            .collect(),
        ))
        .child(section(
            u,
            "Game crashes",
            std::iter::once(setting_row(
                u,
                "settings-crash-dumps",
                "Crash dumps",
                detail(u, &dump_line),
                trailing(vec![
                    disabled(
                        button(u, "settings-crash-open", "Show newest", ActionKind::Secondary).on_press(
                            move |st: &mut State| crate::data::reveal_path(st, newest_dump.clone()),
                        ),
                        dumps == 0,
                    ),
                    disabled(
                        u.icon_button(&icons::FOLDER, tr("Open the crash dump folder"))
                            .key("settings-crash-folder")
                            .cursor(rune::Cursor::Pointer)
                            .on_press(move |st: &mut State| {
                                crate::data::reveal_path(st, dump_folder.clone())
                            }),
                        folders.is_empty(),
                    ),
                    u.icon_button(&icons::REFRESH_CW, tr("Look again"))
                        .key("settings-crash-refresh")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            read_crash_dumps(st);
                            super::support::read_crashes(st);
                        }),
                ]),
            ))
            .chain(super::support::crash_rows(s, u))
            .collect(),
        ))
        .child(section(
            u,
            "Report a problem",
            super::support::report_rows(s, u)
                .into_iter()
                .chain(std::iter::once(super::logs_w4::full_log_row(u)))
                .collect(),
        ))
}

/// Count crash dumps in `<game>/game/bin/win64` and every Steam root's `dumps` folder.
pub fn read_crash_dumps(st: &mut State) {
    st.settings.log_size = Some(log::size());
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(game) = &st.game_root {
        folders.push(game.join("game").join("bin").join("win64"));
    }
    for root in deadlock_core::discovery::default_steam_roots() {
        folders.push(root.join("dumps"));
    }
    folders.retain(|f| f.is_dir());
    let mut count = 0;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for folder in &folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mdmp") || e.eq_ignore_ascii_case("dmp"))
            {
                count += 1;
                let modified = entry.metadata().and_then(|m| m.modified()).ok();
                if let Some(at) = modified
                    && newest.as_ref().is_none_or(|(t, _)| at > *t)
                {
                    newest = Some((at, path));
                }
            }
        }
    }
    st.settings.crash_dumps = Some((count, newest.map(|(_, p)| p), folders));
}

/// A plain-text report for a bug: no home folder, no keys.
pub fn diagnostic_report(st: &State) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let mask = |s: String| {
        if home.len() > 1 { s.replace(&home, "~") } else { s }
    };
    let gameinfo = match st.launch.read.as_ref() {
        Some(r) if r.gameinfo_addons => "lists citadel/addons",
        Some(r) if r.gameinfo_present => "needs repair",
        Some(_) => "missing",
        None => "not checked",
    };
    let mut out = vec![
        format!("Sigil {}", env!("CARGO_PKG_VERSION")),
        format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH),
        deadlock_core::diagnostics::machine_line(),
        format!(
            "Game: {}",
            if st.game_detected { mask(st.paths.game.clone()) } else { "not found".into() }
        ),
        format!("gameinfo.gi: {gameinfo}"),
        format!(
            "Mods: {} installed, {} on",
            st.addons.addons.len(),
            st.addons.enabled_count()
        ),
        format!("Offline mode: {}", st.prefs.offline),
        format!("Language: {}", i18n::current().id()),
        format!("Log level: {}", st.prefs.log_level.id()),
        String::new(),
        "Recent log:".into(),
    ];
    out.push(mask(log::tail(200)));
    log::redact(&out.join("\n"))
}

// --- experimental ----------------------------------------------------------------

pub fn experimental(s: &State, u: &Ui) -> El {
    let mut rows = Vec::new();
    for flag in FLAGS.iter() {
        let id = flag.id;
        rows.push(setting_row(
            u,
            &format!("settings-flag-{id}"),
            flag.title,
            detail(u, flag.line),
            u.toggle(s.prefs.flag(id))
                .key(format!("settings-flag-{id}-switch"))
                .label(tr(flag.title))
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| {
                    prefs::edit(st, |p| p.toggle_flag(id));
                    super::apply_flags(st);
                }),
        ));
    }
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(
            text(tr("Features that are still changing. Turning one off hides it; nothing is deleted."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(section(u, "", rows))
}

// --- pages ------------------------------------------------------------------------

/// Settings > General > Pages: each optional page on or off, and "Mods only" over them all
/// (Deadlock Mod Manager's Features tab). A page that is off leaves the sidebar and its route;
/// what it already wrote into the game stays.
pub fn pages(s: &State, u: &Ui) -> El {
    let mods_only = s.prefs.flag(prefs::MODS_ONLY);
    let on = prefs::PAGES.iter().filter(|(_, flag)| s.prefs.flag(flag.id)).count();
    let count_line = if mods_only {
        fill(tr("{} pages paused. Your choices come back when you turn this off."), &[&on.to_string()])
    } else {
        fill(tr("{} of {} pages on."), &[&on.to_string(), &prefs::PAGES.len().to_string()])
    };
    let mut rows = vec![setting_row(
        u,
        "settings-pages-mods-only",
        "Mods only",
        detail(u, &count_line),
        u.toggle(mods_only)
            .key("settings-pages-mods-only-switch")
            .label(tr("Mods only"))
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| {
                prefs::edit(st, |p| p.toggle_flag(prefs::MODS_ONLY));
                super::apply_flags(st);
                crate::screens::crosshair::game::after_page_switch(st);
            }),
    )];
    for (screen, flag) in prefs::PAGES.iter() {
        let id = flag.id;
        let page = *screen;
        let shown = s.prefs.flag(id);
        let row_el = row()
            .key(format!("settings-page-{}", screen.key()))
            .w_full()
            .min_h(ROW_H)
            .py(8.0)
            .gap(12.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
            .child(rune::icon(screen.icon(), 16.0).color(u.muted).a11y_hidden())
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(tr(flag.title).to_string()).font(BODY_MED).color(u.ink))
                    .child(detail(u, flag.line)),
            )
            // "Paused" in a fixed 64 slot before the switch, so the title never moves.
            .child(row().w(64.0).justify_end().child(if mods_only && shown {
                row()
                    .h(24.0)
                    .px(8.0)
                    .center_y()
                    .radius(crate::ui::r(4.0))
                    .fill(u.over.alpha(0.06))
                    .child(text(tr("Paused")).font(crate::ui::LABEL).color(u.muted))
            } else {
                el()
            }))
            .child(disabled(
                u.toggle(shown && !mods_only)
                    .key(format!("settings-page-{}-switch", screen.key()))
                    .label(tr(flag.title))
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |st: &mut State| {
                        if st.prefs.flag(prefs::MODS_ONLY) {
                            return;
                        }
                        prefs::edit(st, |p| p.toggle_flag(id));
                        super::apply_flags(st);
                        if page == Screen::Crosshair {
                            crate::screens::crosshair::game::after_page_switch(st);
                        }
                    }),
                mods_only,
            ));
        rows.push(row_el);
        if page == Screen::Crosshair
            && let Some(note) = crate::screens::crosshair::game::pages_note(s, u)
        {
            rows.push(note);
        }
    }
    col()
        .w_full()
        .gap(LIST_GAP)
        .child(section(u, "Pages", rows))
}

// --- updates (inside About) ------------------------------------------------------

pub fn channel_row(s: &State, u: &Ui) -> El {
    let channels: Vec<(Channel, &str)> = Channel::ALL.iter().map(|c| (*c, c.label())).collect();
    setting_row(
        u,
        "settings-channel",
        "Release channel",
        detail(u, "Preview includes test builds before they are released."),
        segmented(u, "settings-channel-pick", &channels, s.prefs.channel, |st, c| {
            prefs::edit(st, |p| p.channel = c);
            st.settings.update = None;
            super::start_update_check(st);
        }),
    )
}

// --- background work -------------------------------------------------------------

/// Is Deadlock running? The core's process check (it skips a Deadlock stuck while closing), off
/// the UI thread.
pub fn game_running() -> bool {
    deadlock_core::launch::game_running()
}

/// The background half of Settings: presence, timers, start minimized. Runs every poll.
pub fn background(st: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !st.settings.started {
        st.settings.started = true;
        st.settings.session_start = discord::now();
        if st.prefs.start_minimized && super::autostart::started_minimized() && !rune::is_headless() {
            tx.minimize();
        }
    }
    changed |= sharing::poll(st);
    if let Some(result) = st.settings.gb_job.try_take() {
        match result {
            Ok((id, name)) => {
                prefs::edit(st, |p| p.gamebanana = Some((id, name.clone())));
                st.settings.gb_edit = TextEdit::new("");
                say(st, fill(tr("Linked {}."), &[if name.is_empty() { "GameBanana" } else { &name }]), false);
            }
            Err(error) => say(st, error, true),
        }
        changed = true;
    }
    if let Some(Ok(running)) = st.settings.game_job.try_take() {
        changed |= running != st.settings.game_running;
        st.settings.game_running = running;
    }
    // Discord presence.
    if st.prefs.discord && discord::app_id().is_some() && !st.demo {
        let due = st
            .settings
            .last_game_check
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30));
        if due && !st.settings.game_job.is_running() && !rune::is_headless() {
            st.settings.last_game_check = Some(Instant::now());
            let wake = st.wake.clone();
            st.settings.game_job.start(wake, || Ok(game_running()));
        }
        let activity = presence_activity(st);
        let wake = st.wake.clone();
        st.settings.presence.show(Some(activity), wake);
    } else if st.settings.presence.running() {
        st.settings.presence.stop();
        changed = true;
    }
    changed |= st.settings.presence.poll();
    // Timers, skipped while offline.
    if !st.prefs.offline && !st.demo && !rune::is_headless() {
        if st.prefs.check_updates && st.prefs.update_hours > 0 {
            let every = Duration::from_secs(st.prefs.update_hours as u64 * 3600);
            if st.settings.last_update_check.is_some_and(|t| t.elapsed() >= every) {
                st.settings.last_update_check = Some(Instant::now());
                super::start_update_check(st);
            }
        }
        if st.prefs.catalog_minutes > 0 && st.screen == Screen::Browse && !st.browse_job.is_running() {
            let every = Duration::from_secs(st.prefs.catalog_minutes as u64 * 60);
            match st.settings.last_catalog {
                Some(t) if t.elapsed() >= every => {
                    st.settings.last_catalog = Some(Instant::now());
                    let page = st.browse.page.as_ref().map_or(1, |p| p.page);
                    log::debug(format!("Refreshing Browse page {page}"));
                    crate::data::start_browse_fetch(st, page);
                    changed = true;
                }
                None => st.settings.last_catalog = Some(Instant::now()),
                _ => {}
            }
        }
    }
    // Come back for the next timer.
    if st.prefs.discord || st.prefs.share_matches || st.prefs.catalog_minutes > 0 || st.prefs.update_hours > 0 {
        tx.wake_after(Duration::from_secs(15));
    }
    changed
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_proxy_kind_swaps_only_the_scheme() {
        use super::{ProxyScheme, proxy_scheme, with_scheme};
        assert_eq!(with_scheme("http://127.0.0.1:7890", ProxyScheme::Socks5), "socks5://127.0.0.1:7890");
        assert_eq!(with_scheme("", ProxyScheme::Socks5h), "socks5h://");
        assert_eq!(with_scheme("10.0.0.2:1080", ProxyScheme::Https), "https://10.0.0.2:1080");
        assert_eq!(proxy_scheme("SOCKS5H://h:1"), Some(ProxyScheme::Socks5h));
        assert_eq!(proxy_scheme("socks5://h:1"), Some(ProxyScheme::Socks5));
        assert_eq!(proxy_scheme("h:1"), None);
        assert!(deadlock_core::net::validate_proxy(&with_scheme("h:1080", ProxyScheme::Socks5)).is_ok());
    }

    use super::*;

    #[test]
    fn member_ids_come_from_links_or_numbers() {
        assert_eq!(parse_member("1382"), Some(1382));
        assert_eq!(parse_member("https://gamebanana.com/members/1382"), Some(1382));
        assert_eq!(parse_member("gamebanana.com/members/1382/"), Some(1382));
        assert_eq!(parse_member("https://gamebanana.com/members/1382?tab=mods"), Some(1382));
        assert_eq!(parse_member("https://gamebanana.com/mods/1382"), None);
        assert_eq!(parse_member("abc"), None);
        assert_eq!(parse_member("0"), None);
    }

    #[test]
    fn dates_follow_the_chosen_format() {
        assert_eq!(DateFormat::DayFirst.format(2026, 11, 7), "07/11/2026");
        assert_eq!(DateFormat::MonthFirst.format(2026, 11, 7), "11/07/2026");
        assert_eq!(civil(20_774), (2026, 11, 17));
    }

    #[test]
    fn presence_hides_details_when_asked() {
        let mut s = State::default();
        s.prefs.discord_details = false;
        assert_eq!(presence_activity(&s).details, "Using Sigil");
        assert!(presence_activity(&s).state.is_none());
        s.prefs.discord_details = true;
        s.screen = Screen::Browse;
        assert_eq!(presence_activity(&s).details, "Browsing mods");
        s.settings.open = true;
        assert_eq!(presence_activity(&s).details, "Changing settings");
    }
}
