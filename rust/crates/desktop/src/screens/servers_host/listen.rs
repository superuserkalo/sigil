//! "Play with friends, no setup" (ADR 0015, second amendment, first choice): a card at the top
//! of Servers > Host and of the Maps page. The player picks a map (stock or an installed custom
//! map), Launch starts it in their own game through the existing Launch map flow
//! (`screens::maps::launch`: the managed autoexec block, the gameinfo launch gate and the launch
//! steps), and the game's in-game host registers with Steam. Friends join with
//! `connect [A:1:...]` through Steam networking and Valve's relays, with no forwarded port.
//!
//! The code comes from `console.log` (`ServerSteamID=`), which the game writes only with
//! `-condebug`. The Launch press adds `-condebug` to Deadlock's launch options in Steam through
//! the same write as Settings > Game (`steam_launch::write_file`, a backup of localconfig.vdf
//! first) and says so. Deadworks plugins do not run in the in-game host; the card says so too.

use deadlock_core::maps::InstalledMap;
use deadlock_core::servers::invite::{Invite, InviteTarget, item_for_slot};
use rune::{El, Tx, col, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::screens::maps::Invite as Watch;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, H2, MONO, SMALL, Ui};

/// The stock maps the card offers, as on the Host server card.
pub use super::bridge::STOCK_MAPS;

/// What the Launch press did about `-condebug`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConDebug {
    /// Steam's launch options already had it.
    Already,
    /// Sigil added it now.
    Added,
    /// It could not be added (Steam is running, no Steam settings file): the reason.
    Failed(String),
}

/// Every map the card offers: the stock ones, then installed custom maps (enabled slots).
pub fn choices(s: &State) -> Vec<String> {
    let mut maps: Vec<String> = STOCK_MAPS.iter().map(|m| m.to_string()).collect();
    for name in custom_maps(s).into_iter().map(|m| m.name) {
        if !maps.contains(&name) {
            maps.push(name);
        }
    }
    maps
}

/// Installed custom maps, from the Maps page scan and the Host check.
fn custom_maps(s: &State) -> Vec<InstalledMap> {
    let mut maps: Vec<InstalledMap> = s.maps.installed.clone();
    if let Some(Ok(setup)) = &s.servers.host.setup {
        for c in &setup.custom_maps {
            if !maps.iter().any(|m| m.name == c.name) {
                maps.push(InstalledMap { slot: 0, name: c.name.clone(), title: None, vpk: c.vpk.clone() });
            }
        }
    }
    maps.retain(|m| !STOCK_MAPS.contains(&m.name.as_str()));
    maps
}

/// The card opened: make sure the installed maps are known.
pub fn on_open(st: &mut State) {
    if !st.maps.scanned {
        crate::screens::maps::start_scan(st);
    }
}

/// GAME WRITE + PROCESS, the player's press. Make sure Steam starts Deadlock with `-condebug`
/// (so the code reaches console.log), then the Launch map flow.
pub fn launch(st: &mut State) {
    let map = st.servers.host.listen_map.clone();
    if deadlock_core::maps::map_command(&map).is_none() {
        return;
    }
    st.servers.host.listen_condebug = Some(ensure_condebug(st));
    st.servers.host.listen_invite = None;
    crate::screens::maps::launch(st, map);
}

/// Add `-condebug` to Deadlock's launch options in Steam, as Settings > Game does (Steam must
/// be closed; localconfig.vdf is backed up first).
fn ensure_condebug(st: &mut State) -> ConDebug {
    use deadlock_core::{game_session, steam_launch};
    if st.demo || rune::is_headless() {
        return ConDebug::Added;
    }
    let Some(path) = steam_launch::find_localconfigs(st.game_root.as_deref()).into_iter().next() else {
        return ConDebug::Failed(tr("Steam's settings file was not found").to_string());
    };
    let current = match steam_launch::read_file(&path) {
        Ok(value) => value.unwrap_or_default(),
        Err(error) => return ConDebug::Failed(error.to_string()),
    };
    if game_session::has_condebug(&current) {
        crate::screens::settings::options::edit(st, |o| o.condebug = true);
        return ConDebug::Already;
    }
    let value = game_session::with_condebug(&current, true);
    match steam_launch::write_file(&path, &value, true) {
        Ok(_) => {
            crate::screens::settings::options::edit(st, |o| o.condebug = true);
            st.settings.launch_read = None;
            ConDebug::Added
        }
        Err(error) => ConDebug::Failed(error.to_string()),
    }
}

/// Build the invite once the code is in: the code, plus the custom map's GameBanana file when
/// Sigil installed it from there. Cheap when nothing changed; called from the polls.
pub fn refresh(st: &mut State) -> bool {
    let Watch::Ready { map, code } = &st.maps.invite else {
        return false;
    };
    if st.servers.host.listen_invite.as_ref().is_some_and(|(c, _)| c == code) {
        return false;
    }
    let item = custom_maps(st).into_iter().find(|m| &m.name == map).and_then(|m| {
        let size = std::fs::metadata(&m.vpk).map(|meta| meta.len()).ok()?;
        let library = &st.installed.library.mods;
        let by_slot = |slot: u16| library.iter().find(|x| x.slot == slot).and_then(|x| x.gamebanana_id);
        item_for_slot(&st.installed.updates.journal, m.slot, size, &by_slot)
    });
    let Ok(invite) = Invite::new(InviteTarget::Steam(code.clone()), false, item) else {
        return false;
    };
    st.servers.host.listen_invite = Some((code.clone(), invite.to_string()));
    true
}

