//! Tests for game asset previews and swaps. File IO uses a per-test folder under the system
//! temp directory. Real-format fixtures come from vpkmerge's own test set in
//! `vendor/vpkmerge/morphic/fixtures` (`pnpm vendor`); tests that need one skip when it is
//! absent. `DEADLOCK_PAK01=<path to pak01_dir.vpk> cargo test -p deadlock-core -- --ignored`
//! runs the end-to-end check against a real install.

use super::*;
use crate::vpk_write::{PackOptions, pack_vpk_single};

fn root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dah-assets-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Pack `files` into one self-contained `<root>/pak/pak01_dir.vpk`, the game's layout.
fn pak(root: &Path, files: &[(&str, &[u8])]) -> PathBuf {
    let staging = root.join("staging-pak");
    for (path, data) in files {
        let target = staging.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, data).unwrap();
    }
    pack_vpk_single(&PackOptions::new(staging, root.join("pak"), "pak01"))
        .unwrap()
        .dir_path
}

fn fixture(relative: &str) -> Option<Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../vendor/vpkmerge/morphic/fixtures")
        .join(relative);
    match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipping: {} is missing (run pnpm vendor)", path.display());
            None
        }
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = morphic::Image {
        width,
        height,
        data: ImageData::Rgba8(vec![200; (width * height * 4) as usize]),
    };
    morphic::encode_image(&image, morphic::TextureFormat::PngRgba8888).unwrap()
}

/// Three CBR MPEG1 Layer III frames (128 kbps, 44.1 kHz, stereo), zero padded.
fn mp3() -> Vec<u8> {
    let mut frame = vec![0xFF, 0xFB, 0x90, 0x00];
    frame.resize(417, 0);
    frame.repeat(3)
}

#[test]
fn classifies_cards_portraits_and_heroes() {
    let card = classify_game_texture("panorama/images/heroes/hornet_card_critical_psd.vtex_c");
    assert_eq!(card.kind, TextureKind::HeroCard);
    assert_eq!(card.variant.as_deref(), Some("card_critical"));
    assert_eq!(card.hero.as_deref(), Some("Vindicta"));

    let minimap = classify_game_texture("panorama/images/heroes/vampirebat_mm_psd.vtex_c");
    assert_eq!(minimap.kind, TextureKind::HeroPortrait);
    assert_eq!(minimap.variant.as_deref(), Some("minimap"));
    assert_eq!(minimap.hero.as_deref(), Some("Mina"));

    let icon = classify_game_texture("panorama/images/hud/abilities/archer/charged_shot_psd.vtex_c");
    assert_eq!(icon.kind, TextureKind::AbilityIcon);
    assert_eq!(icon.hero.as_deref(), Some("Grey Talon"));

    let skin = classify_game_texture("models/heroes_wip/haze/materials/haze_color_png_1a2b3c4d.vtex_c");
    assert_eq!(skin.kind, TextureKind::SkinTexture);
    assert_eq!(skin.hero.as_deref(), Some("Haze"));
    assert_eq!(skin.variant, None);
}

#[test]
fn sound_codenames_map_to_the_roster() {
    assert_eq!(roster_for_codename("abrams").map(|h| h.name), Some("Abrams"));
    assert_eq!(roster_for_codename("mokrill").map(|h| h.name), Some("Mo & Krill"));
    assert_eq!(roster_for_codename("gigawatt").map(|h| h.name), Some("Seven"));
    assert_eq!(roster_for_codename("announcer"), None);
}

#[test]
fn card_variants_sort_like_grimoire_and_dmm() {
    let mut variants = vec!["minimap", "card_gloat", "weird", "card", "vertical"];
    variants.sort_by_key(|variant| card_variant_rank(variant));
    assert_eq!(variants, ["card", "vertical", "card_gloat", "minimap", "weird"]);
    assert_eq!(card_variant("panorama/images/heroes/haze_sm_psd.vtex_c", "haze"), "small");
}

