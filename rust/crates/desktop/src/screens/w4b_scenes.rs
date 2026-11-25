//! Headless scenes for parity wave 4, UI B: Settings search focus, the full size image, and
//! the Browse, Settings, shell, Stats and Crosshair additions.

use crate::app::State;

/// Settings opened on Game with an empty search: no caret until the field is focused (issue #2).
pub fn settings_open(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Game);
}

/// The details with the full size image open: X and counter sit on the picture (issue #3).
pub fn lightbox(s: &mut State) {
    crate::screens::browse::scene_details(s);
    if let Some(details) = s.browse_view.details.as_mut() {
        details.lightbox = Some(0);
    }
}

/// The same from the docked details panel.
pub fn lightbox_docked(s: &mut State) {
    crate::screens::browse::scene_docked(s);
    if let Some(details) = s.browse_view.details.as_mut() {
        details.lightbox = Some(1);
    }
}

/// Settings > Network with a SOCKS5 proxy typed: the kind picker and where names resolve.
pub fn proxy_socks(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Network);
    s.settings.proxy_edit = rune::TextEdit::new("socks5h://127.0.0.1:1080");
}

/// The sidebar collapsed to icons over Installed, with an update waiting in the banner.
pub fn sidebar_collapsed(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.w4.collapsed = true;
}

/// The expanded sidebar with "New" on the newest pages and the update banner on top.
pub fn update_banner(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.w4.seen_pages.clear();
    s.settings.update = Some(Ok(crate::screens::settings::UpdateCheck {
        latest: Some(crate::screens::settings::Release {
            version: "0.2.0".to_string(),
            url: "https://github.com/".to_string(),
            notes: String::new(),
            install: Err(String::new()),
        }),
        newer: true,
    }));
}

/// Settings > Logs and support: Log source, Copy logs, Ask Sigil and the full log switch.
pub fn logs(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Logs);
    s.settings.w4.scene_only = Some("logs");
}

/// The confirm before the logs go into the agent's message box.
pub fn logs_ask(s: &mut State) {
    logs(s);
    s.settings.confirm = Some(crate::screens::settings::Confirm::AskAgent);
}

/// Settings > Game: the common launch options under the field, two of them on, and the
/// vanilla switch.
pub fn launch_options(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Game);
    s.settings.launch_options = rune::TextEdit::new("-novid -high +fps_max 240");
    s.settings.w4.scene_only = Some("launch");
}

/// Settings > About and updates on Preview: the channel notes and the Releases link.
pub fn channel_notes(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::About);
    s.prefs.channel = crate::prefs::Channel::Preview;
}

/// Browse's landing with this week's featured mods per category (from the saved catalog) and
/// the Random button on the hero rail.
pub fn browse_featured(s: &mut State) {
    crate::screens::browse::scene_dashboard(s);
    let mut mirror = deadlock_core::catalog_mirror::Mirror::default();
    let categories = ["Skins", "HUD", "Other/Misc", "Skins", "HUD", "Other/Misc"];
    let top = s.browse_view.top.clone().unwrap_or_default();
    for (index, sub) in top.iter().filter(|t| !t.nsfw).take(6).enumerate() {
        mirror.upsert(deadlock_core::catalog_mirror::MirrorEntry {
            id: sub.id,
            model: "Mod".into(),
            name: sub.name.clone(),
            author: sub.author.clone(),
            category: categories[index % categories.len()].into(),
            profile_url: format!("https://gamebanana.com/mods/{}", sub.id),
            thumbnail_url: sub.image_url.clone(),
            likes: sub.likes,
            views: 1_000,
            ..deadlock_core::catalog_mirror::MirrorEntry::default()
        });
    }
    s.browse_view.mirror = Some(mirror);
    crate::screens::browse::w4::refresh_featured(s);
    s.browse_view.mirror = None;
}