/// A labelled value with a Copy button; the button reads Copied after a press.
pub fn copy_box(s: &State, u: &Ui, key: &str, label: &str, value: String) -> El {
    let copied = s.servers.host.copied.as_deref() == Some(value.as_str());
    col()
        .w_full()
        .gap(4.0)
        .child(text(label.to_string()).font(SMALL).color(u.muted))
        .child(
            row()
                .key(key.to_string())
                .w_full()
                .h(34.0)
                .pl(10.0)
                .pr(2.0)
                .gap(8.0)
                .center_y()
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(text(value.clone()).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate().selectable(true))
                .child(
                    u.icon_button(if copied { &icons::CHECK } else { &icons::COPY }, if copied { tr("Copied") } else { tr("Copy") })
                        .key(format!("{key}-copy"))
                        .cursor(rune::Cursor::Pointer)
                        .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                            tx.copy(value.clone());
                            st.servers.host.copied = Some(value.clone());
                        }),
                ),
        )
}

fn line(_: &Ui, words: String, color: rune::Color) -> El {
    text(words).font(SMALL).color(color).wrap().w_full()
}

fn map_chip(u: &Ui, key: String, name: String, on: bool) -> El {
    let label = name.clone();
    row()
        .key(key)
        .h(28.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(14.0))
        .fill(if on { u.accent.alpha(0.14) } else { u.well })
        .border(1.0, if on { u.accent.alpha(0.5) } else { u.hair })
        .focusable(true)
        .role("radio")
        .label(label.clone())
        .cursor(rune::Cursor::Pointer)
        .child(text(label).font(MONO).color(if on { u.ink } else { u.muted }).a11y_hidden())
        .on_press(move |st: &mut State| {
            st.servers.host.listen_map = name.clone();
            st.servers.host.listen_invite = None;
        })
}

/// The card. `key` keeps the Host page and the Maps page copies apart.
pub fn card(s: &State, u: &Ui, key: &str) -> El {
    let host = &s.servers.host;
    let map = host.listen_map.clone();
    let mut chips = row().w_full().gap(6.0).flex_wrap();
    for name in choices(s) {
        let on = name == map;
        chips = chips.child(map_chip(u, format!("{key}-map-{name}"), name, on));
    }
    // Launch map itself refuses while Deadlock runs and says why.
    let launch = u
        .icon_action(&icons::PLAY, tr("Launch"), ActionKind::Primary)
        .key(format!("{key}-launch"))
        .cursor(rune::Cursor::Pointer)
        .on_press(launch);
    let mut c = col()
        .key(format!("{key}-listen"))
        .w_full()
        .gap(12.0)
        .padding(16.0)
        .radius(crate::ui::r(10.0))
        .fill(u.accent.alpha(0.05))
        .border(1.0, u.accent.alpha(0.25))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(icon(&icons::USER, 18.0).color(u.accent).a11y_hidden())
                .child(text(tr("Play with friends, no setup")).font(H2).color(u.ink)),
        )
        .child(
            text(tr("Start a map in your own game. Friends join you through Steam: no server files and no ports to open."))
                .font(BODY)
                .color(u.prose)
                .wrap()
                .w_full(),
        )
        .child(chips)
        .child(row().w_full().gap(10.0).center_y().child(launch).child(
            text(fill(tr("Starts {} in Deadlock."), &[&map])).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).wrap(),
        ))
        .child(line(
            u,
            tr("Sigil adds -condebug to Deadlock's launch options in Steam for this launch, so it can read the invite code from the game's console log. It stays on; turn it off in Settings > Game.").to_string(),
            u.faint,
        ))
        .child(line(
            u,
            if key == "host" {
                tr("Plugins do not run in the in-game host. For plugins, start the Deadworks server below.")
            } else {
                tr("Plugins do not run in the in-game host. For plugins, host a Deadworks server in Servers > Host.")
            }
            .to_string(),
            u.faint,
        ));
    match &host.listen_condebug {
        Some(ConDebug::Added) => {
            c = c.child(line(u, tr("Sigil added -condebug to Deadlock's launch options.").to_string(), u.teal));
        }
        Some(ConDebug::Failed(why)) => {
            c = c.child(line(
                u,
                fill(
                    tr("Sigil could not add -condebug ({}). Without it no code shows here: open the console (F7) in the game, type status and share the steamid line, like [A:1:123:456]."),
                    &[why],
                ),
                u.amber,
            ));
        }
        _ => {}
    }
    match &s.maps.invite {
        Watch::Ready { map: running, code } if *running == map => {
            c = c.child(line(u, tr("Your game is hosting. Send friends one of these:").to_string(), u.prose));
            if let Some((_, invite)) = &host.listen_invite {
                c = c.child(copy_box(s, u, &format!("{key}-invite"), tr("Sigil invite (friends paste it in Servers > Join with invite)"), invite.clone()));
            }
            c = c.child(copy_box(
                s,
                u,
                &format!("{key}-connect"),
                tr("Console line (F7 in Deadlock)"),
                deadlock_core::maps::connect_command(code),
            ));
            c = c.child(line(u, tr("The code changes every time you start a map. Friends need the same map installed; Sigil's invite installs it for them.").to_string(), u.faint));
        }
        Watch::Watching(running) if *running == map => {
            c = c.child(line(u, tr("Waiting for your game to host the map. The invite shows here when it is up.").to_string(), u.muted));
        }
        Watch::Missed(running) if *running == map => {
            c = c.child(line(u, tr("No code showed up in console.log. Open the console (F7), type status and share the steamid line.").to_string(), u.amber));
        }
        _ => {}
    }
    c
}