#[test]
fn texture_catalog_filters_by_hero_kind_and_search() {
    let root = root("catalog");
    let pak = pak(
        &root,
        &[
            ("panorama/images/heroes/bull_card_psd.vtex_c", b"x"),
            ("panorama/images/heroes/atlas_mm_psd.vtex_c", b"x"),
            ("panorama/images/hud/abilities/atlas/seismic_impact_psd.vtex_c", b"x"),
            ("panorama/images/hud/abilities/haze/sleep_dagger_psd.vtex_c", b"x"),
            ("models/heroes/haze/readme.txt", b"x"),
        ],
    );
    let assets = GameAssets::open_vpk(&pak).unwrap();

    let all = assets.texture_catalog(&TextureQuery::default());
    assert_eq!(all.len(), 4, "non-texture files are not listed");

    let abrams = assets.texture_catalog(&TextureQuery {
        hero: Some("Abrams".to_string()),
        ..TextureQuery::default()
    });
    assert_eq!(abrams.len(), 3, "legacy bull art and atlas art are both Abrams");
    assert_eq!(abrams[0].kind, TextureKind::HeroCard);

    let icons = assets.texture_catalog(&TextureQuery {
        kinds: vec![TextureKind::AbilityIcon],
        search: Some("DAGGER".to_string()),
        ..TextureQuery::default()
    });
    assert_eq!(icons.len(), 1);
    assert_eq!(icons[0].hero.as_deref(), Some("Haze"));

    let limited = assets.texture_catalog(&TextureQuery {
        limit: Some(1),
        ..TextureQuery::default()
    });
    assert_eq!(limited.len(), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn downsample_weights_color_by_alpha() {
    // A red opaque pixel beside a transparent black one averages to red at half alpha.
    let src = [255, 0, 0, 255, 0, 0, 0, 0];
    assert_eq!(downsample(&src, 2, 1, 1, 1), vec![255, 0, 0, 127]);
    assert_eq!(fit_within(1024, 512, 256), (256, 128));
    assert_eq!(fit_within(100, 50, 256), (100, 50));
    assert_eq!(choose_mip(1024, 1024, 11, 256), 2);
    assert_eq!(choose_mip(1024, 1024, 1, 256), 0);
}

#[test]
fn cached_thumbnail_round_trips_and_rejects_garbage() {
    let thumbnail = AssetThumbnail {
        width: 2,
        height: 1,
        source_width: 400,
        source_height: 200,
        rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
    };
    let bytes = encode_cached_thumbnail(&thumbnail);
    assert_eq!(decode_cached_thumbnail(&bytes), Some(thumbnail));
    assert_eq!(decode_cached_thumbnail(b"SGTH"), None);
    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 2);
    assert_eq!(decode_cached_thumbnail(&truncated), None);
}

#[test]
fn thumbnails_decode_real_textures_bounded_and_cache_on_disk() {
    let Some(bc7) = fixture("bc7/generic_sleep_icon.vtex_c") else {
        return;
    };
    let Some(rgba8) = fixture("rgba8/minimap_circle.vtex_c") else {
        return;
    };
    let root = root("thumbs");
    let pak = pak(
        &root,
        &[
            ("panorama/images/hud/abilities/haze/sleep_psd.vtex_c", &bc7),
            ("panorama/images/heroes/haze_mm_psd.vtex_c", &rgba8),
        ],
    );
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let cache = root.join("cache");

    for path in [
        "panorama/images/hud/abilities/haze/sleep_psd.vtex_c",
        "panorama/images/heroes/haze_mm_psd.vtex_c",
    ] {
        let first = assets.texture_thumbnail(path, 10_000, &cache).unwrap();
        assert!(first.width <= THUMBNAIL_MAX_EDGE && first.height <= THUMBNAIL_MAX_EDGE);
        assert_eq!(first.rgba.len(), (first.width * first.height * 4) as usize);
        assert!(first.source_width >= first.width);
        let file = assets.thumbnail_cache_path(&cache, path, THUMBNAIL_MAX_EDGE);
        assert!(file.is_file(), "the preview is cached on disk");
        let again = assets.texture_thumbnail(path, 256, &cache).unwrap();
        assert_eq!(again, first);
    }

    let small = assets
        .texture_thumbnail("panorama/images/heroes/haze_mm_psd.vtex_c", 8, &cache)
        .unwrap();
    assert!(small.width.max(small.height) <= 8);

    assert!(assets.texture_thumbnail("missing/file.vtex_c", 64, &cache).is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn prune_cache_drops_old_builds_and_caps_size() {
    let root = root("prune");
    let pak = pak(&root, &[("panorama/images/heroes/haze_mm_psd.vtex_c", b"x")]);
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let cache = root.join("cache");
    let stale = cache.join("thumbs").join("old-build").join("64");
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("a.thumb"), vec![0; 100]).unwrap();
    let current = cache.join("thumbs").join(assets.fingerprint()).join("64");
    fs::create_dir_all(&current).unwrap();
    fs::write(current.join("b.thumb"), vec![0; 100]).unwrap();
    fs::write(current.join("c.thumb"), vec![0; 100]).unwrap();

    let freed = assets.prune_cache(&cache, 150).unwrap();
    assert_eq!(freed, 200);
    assert!(!cache.join("thumbs").join("old-build").exists());
    assert_eq!(fs::read_dir(&current).unwrap().count(), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn hero_card_slots_list_variants_in_order() {
    let Some(rgba8) = fixture("rgba8/minimap_circle.vtex_c") else {
        return;
    };
    let root = root("slots");
    let pak = pak(
        &root,
        &[
            ("panorama/images/heroes/hornet_mm_psd.vtex_c", &rgba8),
            ("panorama/images/heroes/hornet_card_psd.vtex_c", &rgba8),
            ("panorama/images/heroes/haze_card_psd.vtex_c", &rgba8),
        ],
    );
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let slots = assets.hero_card_slots("Vindicta").unwrap();
    let variants: Vec<&str> = slots.iter().map(|slot| slot.variant.as_str()).collect();
    assert_eq!(variants, ["card", "minimap"]);
    assert!(slots[0].width > 0 && slots[0].height > 0);
    assert!(assets.hero_card_slots("").is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn hero_card_mod_mints_the_template_and_packs_a_vpk() {
    let Some(rgba8) = fixture("rgba8/minimap_circle.vtex_c") else {
        return;
    };
    let root = root("card");
    let entry = "panorama/images/heroes/hornet_card_psd.vtex_c";
    let pak = pak(&root, &[(entry, &rgba8)]);
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let out = root.join("built");

    let built = assets
        .build_hero_card_mod(
            &[TextureUpload {
                entry: entry.to_string(),
                png: png(7, 5),
            }],
            &out,
            "sigil_card_vindicta",
        )
        .unwrap();
    assert_eq!(built.entries, [entry]);
    assert_eq!(built.vpk_path, out.join("sigil_card_vindicta_dir.vpk"));
    assert_eq!(built.looped, None);
    let leftover: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|child| child.file_name().to_string_lossy().starts_with(".staging"))
        .collect();
    assert!(leftover.is_empty(), "the staging folder is removed");

    let directory = read_vpk_directory(&built.vpk_path).unwrap();
    let minted = read_vpk_entry_data(&directory, &directory.by_path[entry]).unwrap();
    let template = morphic::inspect(&rgba8).unwrap();
    let info = morphic::inspect(&minted).unwrap();
    assert_eq!((info.width, info.height), (template.width, template.height));
    assert_ne!(minted, rgba8);

    let not_png = assets.build_hero_card_mod(
        &[TextureUpload {
            entry: entry.to_string(),
            png: b"GIF89a".to_vec(),
        }],
        &out,
        "bad",
    );
    assert!(not_png.unwrap_err().to_string().contains("not a PNG"));
    let unknown = assets.build_hero_card_mod(
        &[TextureUpload {
            entry: "panorama/images/heroes/nobody_card_psd.vtex_c".to_string(),
            png: png(2, 2),
        }],
        &out,
        "bad",
    );
    assert!(unknown.unwrap_err().to_string().contains("no template"));
    assert!(assets.build_hero_card_mod(&[], &out, "bad").is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sound_index_groups_by_hero_and_caches_per_hero() {
    let Some(events) = fixture("kv3/gigawatt.vsndevts_c") else {
        return;
    };
    let root = root("sounds");
    let pak = pak(&root, &[("soundevents/hero/gigawatt.vsndevts_c", &events)]);
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let cache = root.join("cache");

    let heroes = assets.sound_heroes(&cache).unwrap();
    let seven = heroes.iter().find(|hero| hero.name == "Seven").expect("Seven indexed");
    assert!(seven.sounds > 0);
    assert!(cache
        .join("sounds")
        .join(assets.fingerprint())
        .join("seven.json")
        .is_file());

    for wanted in ["Seven", "gigawatt", "seven"] {
        let sounds = assets
            .sound_catalog(&cache, &SoundQuery {
                hero: wanted.to_string(),
                ..SoundQuery::default()
            })
            .unwrap();
        assert_eq!(sounds.len(), seven.sounds, "{wanted} resolves to the same hero");
        assert!(sounds.iter().all(|sound| {
            sound.soundevents.as_deref() == Some("soundevents/hero/gigawatt.vsndevts_c")
                && !sound.clips.is_empty()
        }));
    }

    let weapon = assets
        .sound_catalog(&cache, &SoundQuery {
            hero: "Seven".to_string(),
            kinds: vec![SoundKind::Weapon],
            limit: Some(2),
            ..SoundQuery::default()
        })
        .unwrap();
    assert!(weapon.len() <= 2);
    assert!(weapon.iter().all(|sound| sound.kind == SoundKind::Weapon));

    let nobody = assets
        .sound_catalog(&cache, &SoundQuery {
            hero: "Haze".to_string(),
            ..SoundQuery::default()
        })
        .unwrap();
    assert!(nobody.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sound_swap_explains_bad_input() {
    let Some(events) = fixture("kv3/gigawatt.vsndevts_c") else {
        return;
    };
    let root = root("swap");
    let pak = pak(&root, &[("soundevents/hero/gigawatt.vsndevts_c", &events)]);
    let assets = GameAssets::open_vpk(&pak).unwrap();
    let cache = root.join("cache");
    let out = root.join("built");
    let sound = assets
        .sound_catalog(&cache, &SoundQuery {
            hero: "Seven".to_string(),
            ..SoundQuery::default()
        })
        .unwrap()
        .remove(0);

    let no_target = assets.build_sound_swap(&SoundSwapRequest::default(), &out, "swap");
    assert!(no_target.unwrap_err().to_string().contains("Pick a sound"));

    let ogg = SoundSwapRequest {
        target: Some(sound.swap_target()),
        audio: Some(b"OggS\0\0\0\0not mp3 at all".to_vec()),
        audio_name: "song.ogg".to_string(),
        ..SoundSwapRequest::default()
    };
    let error = assets.build_sound_swap(&ogg, &out, "swap").unwrap_err().to_string();
    assert!(error.contains("song.ogg is not an MP3"), "{error}");

    let nothing = SoundSwapRequest {
        target: Some(sound.swap_target()),
        ..SoundSwapRequest::default()
    };
    assert!(assets.build_sound_swap(&nothing, &out, "swap").is_err());

    // The fixture pak has the soundevents file but none of its clips.
    let missing = SoundSwapRequest {
        target: Some(sound.swap_target()),
        audio: Some(mp3()),
        audio_name: "song.mp3".to_string(),
        ..SoundSwapRequest::default()
    };
    let error = assets.build_sound_swap(&missing, &out, "swap").unwrap_err().to_string();
    assert!(error.contains("were found"), "{error}");

    let wrong_event = SoundSwapRequest {
        target: Some(SoundSwapTarget::Event {
            soundevents: "soundevents/hero/gigawatt.vsndevts_c".to_string(),
            event: "No.Such.Event".to_string(),
        }),
        audio: Some(mp3()),
        ..SoundSwapRequest::default()
    };
    let error = assets.build_sound_swap(&wrong_event, &out, "swap").unwrap_err().to_string();
    assert!(error.contains("No.Such.Event is not in"), "{error}");

    let wav_with_gain = SoundSwapRequest {
        target: Some(sound.swap_target()),
        audio: Some(b"RIFF\0\0\0\0WAVEfmt ".to_vec()),
        gain_db: Some(3.0),
        ..SoundSwapRequest::default()
    };
    let error = assets.build_sound_swap(&wav_with_gain, &out, "swap").unwrap_err().to_string();
    assert!(error.contains("Convert"), "{error}");

    assert!(assets.clip_audio("sounds/missing.vsnd").is_err());
    assert_eq!(compiled_clip_entry("sounds/a.vsnd"), "sounds/a.vsnd_c");
    assert_eq!(compiled_clip_entry("sounds/a.vsnd_c"), "sounds/a.vsnd_c");
    assert_eq!(compiled_clip_entry("sounds/a"), "sounds/a.vsnd_c");
    let _ = fs::remove_dir_all(&root);
}

/// End to end against a real install: catalog, preview, clip audio, a volume swap and a card.
#[test]
#[ignore = "needs DEADLOCK_PAK01 pointing at a real pak01_dir.vpk"]
fn real_pak01_end_to_end() {
    let Ok(pak) = std::env::var("DEADLOCK_PAK01") else {
        eprintln!("skipping: DEADLOCK_PAK01 is not set");
        return;
    };
    let root = root("real");
    let cache = root.join("cache");
    let out = root.join("built");
    let assets = GameAssets::open_vpk(Path::new(&pak)).unwrap();

    let cards = assets.texture_catalog(&TextureQuery {
        hero: Some("Haze".to_string()),
        kinds: vec![TextureKind::HeroCard],
        ..TextureQuery::default()
    });
    assert!(!cards.is_empty());
    let thumbnail = assets.texture_thumbnail(&cards[0].path, 256, &cache).unwrap();
    assert!(thumbnail.width.max(thumbnail.height) <= 256);

    let sounds = assets
        .sound_catalog(&cache, &SoundQuery {
            hero: "Haze".to_string(),
            kinds: vec![SoundKind::Ability],
            ..SoundQuery::default()
        })
        .unwrap();
    let sound = sounds.first().expect("Haze has ability sounds");
    let audio = assets.clip_audio(&sound.clips[0]).unwrap();
    assert!(!audio.bytes.is_empty());

    let swap = assets
        .build_sound_swap(
            &SoundSwapRequest {
                target: Some(sound.swap_target()),
                gain_db: Some(3.0),
                ..SoundSwapRequest::default()
            },
            &out,
            "sigil_soundswap_haze",
        )
        .unwrap();
    assert!(!swap.entries.is_empty());

    let slots = assets.hero_card_slots("Haze").unwrap();
    let card = assets
        .build_hero_card_mod(
            &[TextureUpload {
                entry: slots[0].entry.clone(),
                png: png(64, 64),
            }],
            &out,
            "sigil_card_haze",
        )
        .unwrap();
    assert_eq!(card.entries, [slots[0].entry.clone()]);

    let voice = assets
        .sound_catalog(&cache, &SoundQuery {
            hero: "Haze".to_string(),
            kinds: vec![SoundKind::Voice],
            limit: Some(1),
            ..SoundQuery::default()
        })
        .unwrap();
    assert!(!voice.is_empty());
    let _ = fs::remove_dir_all(&root);
}
