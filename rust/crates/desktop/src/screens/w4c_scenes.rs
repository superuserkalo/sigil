//! Headless scenes for parity wave 4, UI C: the Locker's game art, game sounds, sound swaps and
//! in-game card. Registered in `main.rs` next to the app's own list. Fixtures only: no game file
//! is read.

use std::path::PathBuf;

use crate::app::State;
use crate::screens::locker::HeroSection;
use crate::screens::locker::assets::{CardSlot, Load, SoundGroup, SoundRow, Swap, Tex, TexKind, thumb_key};

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("w4c-game-art", game_art),
    ("w4c-game-art-cards", game_art_cards),
    ("w4c-game-art-lightbox", game_art_lightbox),
    ("w4c-game-art-loading", game_art_loading),
    ("w4c-game-art-error", game_art_error),
    ("w4c-game-sounds", game_sounds),
    ("w4c-sound-swap", sound_swap),
    ("w4c-sound-swap-error", sound_swap_error),
    ("w4c-ingame-card", ingame_card),
    ("w4c-ingame-review", ingame_review),
];

/// The Haze page with the assets state pointed at her.
fn haze(s: &mut State, section: HeroSection) {
    crate::screens::locker::scene_hero(s);
    s.locker.hero_section = section;
    let code = s
        .locker
        .heroes
        .iter()
        .find(|h| h.display_name == "Haze")
        .map(|h| h.codename.clone())
        .unwrap_or_else(|| "haze".to_string());
    s.locker.assets.hero = Some(code);
}