/// The details with the creator's links, the installed copy with Delete, and the image menu
/// open on the first preview.
pub fn browse_details_links(s: &mut State) {
    crate::screens::browse::scene_details_extras(s);
    use deadlock_core::library_facts::{AuthorContact, AuthorProfile, ContactKind};
    let member = s
        .browse_view
        .details
        .as_ref()
        .and_then(|d| d.profile.as_ref())
        .and_then(|p| p.submitter.as_ref())
        .map(|m| (m.id, m.name.clone()));
    if let Some((id, name)) = member {
        let contact = |kind, value: &str, url: Option<&str>| AuthorContact {
            kind,
            value: value.to_string(),
            url: url.map(str::to_string),
        };
        s.browse_view.w4.authors.insert(
            id,
            Ok(AuthorProfile {
                member_id: id,
                name,
                profile_url: String::new(),
                avatar_url: None,
                contacts: vec![
                    contact(ContactKind::Discord, "creator_name", None),
                    contact(ContactKind::Youtube, "@creator", Some("https://www.youtube.com/@creator")),
                    contact(ContactKind::X, "@creator", Some("https://x.com/creator")),
                ],
            }),
        );
    }
    s.addons.addons.push(crate::model::AddonRow {
        slot: 7,
        file: "pak07_dir.vpk".into(),
        size_bytes: 1,
        chunk_files: Vec::new(),
        entry_count: None,
        enabled: true,
    });
    s.addons.scanned = true;
    s.installed.library_sig = Some(vec![(7, 1)]);
    s.addons_dir = Some("/game/citadel/addons".into());
    s.browse_view.docked = true;
    s.browse_view.revealed.clear();
}

/// The image menu over the first preview.
pub fn browse_image_menu(s: &mut State) {
    crate::screens::browse::scene_details(s);
    s.browse_view.w4.image_menu = Some(("gallery-0".to_string(), "https://images.gamebanana.com/img/ss/mods/example.jpg".to_string()));
}

/// The docked details panel dragged wider.
pub fn browse_dock_wide(s: &mut State) {
    crate::screens::browse::scene_docked(s);
    s.browse_view.w4.dock_w = 620.0;
}

/// GameBanana answered 429: the grid says so instead of blaming the connection.
pub fn browse_rate_limited(s: &mut State) {
    s.screen = crate::model::Screen::Browse;
    s.browse.page = None;
    s.browse.loading = false;
    s.browse.error = Some("GameBanana returned HTTP 429 for https://gamebanana.com/apiv11/Mod/Index.".into());
}

/// Settings > Network: DMM's checklist after a run, with a slow service, a timeout, a stale
/// catalog and limited auto-tuning, their steps open.
pub fn troubleshoot(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Network);
    let checks = crate::screens::settings::diag_w4::demo_checks();
    s.settings.w4.open = checks
        .iter()
        .filter(|c| c.status != deadlock_core::diagnostics::Status::Ok && !c.fixes.is_empty())
        .map(|c| c.id.clone())
        .collect();
    s.settings.w4.checks = Some(checks);
    crate::screens::settings::diag_w4::read_catalog(s);
    s.settings.w4.scene_only = Some("troubleshoot");
}

/// Settings > Storage: the saved catalog card with the wipe question open.
pub fn catalog(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Storage);
    crate::screens::settings::diag_w4::read_catalog(s);
    s.settings.w4.wipe_ask = true;
    s.settings.w4.scene_only = Some("catalog");
}

/// Stats with the ranked filter on top scoping the overview, a party picked by hand and the
/// item meta.
pub fn stats(s: &mut State) {
    crate::screens::stats::w4::scene(s);
}

/// Crosshair > Game with "Write to the game's hero settings" on: the plan is read, the write
/// waits for the owner's apply step.
pub fn crosshair_hero(s: &mut State) {
    s.screen = crate::model::Screen::Crosshair;
    s.crosshair.group = crate::screens::crosshair::Group::Game;
    let mut o = crate::screens::settings::options::get_owned();
    o.crosshair_hero_settings = true;
    crate::screens::settings::options::set(o);
    s.crosshair.w4_plan = Some(Ok((2, 2)));
}

/// The first-run setup on its first step, finding the game. The proxy and offline mode it once
/// asked live in Settings > Network (`settings-network` scene).
pub fn onboarding_game(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.settings.extra.onboarding = Some(crate::screens::settings::onboarding::Step::Game);
}

/// A feature tip over the sidebar's collapse button.
pub fn feature_tip(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.tips.active = crate::shell_tips::HINTS.iter().position(|h| h.id == "sidebar-collapse");
}

/// The experimental tour on its Agent stop, over Settings > Experimental.
pub fn tour(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Experimental);
    s.shell.tips.tour = Some(1);
}

/// The news computer after a few commands, and a seasonal mark in the sidebar.
pub fn easter_eggs(s: &mut State) {
    crate::app::scene_demo_installed(s);
    let mut lines = vec!["NEWS COMPUTER v0.1. Type help.".to_string(), "> door stuck".to_string()];
    lines.extend(crate::shell_tips::answer("door stuck"));
    lines.push("> Haze".to_string());
    lines.extend(crate::shell_tips::answer("Haze"));
    s.shell.tips.toy = Some(crate::shell_tips::Toy { lines, input: rune::TextEdit::new("nerf bebop") });
    s.shell.tips.scene_season = Some(crate::shell_tips::Season::Christmas);
}

/// Settings > Appearance: Seasons and extras with both switches on.
pub fn seasons_settings(s: &mut State) {
    crate::screens::settings::scene_tab(s, crate::screens::settings::Tab::Appearance);
    let mut o = crate::screens::settings::options::get_owned();
    o.seasonal = true;
    o.easter_eggs = true;
    crate::screens::settings::options::set(o);
    s.shell.w4.eggs = true;
    s.settings.w4.scene_only = Some("fun");
}

/// The blueprint grid behind Installed.
pub fn themed_grid(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.w4.grid = true;
}

/// The paste dialog after Preview: picked mods, one installed, one mature skipped.
pub fn browse_paste(s: &mut State) {
    crate::screens::browse::scene_dashboard(s);
    s.browse_view.paste.open = true;
    s.browse_view.paste.text = rune::TextEdit::new(
        "https://gamebanana.com/mods/741478\nhttps://gamebanana.com/mods/741608, 741500 741478 https://example.org/9",
    );
    s.browse_view.paste.skip_nsfw = true;
    let rows = vec![
        crate::screens::browse::paste::Row { id: 741478, name: "Long Hair Haze".into(), nsfw: false, installed: false },
        crate::screens::browse::paste::Row { id: 741608, name: "Random Respawn Dances".into(), nsfw: false, installed: true },
        crate::screens::browse::paste::Row { id: 741500, name: "Haze jacket".into(), nsfw: true, installed: false },
    ];
    s.browse_view.paste.picked = [741478].into_iter().collect();
    s.browse_view.paste.rows = Some(rows);
    s.browse_view.paste.skipped = crate::screens::browse::paste::Skipped { duplicates: 1, invalid: 1, over: 0 };
}

/// The downloads panel: places in line, and the running row that failed over from one file
/// server to another.
pub fn downloads_failover(s: &mut State) {
    crate::screens::launch::scene_downloads_panel(s);
    if let Some(queue) = s.launch.downloads.demo.as_mut()
        && let Some(item) = queue.items.iter_mut().find(|i| i.id == "d1")
    {
        item.server = Some("filecache9.gamebanana.com".into());
    }
    s.launch.downloads.servers.insert(
        "d1".to_string(),
        ("filecache9".to_string(), Some(("filecache3".to_string(), std::time::Instant::now()))),
    );
}

/// The full-size picture zoomed to 200 percent and panned.
pub fn lightbox_zoomed(s: &mut State) {
    lightbox(s);
    s.browse_view.w4.zoom = 2.0;
    s.browse_view.w4.pan = (120.0, -60.0);
}

/// Paste after a queue: one row's files open with a file picked, and the saved profile line.
pub fn browse_paste_files(s: &mut State) {
    browse_paste(s);
    use deadlock_core::mod_lookup::ModFile;
    let file = |id: u64, name: &str, size: u64| ModFile {
        file_id: id,
        file_name: name.to_string(),
        download_url: format!("https://gamebanana.com/dl/{id}"),
        size: Some(size),
    };
    s.browse_view.paste.files.insert(
        741478,
        Ok(vec![file(1_550_001, "long_hair_haze.zip", 18_400_000), file(1_550_002, "long_hair_haze_red.zip", 18_100_000)]),
    );
    s.browse_view.paste.open_row = Some(741478);
    s.browse_view.paste.variant.insert(741478, 1_550_002);
    s.browse_view.paste.saved = Some(Ok("Pasted mods 2026-11-18 15:04".to_string()));
}

/// The Crest themed preset behind Installed.
pub fn themed_crest(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.w4.crest = true;
}

/// Christmas snow over Installed, a few seconds in.
pub fn seasonal_effects(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.tips.scene_season = Some(crate::shell_tips::Season::Christmas);
    s.shell.w4.effects = true;
    s.shell.tips.effect_t = 7.5;
}

/// The Terminal themed preset: the grid and monospace type.
pub fn themed_terminal(s: &mut State) {
    crate::app::scene_demo_installed(s);
    s.shell.w4.grid = true;
    s.shell.w4.mono = true;
}