/// A small PNG tile in one hue with a lighter disc, standing in for a decoded game texture.
fn fixture_png(hue: f32, glyph: u8) -> Vec<u8> {
    let size = 96u32;
    let rgb = |h: f32, l: f32| -> [u8; 3] {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = (1.0 - (2.0 * l - 1.0).abs()) * 0.7;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        [((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8]
    };
    let img = image::RgbaImage::from_fn(size, size, |x, y| {
        let (fx, fy) = (x as f32 / size as f32 - 0.5, y as f32 / size as f32 - 0.5);
        let d = (fx * fx + fy * fy).sqrt();
        let ring = match glyph % 3 {
            0 => d < 0.28,
            1 => fx.abs() < 0.08 || fy.abs() < 0.08,
            _ => (fx.abs() + fy.abs()) < 0.3,
        };
        let [r, g, b] = if ring { rgb(hue + 20.0, 0.72) } else { rgb(hue, 0.22 + 0.25 * (0.5 - fy)) };
        image::Rgba([r, g, b, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

fn textures() -> Vec<Tex> {
    let mut out = Vec::new();
    let mut add = |path: &str, label: &str, kind: TexKind, size: (u32, u32)| {
        out.push(Tex { path: path.to_string(), label: label.to_string(), kind, size: Some(size) });
    };
    add("panorama/images/heroes/haze_card_psd.vtex_c", "card", TexKind::Card, (280, 380));
    add("panorama/images/heroes/haze_vertical_psd.vtex_c", "vertical", TexKind::Card, (340, 1024));
    add("panorama/images/heroes/haze_card_critical_psd.vtex_c", "card critical", TexKind::Card, (280, 380));
    add("panorama/images/heroes/haze_card_gloat_psd.vtex_c", "card gloat", TexKind::Card, (280, 380));
    add("panorama/images/heroes/haze_minimap_psd.vtex_c", "minimap", TexKind::Card, (64, 64));
    add("panorama/images/heroes/haze_sm_psd.vtex_c", "small", TexKind::Card, (128, 128));
    for (file, label) in [
        ("sleep_dagger", "sleep dagger"),
        ("smoke_bomb", "smoke bomb"),
        ("fixation", "fixation"),
        ("bullet_dance", "bullet dance"),
    ] {
        add(&format!("panorama/images/hud/abilities/haze/{file}_psd.vtex_c"), label, TexKind::Ability, (128, 128));
    }
    for (file, label) in [
        ("haze_body_color", "body color"),
        ("haze_body_normal", "body normal"),
        ("haze_head_color", "head color"),
        ("haze_hair_color", "hair color"),
        ("haze_gun_color", "gun color"),
        ("haze_coat_color", "coat color"),
        ("haze_coat_ao", "coat ao"),
        ("haze_dagger_color", "dagger color"),
    ] {
        add(&format!("models/heroes_staging/haze/materials/{file}_png.vtex_c"), label, TexKind::Skin, (2048, 2048));
    }
    for (file, label) in [("smoke_puff", "smoke puff"), ("dagger_trail", "dagger trail"), ("dance_spark", "dance spark")] {
        add(&format!("materials/particle/abilities/haze/{file}_psd.vtex_c"), label, TexKind::Effect, (512, 512));
    }
    out
}

fn stage_textures(s: &mut State) {
    let list = textures();
    for (n, tex) in list.iter().enumerate() {
        let hue = match tex.kind {
            TexKind::Card => 28.0,
            TexKind::Ability => 200.0,
            TexKind::Skin => 260.0 + n as f32 * 9.0,
            TexKind::Effect => 150.0,
            TexKind::Other => 0.0,
        };
        s.pending_thumbs.push((thumb_key(&tex.path), fixture_png(hue, n as u8)));
    }
    // One decode failed: it shows a quiet warning box.
    s.locker.assets.thumbs.failed.insert("models/heroes_staging/haze/materials/haze_coat_ao_png.vtex_c".to_string());
    s.pending_thumbs.retain(|(k, _)| !k.ends_with("haze_coat_ao_png.vtex_c"));
    s.locker.assets.textures = Load::Ready(list);
}

/// The Game art grid with every kind.
pub fn game_art(s: &mut State) {
    haze(s, HeroSection::GameArt);
    stage_textures(s);
}

/// Filtered to cards and portraits, with a search.
pub fn game_art_cards(s: &mut State) {
    game_art(s);
    s.locker.assets.tex_kind = Some(TexKind::Card);
    s.locker.assets.tex_query = rune::TextEdit::new("card");
}

pub fn game_art_lightbox(s: &mut State) {
    game_art(s);
    s.locker.assets.lightbox = Some("panorama/images/heroes/haze_card_psd.vtex_c".to_string());
}

pub fn game_art_loading(s: &mut State) {
    haze(s, HeroSection::GameArt);
    s.locker.assets.textures = Load::Loading;
}

pub fn game_art_error(s: &mut State) {
    haze(s, HeroSection::GameArt);
    s.locker.assets.textures = Load::Failed("pak01_dir.vpk could not be opened: the file ends early.".to_string());
}

fn sounds() -> Vec<SoundRow> {
    let mut out = Vec::new();
    let mut add = |event: &str, label: &str, group: SoundGroup, clips: usize, seconds: f32| {
        out.push(SoundRow {
            event: event.to_string(),
            label: label.to_string(),
            group,
            soundevents: Some("soundevents/hero/haze.vsndevts_c".to_string()),
            clips: (0..clips).map(|n| format!("sounds/abilities/haze/{event}_{n:02}.vsnd_c")).collect(),
            seconds: Some(seconds),
        });
    };
    let dagger = || SoundGroup::Ability(1, "Sleep Dagger".to_string());
    let smoke = || SoundGroup::Ability(2, "Smoke Bomb".to_string());
    let dance = || SoundGroup::Ability(4, "Bullet Dance".to_string());
    add("Haze.SleepDagger.Cast", "Dagger throw", dagger(), 3, 0.6);
    add("Haze.SleepDagger.Impact", "Dagger hit", dagger(), 2, 0.4);
    add("Haze.SmokeBomb.Activate", "Smoke bomb", smoke(), 1, 1.8);
    add("Haze.BulletDance.Start", "Bullet dance start", dance(), 1, 2.2);
    add("Haze.BulletDance.Loop", "Bullet dance loop", dance(), 1, 4.0);
    add("Haze.Weapon.Fire", "Pistol shot", SoundGroup::Weapon, 6, 0.3);
    add("Haze.Weapon.Reload", "Reload", SoundGroup::Weapon, 1, 1.4);
    add("Haze.Melee.Swing", "Melee swing", SoundGroup::Melee, 4, 0.5);
    add("Haze.Footstep", "Footstep", SoundGroup::Movement, 8, 0.2);
    for n in 0..340 {
        add(&format!("Haze.Voice.Line{n:03}"), &format!("Voice line {n}"), SoundGroup::Voice, 1, 2.0);
    }
    out
}

/// The sound browser, one clip playing.
pub fn game_sounds(s: &mut State) {
    haze(s, HeroSection::GameSounds);
    s.locker.assets.sounds = Load::Ready(sounds());
    let clip = "sounds/abilities/haze/Haze.SmokeBomb.Activate_00.vsnd_c";
    s.browse_view.audio.playing = Some(crate::screens::locker::assets::play_id(clip));
}

fn swaps(s: &mut State) {
    let list = sounds();
    let pick = |event: &str, file: &str, looped: Option<bool>| {
        let row = list.iter().find(|r| r.event == event).unwrap();
        Swap {
            event: row.event.clone(),
            label: row.label.clone(),
            soundevents: row.soundevents.clone(),
            clips: row.clips.clone(),
            file: (!file.is_empty()).then(|| PathBuf::from(format!("/Users/me/Music/{file}"))),
            looped,
            gain_db: if file.is_empty() { -4.5 } else { 0.0 },
        }
    };
    s.locker.assets.swaps = vec![
        pick("Haze.SleepDagger.Cast", "whoosh.mp3", None),
        pick("Haze.BulletDance.Loop", "dance-loop.mp3", Some(true)),
        pick("Haze.Weapon.Fire", "", None),
    ];
    s.locker.assets.sounds = Load::Ready(list);
}

/// Two swaps staged, ready to build.
pub fn sound_swap(s: &mut State) {
    haze(s, HeroSection::GameSounds);
    swaps(s);
    s.locker.assets.sound_query = rune::TextEdit::new("dagger");
    // The shared-clip check ran: the pistol shot is shared with two heroes.
    let sig = crate::screens::locker::game_sounds::signature(&s.locker.assets.swaps);
    s.locker.assets.shared = Some((sig, vec![("Haze.Weapon.Fire".to_string(), vec!["Wraith".to_string(), "Vindicta".to_string()])]));
}

/// A WAV picked: the swap is refused with the reason.
pub fn sound_swap_error(s: &mut State) {
    sound_swap(s);
    s.locker.assets.swap_error =
        Some(crate::screens::settings::i18n::tr("Swaps take an MP3 (or a 16-bit WAV for sounds the game stores uncompressed). Convert OGG, FLAC and other audio to MP3 first.").to_string());
}

/// The in-game card dialog: the game's art under each variant, one chosen (smaller than the
/// card, so it warns).
pub fn ingame_card(s: &mut State) {
    haze(s, HeroSection::GameArt);
    stage_textures(s);
    let hero = s.locker.heroes.iter().find(|h| h.display_name == "Haze").cloned();
    let (code, display) = hero.as_ref().map(|h| (h.codename.clone(), h.display_name.clone())).unwrap_or(("haze".into(), "Haze".into()));
    crate::screens::locker::card_art::open(s, &code, &display);
    let slots = vec![
        CardSlot { variant: "Card".into(), path: "panorama/images/heroes/haze_card_psd.vtex_c".into(), size: (280, 380) },
        CardSlot { variant: "Vertical".into(), path: "panorama/images/heroes/haze_vertical_psd.vtex_c".into(), size: (340, 1024) },
        CardSlot { variant: "Low HP".into(), path: "panorama/images/heroes/haze_card_critical_psd.vtex_c".into(), size: (280, 380) },
        CardSlot { variant: "Gloat".into(), path: "panorama/images/heroes/haze_card_gloat_psd.vtex_c".into(), size: (280, 380) },
        CardSlot { variant: "Minimap".into(), path: "panorama/images/heroes/haze_minimap_psd.vtex_c".into(), size: (64, 64) },
        CardSlot { variant: "Small".into(), path: "panorama/images/heroes/haze_sm_psd.vtex_c".into(), size: (128, 128) },
    ];
    // The game's art under the tiles: the hero's own card copy stands in.
    if let Some(card) = hero.as_ref().and_then(|h| h.card.clone())
        && let Ok(bytes) = std::fs::read(&card)
    {
        for slot in &slots[..4] {
            s.pending_thumbs.retain(|(k, _)| k != &thumb_key(&slot.path));
            s.pending_thumbs.push((thumb_key(&slot.path), bytes.clone()));
        }
    }
    let picked = hero.as_ref().and_then(|h| h.render.clone().map(|r| (r, h.render_size)));
    if let Some(c) = s.locker.assets.card.as_mut() {
        c.slots = Load::Ready(slots);
        if let Some((render, _)) = picked {
            c.picks.push(crate::screens::locker::card_art::Pick {
                slot: "panorama/images/heroes/haze_card_psd.vtex_c".into(),
                file: PathBuf::from(&render),
                size: Some((200, 300)),
            });
        }
    }
    if let Some(render) = hero.and_then(|h| h.render) {
        crate::data::request_locker_art(s, &render);
    }
}

/// After Build mod: the import review with the new mod, landing switched off.
pub fn ingame_review(s: &mut State) {
    haze(s, HeroSection::GameSounds);
    swaps(s);
    s.locker.assets.swaps.clear();
    use deadlock_core::import::{ForeignMod, ImportAction, ImportPlan, PlannedImport, SourceKind};
    use crate::screens::import::{Dialog, Stage};
    let plan = ImportPlan {
        kind: SourceKind::Local,
        addons_dir: PathBuf::new(),
        items: vec![PlannedImport {
            source: ForeignMod { name: "Haze sounds".to_string(), ..ForeignMod::default() },
            action: ImportAction::Copy { slot: 14 },
        }],
    };
    s.installed.import.dialog = Some(Dialog {
        kind: SourceKind::Local,
        stage: Stage::Review,
        plan: Some(plan),
        selected: vec![true],
        report: None,
        error: None,
    });
}
