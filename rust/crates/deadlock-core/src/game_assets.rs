//! The game's own assets for the Locker: texture previews, the sound browser, sound swaps and
//! hero card art.
//!
//! Everything reads `citadel/pak01_dir.vpk` in place through [`crate::vpk`]; the game's paks are
//! never copied. Formats come from vpkmerge (MIT): `morphic` decodes `.vtex_c` textures and
//! `.vsnd_c` clips, `vpkmerge_core` classifies texture paths, indexes sound events and mints
//! replacement clips and card textures.
//!
//! Memory stays low on purpose. A texture preview decodes the smallest mip that still covers the
//! requested edge, crops the padding, downsamples to at most [`THUMBNAIL_MAX_EDGE`] pixels and
//! drops the decode before returning. Previews land in a small on-disk cache keyed by the game
//! build, so a reopened page reads a few kilobytes instead of decoding again. The sound index is
//! built once per game build and stored as one JSON file per hero, so a hero page loads only that
//! hero's rows.
//!
//! The two builders ([`GameAssets::build_sound_swap`] and [`GameAssets::build_hero_card_mod`])
//! write a staging folder with the minted files at the game's own paths, pack it with
//! [`crate::vpk_write::pack_vpk_single`] into a new mod VPK in a folder the caller picks (app
//! data), and remove the staging folder. The result becomes a mod through the existing import and
//! install path. Nothing here writes to the game install.
//!
//! Behaviour is the union of Grimoire (Foundry catalog, sound swap, custom hero cards) and
//! Deadlock Mod Manager (Foundry sound groups, card variants, per-sound volume).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use morphic::{DecodeOptions, ImageData};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use vpkmerge_core::{HeroSoundCategory, TextureCategory};

use crate::error::{Error, Result};
use crate::game_index::game_vpk_path;
use crate::locker::{ROSTER, RosterHero, hero_asset_key, hero_for_codename};
use crate::util::write_atomic;
use crate::vpk::{VpkDirectory, VpkEntry, is_safe_vpk_path, read_vpk_directory, read_vpk_entry_data};
use crate::vpk_write::{PackOptions, pack_vpk_single};

/// Largest preview edge in pixels. Requests above this are clamped.
pub const THUMBNAIL_MAX_EDGE: u32 = 256;
/// Default cap for [`GameAssets::prune_cache`]: 64 MiB of previews and indexes.
pub const DEFAULT_ASSET_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest audio file a sound swap accepts.
pub const MAX_SWAP_AUDIO_BYTES: usize = 64 * 1024 * 1024;
/// Largest PNG a card art upload accepts (Grimoire's limit).
pub const MAX_CARD_PNG_BYTES: usize = 32 * 1024 * 1024;

/// Refuse to decode a texture mip above this many pixels (a 4096 square). Keeps one preview's
/// transient decode bounded even for a texture that ships without a mip chain.
const MAX_DECODE_PIXELS: u64 = 4096 * 4096;
const THUMB_MAGIC: &[u8; 4] = b"SGTH";
const THUMB_VERSION: u8 = 1;
const THUMBS_DIR: &str = "thumbs";
const SOUNDS_DIR: &str = "sounds";
const SOUND_INDEX_FILE: &str = "index.json";
const SOUND_INDEX_SCHEMA: u32 = 1;
const HERO_SOUNDEVENTS_PREFIX: &str = "soundevents/hero/";
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
/// Card variants in display order: Grimoire's `heroCardVariantRank` and DMM's `VARIANT_ORDER`.
const CARD_VARIANT_ORDER: [&str; 11] = [
    "card",
    "vertical",
    "card_critical",
    "card_gloat",
    "minimap",
    "mm",
    "small",
    "sm",
    "background",
    "weapon",
    "other",
];

/// What a game texture is used for, from its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureKind {
    /// `panorama/images/heroes/<code>_card*` and `_vertical`: the hero cards.
    HeroCard,
    /// The other `panorama/images/heroes/` art: minimap and small portraits, backgrounds.
    HeroPortrait,
    /// `panorama/images/hud/abilities/`: HUD ability icons.
    AbilityIcon,
    /// Shop and upgrade item icons.
    ItemIcon,
    /// `models/heroes*/<hero>/`: hero model textures, the skin and recolor targets.
    SkinTexture,
    /// `materials/particle/abilities/<hero>/`: ability effect textures.
    AbilityVfx,
    Other,
}

impl TextureKind {
    pub fn id(self) -> &'static str {
        match self {
            TextureKind::HeroCard => "heroCard",
            TextureKind::HeroPortrait => "heroPortrait",
            TextureKind::AbilityIcon => "abilityIcon",
            TextureKind::ItemIcon => "itemIcon",
            TextureKind::SkinTexture => "skinTexture",
            TextureKind::AbilityVfx => "abilityVfx",
            TextureKind::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TextureKind::HeroCard => "Hero cards",
            TextureKind::HeroPortrait => "Portraits",
            TextureKind::AbilityIcon => "Ability icons",
            TextureKind::ItemIcon => "Item icons",
            TextureKind::SkinTexture => "Skin textures",
            TextureKind::AbilityVfx => "Ability effects",
            TextureKind::Other => "Other",
        }
    }
}

/// One `.vtex_c` in the game, classified from its path. No pixels are read to build this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameTexture {
    /// Exact game path, usable as a replacement target.
    pub path: String,
    pub kind: TextureKind,
    /// Codename the path encodes (`astro`, `archer`), if any.
    pub codename: Option<String>,
    /// Roster display name for that codename, if it is a playable hero.
    pub hero: Option<String>,
    /// Search label from the file name, for example `shotgun` or `bow color`.
    pub label: String,
    /// Card variant (`card`, `card_critical`, `minimap`) for hero card and portrait art.
    pub variant: Option<String>,
}

/// Filter for [`GameAssets::texture_catalog`]. Every set field must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureQuery {
    /// Display name or any codename (`Abrams`, `atlas`, `bull`).
    pub hero: Option<String>,
    /// Kinds to keep. Empty keeps every kind.
    pub kinds: Vec<TextureKind>,
    /// Case-insensitive words that must all appear in the label or path.
    pub search: Option<String>,
    pub limit: Option<usize>,
}

/// A bounded preview: tightly packed RGBA8, at most [`THUMBNAIL_MAX_EDGE`] on the longer side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetThumbnail {
    pub width: u32,
    pub height: u32,
    /// Real size of the full texture (mip 0, padding removed).
    pub source_width: u32,
    pub source_height: u32,
    /// `width * height * 4` bytes, row-major, top-left origin, straight alpha.
    pub rgba: Vec<u8>,
}

/// Which family a sound belongs to. Union of Grimoire's hero sound categories and DMM's sound
/// groups (ability, voice, weapon, other).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SoundKind {
    Ability,
    Weapon,
    Movement,
    Melee,
    Voice,
    Other,
}

impl SoundKind {
    pub fn id(self) -> &'static str {
        match self {
            SoundKind::Ability => "ability",
            SoundKind::Weapon => "weapon",
            SoundKind::Movement => "movement",
            SoundKind::Melee => "melee",
            SoundKind::Voice => "voice",
            SoundKind::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SoundKind::Ability => "Abilities",
            SoundKind::Weapon => "Weapon",
            SoundKind::Movement => "Movement",
            SoundKind::Melee => "Melee",
            SoundKind::Voice => "Voice lines",
            SoundKind::Other => "Other",
        }
    }
}

/// One playable sound event: a hero gameplay sound or a voice line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSound {
    /// Sound event name, for example `Haze.Finesse.Dagger.Cast`.
    pub event: String,
    pub kind: SoundKind,
    /// Codename the game files use for this hero (`abrams`, `gigawatt`), if any.
    pub codename: Option<String>,
    /// Roster display name, if the codename is a playable hero.
    pub hero: Option<String>,
    /// Ability display name for ability sounds (`Finesse`, `Siphon Life`).
    pub ability: Option<String>,
    /// Ability slot 1 to 4 when the game files say which.
    pub slot: Option<u8>,
    /// Search label, for example `Dagger Cast` or `ally atlas killed in lane`.
    pub label: String,
    /// Clip paths the event plays, as the event lists them (`.vsnd`). More than one is a pool
    /// the game picks from at random.
    pub clips: Vec<String>,
    pub duration: Option<f64>,
    /// English subtitle, when the caption database has one (rare for hero voice lines).
    pub caption: Option<String>,
    /// `soundevents/hero/<code>.vsndevts_c` for gameplay sounds. Voice lines have none.
    pub soundevents: Option<String>,
}

impl GameSound {
    /// What a sound swap overrides for this sound: every clip in the event's pool.
    pub fn swap_target(&self) -> SoundSwapTarget {
        match &self.soundevents {
            Some(soundevents) => SoundSwapTarget::Event {
                soundevents: soundevents.clone(),
                event: self.event.clone(),
            },
            None => SoundSwapTarget::Clips {
                clips: self.clips.clone(),
            },
        }
    }
}

/// One hero (or the shared bucket) in the sound index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundHero {
    /// File key: [`hero_asset_key`] of the display name, or of the codename for a non-roster
    /// speaker, or `other` for sounds without a speaker.
    pub key: String,
    /// Display name, or the codename when it is not a roster hero.
    pub name: String,
    pub sounds: usize,
    pub voice_lines: usize,
}

/// Filter for [`GameAssets::sound_catalog`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundQuery {
    /// Display name, codename or a [`SoundHero::key`].
    pub hero: String,
    /// Kinds to keep. Empty keeps every kind.
    pub kinds: Vec<SoundKind>,
    /// Ability slot to keep (1 to 4).
    pub slot: Option<u8>,
    /// Case-insensitive words that must all appear in the label, event, ability or caption.
    pub search: Option<String>,
    pub limit: Option<usize>,
}

/// Playable audio cut out of a `.vsnd_c`, for the shared preview player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipAudio {
    pub bytes: Vec<u8>,
    /// `audio/mpeg` or `audio/wav`.
    pub mime: &'static str,
    /// `mp3` or `wav`.
    pub extension: &'static str,
}

/// What a sound swap overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SoundSwapTarget {
    /// Every clip in an event's pool, read from its soundevents file (gameplay sounds). The
    /// soundevents file itself is left alone, so the event keeps working with any other mod.
    Event { soundevents: String, event: String },
    /// Explicit clips, overridden in place (voice lines, or a single clip).
    Clips { clips: Vec<String> },
}

/// Whether the minted clips loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoopMode {
    /// Keep each original clip's own loop setting.
    #[default]
    Auto,
    On,
    Off,
}

/// A sound swap: the player's audio minted over the target clips.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundSwapRequest {
    pub target: Option<SoundSwapTarget>,
    /// The player's audio: MP3, or a 16-bit PCM WAV for clips the game stores as PCM. `None`
    /// re-mints each clip's own audio, which with `gain_db` or `trim_ms` is a volume or length
    /// edit of the game's sound (DMM's per-sound volume).
    #[serde(skip)]
    pub audio: Option<Vec<u8>>,
    /// File name of the audio, for error messages.
    pub audio_name: String,
    pub loop_mode: LoopMode,
    /// Keep only this window, in milliseconds (MP3 only, snapped to frames of about 26 ms).
    pub trim_ms: Option<(u32, u32)>,
    /// Lossless gain in decibels before minting (MP3 only, steps of about 1.5 dB).
    pub gain_db: Option<f64>,
}

/// One card art slot the game ships for a hero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroCardSlot {
    /// `card`, `vertical`, `card_critical`, `card_gloat`, `minimap`, `small`, ...
    pub variant: String,
    /// Template entry path. An upload replaces exactly this file.
    pub entry: String,
    /// Size an upload is resized to (the template's stored mip 0).
    pub width: u32,
    pub height: u32,
    /// Texture format name, for example `Bc7`.
    pub format: String,
}

/// One PNG for one texture: hero card art, or any 8-bit game texture such as an ability icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureUpload {
    /// Game path of the template `.vtex_c`, usually a [`HeroCardSlot::entry`].
    pub entry: String,
    pub png: Vec<u8>,
}

/// A mod VPK built in app data, ready for the import review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltAssetMod {
    /// The single-file `<base>_dir.vpk`.
    pub vpk_path: PathBuf,
    /// Game paths the mod replaces.
    pub entries: Vec<String>,
    /// Pool clips that were not in pak01 and keep the original sound. Empty on a clean build.
    pub skipped: Vec<String>,
    /// Whether the minted clips loop. `None` for card art.
    pub looped: Option<bool>,
    pub total_bytes: u64,
}

/// The game's `pak01_dir.vpk`, opened for reading. Holds the directory (paths and offsets, no
/// file bodies). Open it on a worker, keep it while a Locker asset page is open and drop it when
/// the page closes.
#[derive(Debug)]
pub struct GameAssets {
    pak_path: PathBuf,
    directory: VpkDirectory,
    fingerprint: String,
}

impl GameAssets {
    /// Open `<game_root>/game/citadel/pak01_dir.vpk`.
    pub fn open(game_root: &Path) -> Result<Self> {
        Self::open_vpk(&game_vpk_path(game_root))
    }

    /// Open a specific `_dir.vpk`. Tests and custom installs use this.
    pub fn open_vpk(pak_path: &Path) -> Result<Self> {
        let fingerprint = build_fingerprint(pak_path)?;
        let mut directory = read_vpk_directory(pak_path)?;
        // `by_path` holds every entry already; the ordered copy would double the memory.
        directory.entries = Vec::new();
        Ok(Self {
            pak_path: pak_path.to_path_buf(),
            directory,
            fingerprint,
        })
    }

    pub fn pak_path(&self) -> &Path {
        &self.pak_path
    }

    /// Build key from the pak's size and modification time. Caches are keyed by it.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn entry(&self, path: &str) -> Option<&VpkEntry> {
        self.directory.by_path.get(&path.to_ascii_lowercase())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let entry = self.entry(path).ok_or_else(|| {
            Error::msg(format!("{path} is not in {}.", self.pak_path.display()))
        })?;
        read_vpk_entry_data(&self.directory, entry)
    }

    // ----- textures ------------------------------------------------------------------------

    /// Every game texture matching `query`, sorted by kind then path. Reads the directory only.
    pub fn texture_catalog(&self, query: &TextureQuery) -> Vec<GameTexture> {
        let hero = query.hero.as_deref().and_then(HeroMatch::new);
        let words = search_words(query.search.as_deref());
        let mut textures: Vec<GameTexture> = self
            .directory
            .by_path
            .values()
            .filter(|entry| entry.path.to_ascii_lowercase().ends_with(".vtex_c"))
            .map(|entry| classify_game_texture(&entry.path))
            .filter(|texture| query.kinds.is_empty() || query.kinds.contains(&texture.kind))
            .filter(|texture| {
                hero.as_ref()
                    .is_none_or(|hero| hero.matches(texture.codename.as_deref()))
            })
            .filter(|texture| {
                let haystack = format!("{} {}", texture.label, texture.path).to_ascii_lowercase();
                words.iter().all(|word| haystack.contains(word.as_str()))
            })
            .collect();
        textures.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
        if let Some(limit) = query.limit {
            textures.truncate(limit);
        }
        textures
    }

    /// A bounded RGBA preview of one texture, through the on-disk cache under `cache_dir`.
    /// `max_edge` is clamped to [`THUMBNAIL_MAX_EDGE`]. A cache miss decodes once and writes
    /// a compressed preview; nothing full-size is kept.
    pub fn texture_thumbnail(
        &self,
        path: &str,
        max_edge: u32,
        cache_dir: &Path,
    ) -> Result<AssetThumbnail> {
        let edge = max_edge.clamp(1, THUMBNAIL_MAX_EDGE);
        let file = self.thumbnail_cache_path(cache_dir, path, edge);
        if let Some(cached) = read_cached_thumbnail(&file) {
            return Ok(cached);
        }
        let bytes = self.read(path)?;
        let thumbnail = decode_thumbnail(&bytes, edge)
            .map_err(|error| Error::msg(format!("{path}: {error}")))?;
        drop(bytes);
        let fingerprint_dir = cache_dir.join(THUMBS_DIR).join(&self.fingerprint);
        if !fingerprint_dir.exists() {
            prune_stale_fingerprints(&cache_dir.join(THUMBS_DIR), &self.fingerprint);
        }
        // A cache write failure only costs a decode next time.
        let _ = write_atomic(&file, &encode_cached_thumbnail(&thumbnail));
        Ok(thumbnail)
    }

    fn thumbnail_cache_path(&self, cache_dir: &Path, path: &str, edge: u32) -> PathBuf {
        let mut hasher = Sha1::new();
        hasher.update(path.to_ascii_lowercase().as_bytes());
        let digest = hasher.finalize();
        let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        cache_dir
            .join(THUMBS_DIR)
            .join(&self.fingerprint)
            .join(edge.to_string())
            .join(format!("{name}.thumb"))
    }

    /// Delete caches for older game builds, then the oldest previews until the cache is at most
    /// `max_bytes`. Returns the bytes freed. Run at startup or when a page closes.
    pub fn prune_cache(&self, cache_dir: &Path, max_bytes: u64) -> Result<u64> {
        let mut freed = 0;
        for sub in [THUMBS_DIR, SOUNDS_DIR] {
            freed += prune_stale_fingerprints(&cache_dir.join(sub), &self.fingerprint);
        }
        let mut files = Vec::new();
        collect_files(&cache_dir.join(THUMBS_DIR), &mut files);
        let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
        let sound_dir = cache_dir.join(SOUNDS_DIR).join(&self.fingerprint);
        let mut index_files = Vec::new();
        collect_files(&sound_dir, &mut index_files);
        total += index_files.iter().map(|(_, size, _)| *size).sum::<u64>();
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
                freed += size;
            }
        }
        Ok(freed)
    }

    // ----- sounds --------------------------------------------------------------------------

    /// Heroes in the sound index, building it on first use for this game build.
    pub fn sound_heroes(&self, cache_dir: &Path) -> Result<Vec<SoundHero>> {
        Ok(self.ensure_sound_index(cache_dir)?.heroes)
    }

    /// Sounds for one hero, filtered by `query`. Loads only that hero's file from the cache;
    /// the first call for a game build indexes every soundevents file once (a second or two).
    pub fn sound_catalog(&self, cache_dir: &Path, query: &SoundQuery) -> Result<Vec<GameSound>> {
        let index = self.ensure_sound_index(cache_dir)?;
        let Some(key) = resolve_sound_key(&index.heroes, &query.hero) else {
            return Ok(Vec::new());
        };
        let file = self.sound_dir(cache_dir).join(format!("{key}.json"));
        let mut sounds: Vec<GameSound> = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let words = search_words(query.search.as_deref());
        sounds.retain(|sound| {
            (query.kinds.is_empty() || query.kinds.contains(&sound.kind))
                && query.slot.is_none_or(|slot| sound.slot == Some(slot))
                && {
                    let haystack = format!(
                        "{} {} {} {}",
                        sound.label,
                        sound.event,
                        sound.ability.as_deref().unwrap_or(""),
                        sound.caption.as_deref().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    words.iter().all(|word| haystack.contains(word.as_str()))
                }
        });
        if let Some(limit) = query.limit {
            sounds.truncate(limit);
        }
        Ok(sounds)
    }

    fn sound_dir(&self, cache_dir: &Path) -> PathBuf {
        cache_dir.join(SOUNDS_DIR).join(&self.fingerprint)
    }

    fn ensure_sound_index(&self, cache_dir: &Path) -> Result<SoundIndexFile> {
        let dir = self.sound_dir(cache_dir);
        let index_path = dir.join(SOUND_INDEX_FILE);
        if let Ok(bytes) = fs::read(&index_path)
            && let Ok(index) = serde_json::from_slice::<SoundIndexFile>(&bytes)
            && index.schema == SOUND_INDEX_SCHEMA
            && index.fingerprint == self.fingerprint
        {
            return Ok(index);
        }

        prune_stale_fingerprints(&cache_dir.join(SOUNDS_DIR), &self.fingerprint);
        let mut by_hero: BTreeMap<String, (String, Vec<GameSound>)> = BTreeMap::new();
        let hero_sounds = vpkmerge_core::build_hero_sound_index(&self.pak_path)
            .map_err(|error| Error::msg(format!("Indexing hero sounds failed: {error:#}")))?;
        for sound in hero_sounds {
            let game_sound = game_sound_from_hero(sound);
            push_sound(&mut by_hero, game_sound);
        }
        let voice_lines = vpkmerge_core::build_voiceline_index(&self.pak_path)
            .map_err(|error| Error::msg(format!("Indexing voice lines failed: {error:#}")))?;
        for line in voice_lines {
            push_sound(&mut by_hero, game_sound_from_voice(line));
        }

        let mut heroes = Vec::with_capacity(by_hero.len());
        for (key, (name, sounds)) in &by_hero {
            let voice_lines = sounds.iter().filter(|s| s.kind == SoundKind::Voice).count();
            heroes.push(SoundHero {
                key: key.clone(),
                name: name.clone(),
                sounds: sounds.len() - voice_lines,
                voice_lines,
            });
            write_atomic(&dir.join(format!("{key}.json")), &serde_json::to_vec(sounds)?)?;
        }
        heroes.sort_by_key(|hero| hero.name.to_ascii_lowercase());
        let index = SoundIndexFile {
            schema: SOUND_INDEX_SCHEMA,
            fingerprint: self.fingerprint.clone(),
            heroes,
        };
        // Written last: its presence marks a complete index.
        write_atomic(&index_path, &serde_json::to_vec(&index)?)?;
        Ok(index)
    }

    /// Playable audio for one clip (`.vsnd` or `.vsnd_c` path): MP3 as stored, or PCM wrapped
    /// as a WAV. Clips are tens of kilobytes, so nothing is cached.
    pub fn clip_audio(&self, clip: &str) -> Result<ClipAudio> {
        let entry = compiled_clip_entry(clip);
        let bytes = self.read(&entry)?;
        match morphic::extract_vsnd_audio(&bytes) {
            Ok(morphic::VsndAudio::Mp3(mp3)) => Ok(ClipAudio {
                bytes: mp3,
                mime: "audio/mpeg",
                extension: "mp3",
            }),
            Ok(morphic::VsndAudio::WavPcm16 { wav, .. }) => Ok(ClipAudio {
                bytes: wav,
                mime: "audio/wav",
                extension: "wav",
            }),
            Err(error) => Err(Error::msg(format!(
                "{entry} cannot be previewed: {error}."
            ))),
        }
    }

    /// Mint the player's audio over the target clips and pack a new mod VPK
    /// `<out_dir>/<base_name>_dir.vpk`. A staging folder under `out_dir` holds the minted
    /// clips at the game's own paths while packing and is removed afterwards.
    pub fn build_sound_swap(
        &self,
        request: &SoundSwapRequest,
        out_dir: &Path,
        base_name: &str,
    ) -> Result<BuiltAssetMod> {
        let Some(target) = &request.target else {
            return Err(Error::msg("Pick a sound to swap first."));
        };
        let input = match &request.audio {
            Some(audio) => Some(prepare_swap_audio(audio, request)?),
            None => {
                if request.gain_db.is_none() && request.trim_ms.is_none() {
                    return Err(Error::msg(
                        "Pick an MP3 to swap in, or set a volume or trim for the original sound.",
                    ));
                }
                None
            }
        };

        let pool = match target {
            SoundSwapTarget::Event { soundevents, event } => self.event_pool(soundevents, event)?,
            SoundSwapTarget::Clips { clips } => clips.clone(),
        };
        if pool.is_empty() {
            return Err(Error::msg("The sound has no clips to swap."));
        }

        let mut files = Vec::new();
        let mut skipped = Vec::new();
        let mut looped = match request.loop_mode {
            LoopMode::Auto => None,
            LoopMode::On => Some(true),
            LoopMode::Off => Some(false),
        };
        let mut seen = BTreeSet::new();
        for clip in &pool {
            let entry = compiled_clip_entry(clip);
            if !seen.insert(entry.to_ascii_lowercase()) {
                continue;
            }
            let Ok(donor) = self.read(&entry) else {
                skipped.push(clip.clone());
                continue;
            };
            let this_looped = match looped {
                Some(value) => value,
                None => {
                    let value = vpkmerge_core::donor_is_looped(&donor)
                        .map_err(|error| Error::msg(format!("{entry}: {error:#}")))?;
                    looped = Some(value);
                    value
                }
            };
            let minted = match &input {
                Some(SwapAudio::Mp3(mp3)) => mint_mp3(&donor, mp3, this_looped, &entry)?,
                Some(SwapAudio::Wav(wav)) => mint_wav(&donor, wav, &entry)?,
                None => {
                    let own = match morphic::extract_vsnd_audio(&donor) {
                        Ok(morphic::VsndAudio::Mp3(mp3)) => mp3,
                        _ => {
                            return Err(Error::msg(format!(
                                "{entry} is not stored as MP3, so its volume and length cannot be edited."
                            )));
                        }
                    };
                    let edited = edit_mp3(&own, request.trim_ms, request.gain_db)?;
                    mint_mp3(&donor, &edited, this_looped, &entry)?
                }
            };
            files.push((entry, minted));
        }
        if files.is_empty() {
            return Err(Error::msg(format!(
                "None of the {} clips were found in {}. They may live in another pak.",
                pool.len(),
                self.pak_path.display()
            )));
        }

        let mut built = pack_staged(&files, out_dir, base_name)?;
        built.skipped = skipped;
        built.looped = looped;
        Ok(built)
    }

    fn event_pool(&self, soundevents: &str, event: &str) -> Result<Vec<String>> {
        let bytes = self.read(soundevents)?;
        let events = vpkmerge_core::SoundEvents::from_bytes(bytes)
            .map_err(|error| Error::msg(format!("{soundevents}: {error:#}")))?;
        let Some(value) = events.root.get(event) else {
            return Err(Error::msg(format!("{event} is not in {soundevents}.")));
        };
        let pool = match value.get("vsnd_files") {
            Some(morphic::kv3::Value::Array(items)) => items
                .iter()
                .filter_map(morphic::kv3::Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(morphic::kv3::Value::String(clip)) => vec![clip.clone()],
            _ => Vec::new(),
        };
        if pool.is_empty() {
            return Err(Error::msg(format!(
                "{event} has no clips to swap. It is a template or parameter row."
            )));
        }
        Ok(pool)
    }

    // ----- hero card art -------------------------------------------------------------------

    /// The card art slots the game ships for a hero, in display order.
    pub fn hero_card_slots(&self, hero: &str) -> Result<Vec<HeroCardSlot>> {
        let Some(matcher) = HeroMatch::new(hero) else {
            return Err(Error::msg(format!("{hero} is not a hero name or codename.")));
        };
        let mut slots = Vec::new();
        for entry in self.directory.by_path.values() {
            let texture = classify_game_texture(&entry.path);
            if !matches!(texture.kind, TextureKind::HeroCard | TextureKind::HeroPortrait)
                || !matcher.matches(texture.codename.as_deref())
            {
                continue;
            }
            let bytes = read_vpk_entry_data(&self.directory, entry)?;
            let Ok(info) = morphic::inspect(&bytes) else {
                continue;
            };
            slots.push(HeroCardSlot {
                variant: texture.variant.unwrap_or_else(|| "other".to_string()),
                entry: entry.path.clone(),
                width: u32::from(info.width),
                height: u32::from(info.height),
                format: format!("{:?}", info.format),
            });
        }
        slots.sort_by(|a, b| {
            card_variant_rank(&a.variant)
                .cmp(&card_variant_rank(&b.variant))
                .then_with(|| a.entry.cmp(&b.entry))
        });
        Ok(slots)
    }

    /// Splice each PNG into its template texture (resized to the template, format and mips
    /// kept) and pack a new mod VPK `<out_dir>/<base_name>_dir.vpk`. Works for hero cards and
    /// any other 8-bit game texture; HDR templates are refused.
    pub fn build_hero_card_mod(
        &self,
        uploads: &[TextureUpload],
        out_dir: &Path,
        base_name: &str,
    ) -> Result<BuiltAssetMod> {
        if uploads.is_empty() {
            return Err(Error::msg("Add at least one card image."));
        }
        let mut files = Vec::with_capacity(uploads.len());
        let mut seen = BTreeSet::new();
        for upload in uploads {
            let Some(entry) = self.entry(&upload.entry) else {
                return Err(Error::msg(format!(
                    "{} is not a game texture, so there is no template for it.",
                    upload.entry
                )));
            };
            if !entry.path.to_ascii_lowercase().ends_with(".vtex_c") {
                return Err(Error::msg(format!("{} is not a texture.", entry.path)));
            }
            if !seen.insert(entry.path.to_ascii_lowercase()) {
                return Err(Error::msg(format!("{} has two images.", entry.path)));
            }
            check_png(&upload.png, &entry.path)?;
            let template = read_vpk_entry_data(&self.directory, entry)?;
            let minted = vpkmerge_core::build_icon_from_template(&template, &upload.png)
                .map_err(|error| Error::msg(format!("{}: {error:#}", entry.path)))?;
            files.push((entry.path.clone(), minted));
        }
        pack_staged(&files, out_dir, base_name)
    }
}

// ----- textures: classification and decode ----------------------------------------------------

/// Classify one `.vtex_c` path. Pure; vpkmerge's path taxonomy plus the card variant split.
pub fn classify_game_texture(path: &str) -> GameTexture {
    let classified = vpkmerge_core::classify_texture(path);
    let codename = classified.hero.map(|code| code.to_ascii_lowercase());
    let variant = if classified.category == TextureCategory::HeroImage {
        codename.as_deref().map(|code| card_variant(path, code))
    } else {
        None
    };
    let kind = match classified.category {
        TextureCategory::HeroImage => match variant.as_deref() {
            Some("card" | "card_critical" | "card_gloat" | "vertical") => TextureKind::HeroCard,
            _ => TextureKind::HeroPortrait,
        },
        TextureCategory::AbilityIcon => TextureKind::AbilityIcon,
        TextureCategory::ItemIcon => TextureKind::ItemIcon,
        TextureCategory::HeroModel => TextureKind::SkinTexture,
        TextureCategory::AbilityVfx => TextureKind::AbilityVfx,
        TextureCategory::Other => TextureKind::Other,
    };
    let hero = codename
        .as_deref()
        .and_then(roster_for_codename)
        .map(|hero| hero.name.to_string());
    GameTexture {
        path: path.to_string(),
        kind,
        codename,
        hero,
        label: classified.label,
        variant,
    }
}

/// `panorama/images/heroes/hornet_card_critical_psd.vtex_c` with codename `hornet` gives
/// `card_critical`; `_mm` and `_sm` read as `minimap` and `small`.
fn card_variant(path: &str, codename: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let mut stem = file.strip_suffix(".vtex_c").unwrap_or(&file);
    if let Some((head, tail)) = stem.rsplit_once('_')
        && tail.len() == 8
        && tail.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        stem = head;
    }
    let stem = stem
        .strip_suffix("_psd")
        .or_else(|| stem.strip_suffix("_png"))
        .unwrap_or(stem);
    let rest = stem
        .strip_prefix(codename)
        .map(|rest| rest.trim_start_matches('_'))
        .unwrap_or(stem);
    match rest {
        "" => "other".to_string(),
        "mm" => "minimap".to_string(),
        "sm" => "small".to_string(),
        other => other.to_string(),
    }
}

fn card_variant_rank(variant: &str) -> usize {
    CARD_VARIANT_ORDER
        .iter()
        .position(|known| *known == variant)
        .unwrap_or(CARD_VARIANT_ORDER.len())
}

/// Decode `vtex` to an RGBA preview no larger than `max_edge`. Decodes the smallest mip that
/// still covers `max_edge`, crops non-power-of-two padding, then box-filters down.
pub fn decode_thumbnail(vtex: &[u8], max_edge: u32) -> Result<AssetThumbnail> {
    let max_edge = max_edge.clamp(1, THUMBNAIL_MAX_EDGE);
    let info = morphic::inspect(vtex)
        .map_err(|error| Error::msg(format!("not a readable texture ({error})")))?;
    let mip = choose_mip(
        u32::from(info.width),
        u32::from(info.height),
        info.mip_count,
        max_edge,
    );
    let stored = (
        (u32::from(info.width) >> mip).max(1),
        (u32::from(info.height) >> mip).max(1),
    );
    if u64::from(stored.0) * u64::from(stored.1) > MAX_DECODE_PIXELS {
        return Err(Error::msg(format!(
            "{}x{} is too large to preview",
            stored.0, stored.1
        )));
    }
    let image = morphic::decode_at(
        vtex,
        &DecodeOptions {
            mip,
            slice: 0,
            face: 0,
        },
    )
    .map_err(|error| Error::msg(format!("decode failed ({error})")))?;
    let (actual_width, actual_height) = info.actual_mip_dims(mip);
    let image = if actual_width < image.width || actual_height < image.height {
        morphic::crop_to_actual(&image, actual_width, actual_height)
    } else {
        image
    };
    let (width, height) = (image.width.max(1), image.height.max(1));
    let rgba = match image.data {
        ImageData::Rgba8(bytes) => bytes,
        ImageData::Rgba16F(halfs) => halfs
            .iter()
            .map(|half| unit_to_byte(half.to_f32()))
            .collect(),
    };
    let expected = width as usize * height as usize * 4;
    if rgba.len() < expected {
        return Err(Error::msg("decoded texture is smaller than its header says"));
    }
    let (target_width, target_height) = fit_within(width, height, max_edge);
    let rgba = downsample(&rgba, width, height, target_width, target_height);
    Ok(AssetThumbnail {
        width: target_width,
        height: target_height,
        source_width: u32::from(info.actual_width),
        source_height: u32::from(info.actual_height),
        rgba,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Smallest mip whose longer edge is still at least `max_edge` (vpkmerge's rule).
fn choose_mip(width: u32, height: u32, mip_count: u8, max_edge: u32) -> u8 {
    let last = mip_count.saturating_sub(1);
    let mut chosen = 0u8;
    for mip in 0..=last {
        let edge = (width >> mip).max(1).max((height >> mip).max(1));
        if edge >= max_edge {
            chosen = mip;
        } else {
            break;
        }
    }
    chosen.min(last)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longer = width.max(height);
    if longer <= max_edge {
        return (width, height);
    }
    let scale = f64::from(max_edge) / f64::from(longer);
    (
        ((f64::from(width) * scale).round() as u32).clamp(1, max_edge),
        ((f64::from(height) * scale).round() as u32).clamp(1, max_edge),
    )
}

/// Area-average resample with alpha-weighted color, so transparent edges do not darken.
#[allow(clippy::cast_possible_truncation)]
fn downsample(src: &[u8], width: u32, height: u32, target_width: u32, target_height: u32) -> Vec<u8> {
    if width == target_width && height == target_height {
        return src[..width as usize * height as usize * 4].to_vec();
    }
    let (sw, sh, tw, th) = (
        u64::from(width),
        u64::from(height),
        u64::from(target_width),
        u64::from(target_height),
    );
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        let y0 = ty * sh / th;
        let y1 = ((ty + 1) * sh / th).max(y0 + 1).min(sh);
        for tx in 0..tw {
            let x0 = tx * sw / tw;
            let x1 = ((tx + 1) * sw / tw).max(x0 + 1).min(sw);
            let (mut r, mut g, mut b, mut a, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                let row = (y * sw) as usize;
                for x in x0..x1 {
                    let index = (row + x as usize) * 4;
                    let alpha = u64::from(src[index + 3]);
                    r += u64::from(src[index]) * alpha;
                    g += u64::from(src[index + 1]) * alpha;
                    b += u64::from(src[index + 2]) * alpha;
                    a += alpha;
                    count += 1;
                }
            }
            match (r.checked_div(a), g.checked_div(a), b.checked_div(a)) {
                (Some(r), Some(g), Some(b)) => {
                    out.extend_from_slice(&[r as u8, g as u8, b as u8, (a / count) as u8]);
                }
                _ => out.extend_from_slice(&[0, 0, 0, 0]),
            }
        }
    }
    out
}

fn encode_cached_thumbnail(thumbnail: &AssetThumbnail) -> Vec<u8> {
    let mut out = Vec::with_capacity(21 + thumbnail.rgba.len() / 2);
    out.extend_from_slice(THUMB_MAGIC);
    out.push(THUMB_VERSION);
    for value in [
        thumbnail.width,
        thumbnail.height,
        thumbnail.source_width,
        thumbnail.source_height,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    let mut encoder = ZlibEncoder::new(out, Compression::fast());
    // Writing into a Vec cannot fail.
    let _ = encoder.write_all(&thumbnail.rgba);
    encoder.finish().unwrap_or_default()
}

fn read_cached_thumbnail(file: &Path) -> Option<AssetThumbnail> {
    let bytes = fs::read(file).ok()?;
    decode_cached_thumbnail(&bytes)
}

fn decode_cached_thumbnail(bytes: &[u8]) -> Option<AssetThumbnail> {
    if bytes.len() < 21 || &bytes[..4] != THUMB_MAGIC || bytes[4] != THUMB_VERSION {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4]));
    let (width, height) = (word(5), word(9));
    if width == 0 || height == 0 || width > THUMBNAIL_MAX_EDGE || height > THUMBNAIL_MAX_EDGE {
        return None;
    }
    let expected = width as usize * height as usize * 4;
    let mut rgba = Vec::with_capacity(expected);
    ZlibDecoder::new(&bytes[21..])
        .take(expected as u64 + 1)
        .read_to_end(&mut rgba)
        .ok()?;
    if rgba.len() != expected {
        return None;
    }
    Some(AssetThumbnail {
        width,
        height,
        source_width: word(13),
        source_height: word(17),
        rgba,
    })
}

// ----- sounds: index -------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SoundIndexFile {
    schema: u32,
    fingerprint: String,
    heroes: Vec<SoundHero>,
}

fn game_sound_from_hero(sound: vpkmerge_core::HeroSound) -> GameSound {
    let kind = match sound.category {
        HeroSoundCategory::Ability => SoundKind::Ability,
        HeroSoundCategory::Weapon => SoundKind::Weapon,
        HeroSoundCategory::Movement => SoundKind::Movement,
        HeroSoundCategory::Melee => SoundKind::Melee,
        HeroSoundCategory::Other => SoundKind::Other,
    };
    let codename = sound.hero.to_ascii_lowercase();
    GameSound {
        soundevents: Some(format!("{HERO_SOUNDEVENTS_PREFIX}{}.vsndevts_c", sound.hero)),
        hero: roster_for_codename(&codename).map(|hero| hero.name.to_string()),
        codename: Some(codename),
        event: sound.event,
        kind,
        ability: sound.ability,
        slot: sound.slot,
        label: sound.label,
        clips: sound.vsnd,
        duration: sound.duration,
        caption: None,
    }
}

fn game_sound_from_voice(line: vpkmerge_core::VoiceLine) -> GameSound {
    let codename = line.hero.map(|code| code.to_ascii_lowercase());
    GameSound {
        hero: codename
            .as_deref()
            .and_then(roster_for_codename)
            .map(|hero| hero.name.to_string()),
        codename,
        event: line.event,
        kind: SoundKind::Voice,
        ability: None,
        slot: None,
        label: line.label,
        clips: line.vsnd,
        duration: line.duration,
        caption: line.caption,
        soundevents: None,
    }
}

fn push_sound(by_hero: &mut BTreeMap<String, (String, Vec<GameSound>)>, sound: GameSound) {
    let (key, name) = match (&sound.hero, &sound.codename) {
        (Some(hero), _) => (hero_asset_key(hero), hero.clone()),
        (None, Some(code)) if !hero_asset_key(code).is_empty() => {
            (hero_asset_key(code), code.clone())
        }
        _ => ("other".to_string(), "Other".to_string()),
    };
    by_hero.entry(key).or_insert_with(|| (name, Vec::new())).1.push(sound);
}

fn resolve_sound_key(heroes: &[SoundHero], wanted: &str) -> Option<String> {
    let key = hero_asset_key(wanted);
    if heroes.iter().any(|hero| hero.key == key) {
        return Some(key);
    }
    let roster = roster_for_codename(wanted)
        .or_else(|| crate::locker::roster_hero(wanted))
        .map(|hero| hero_asset_key(hero.name))?;
    heroes.iter().any(|hero| hero.key == roster).then_some(roster)
}

// ----- sounds: swap -------------------------------------------------------------------------

enum SwapAudio {
    Mp3(Vec<u8>),
    Wav(Vec<u8>),
}

fn prepare_swap_audio(audio: &[u8], request: &SoundSwapRequest) -> Result<SwapAudio> {
    let name = if request.audio_name.trim().is_empty() {
        "The audio file"
    } else {
        request.audio_name.trim()
    };
    if audio.is_empty() {
        return Err(Error::msg(format!("{name} is empty.")));
    }
    if audio.len() > MAX_SWAP_AUDIO_BYTES {
        return Err(Error::msg(format!(
            "{name} is larger than {} MB.",
            MAX_SWAP_AUDIO_BYTES / (1024 * 1024)
        )));
    }
    if audio.len() >= 12 && &audio[..4] == b"RIFF" && &audio[8..12] == b"WAVE" {
        if request.trim_ms.is_some() || request.gain_db.is_some() {
            return Err(Error::msg(format!(
                "Trim and volume work on MP3 audio. Convert {name} to MP3 first."
            )));
        }
        return Ok(SwapAudio::Wav(audio.to_vec()));
    }
    if vpkmerge_core::parse_mp3_params(audio, false).is_err() {
        return Err(Error::msg(format!(
            "{name} is not an MP3. Sound swaps take MP3 audio: convert it to MP3 first."
        )));
    }
    Ok(SwapAudio::Mp3(edit_mp3(audio, request.trim_ms, request.gain_db)?))
}

fn edit_mp3(mp3: &[u8], trim_ms: Option<(u32, u32)>, gain_db: Option<f64>) -> Result<Vec<u8>> {
    let mut audio = mp3.to_vec();
    if let Some((start, end)) = trim_ms {
        audio = vpkmerge_core::trim_mp3(&audio, start, end)
            .map_err(|error| Error::msg(format!("Trim failed: {error:#}")))?;
    }
    if let Some(gain) = gain_db.filter(|gain| gain.is_finite() && *gain != 0.0) {
        audio = vpkmerge_core::apply_mp3_gain(&audio, gain)
            .map_err(|error| Error::msg(format!("Volume change failed: {error:#}")))?;
    }
    Ok(audio)
}

fn mint_mp3(donor: &[u8], mp3: &[u8], looped: bool, entry: &str) -> Result<Vec<u8>> {
    if matches!(
        morphic::extract_vsnd_audio(donor),
        Ok(morphic::VsndAudio::WavPcm16 { .. })
    ) {
        return Err(Error::msg(format!(
            "{entry} is stored as uncompressed PCM. Swap it with a 16-bit WAV instead of an MP3."
        )));
    }
    vpkmerge_core::mint_swapped_clip(donor, mp3, looped)
        .map_err(|error| Error::msg(format!("{entry}: {error:#}")))
}

fn mint_wav(donor: &[u8], wav: &[u8], entry: &str) -> Result<Vec<u8>> {
    if !matches!(
        morphic::extract_vsnd_audio(donor),
        Ok(morphic::VsndAudio::WavPcm16 { .. })
    ) {
        return Err(Error::msg(format!(
            "{entry} is stored as MP3. Convert the WAV to MP3 first."
        )));
    }
    morphic::encode_vsnd_pcm16_c(donor, wav)
        .map_err(|error| Error::msg(format!("{entry}: the WAV must be 16-bit PCM ({error}).")))
}

/// `foo.vsnd` and `foo` map to the packed `foo.vsnd_c`.
fn compiled_clip_entry(clip: &str) -> String {
    let clip = clip.trim().replace('\\', "/");
    if clip.ends_with(".vsnd_c") {
        clip
    } else if let Some(stem) = clip.strip_suffix(".vsnd") {
        format!("{stem}.vsnd_c")
    } else {
        format!("{clip}.vsnd_c")
    }
}

fn check_png(png: &[u8], entry: &str) -> Result<()> {
    if png.is_empty() {
        return Err(Error::msg(format!("The image for {entry} is empty.")));
    }
    if png.len() > MAX_CARD_PNG_BYTES {
        return Err(Error::msg(format!(
            "The image for {entry} is larger than {} MB.",
            MAX_CARD_PNG_BYTES / (1024 * 1024)
        )));
    }
    if png.len() < PNG_SIGNATURE.len() || png[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(Error::msg(format!(
            "The image for {entry} is not a PNG. Card art takes PNG files."
        )));
    }
    Ok(())
}

// ----- shared helpers ------------------------------------------------------------------------

/// Write `files` into a staging folder under `out_dir`, pack it as one `<base>_dir.vpk` there,
/// and remove the staging folder whatever happens.
fn pack_staged(files: &[(String, Vec<u8>)], out_dir: &Path, base_name: &str) -> Result<BuiltAssetMod> {
    fs::create_dir_all(out_dir)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let staging = out_dir.join(format!(".staging-{}-{nanos}", std::process::id()));
    let result = (|| {
        for (entry, bytes) in files {
            if !is_safe_vpk_path(entry) {
                return Err(Error::msg(format!("{entry} is not a safe game path.")));
            }
            let target = staging.join(entry);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, bytes)?;
        }
        let mut options = PackOptions::new(staging.clone(), out_dir.to_path_buf(), base_name);
        options.chunk_size = 0;
        pack_vpk_single(&options)
    })();
    let _ = fs::remove_dir_all(&staging);
    let packed = result?;
    let mut entries: Vec<String> = files.iter().map(|(entry, _)| entry.clone()).collect();
    entries.sort();
    Ok(BuiltAssetMod {
        vpk_path: packed.dir_path,
        entries,
        skipped: Vec::new(),
        looped: None,
        total_bytes: packed.total_bytes,
    })
}

/// A roster hero for a game codename: the roster's codenames and folder rules, then the
/// display name with separators removed (`abrams`, `mokrill` from Grimoire's sound codenames).
fn roster_for_codename(code: &str) -> Option<&'static RosterHero> {
    hero_for_codename(code).or_else(|| {
        let compact = compact_name(code);
        ROSTER.iter().find(|hero| compact_name(hero.name) == compact)
    })
}

fn compact_name(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// A requested hero: roster codenames when it is a roster hero, else the raw codename.
struct HeroMatch {
    roster: Option<&'static RosterHero>,
    raw: String,
}

impl HeroMatch {
    fn new(name: &str) -> Option<Self> {
        let raw = name.trim().to_ascii_lowercase();
        if raw.is_empty() {
            return None;
        }
        let roster = crate::locker::roster_hero(name).or_else(|| roster_for_codename(&raw));
        Some(Self { roster, raw })
    }

    fn matches(&self, codename: Option<&str>) -> bool {
        let Some(code) = codename else {
            return false;
        };
        if code.eq_ignore_ascii_case(&self.raw) {
            return true;
        }
        match (self.roster, roster_for_codename(code)) {
            (Some(wanted), Some(found)) => wanted.name == found.name,
            _ => false,
        }
    }
}

fn search_words(search: Option<&str>) -> Vec<String> {
    search
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect()
}

fn build_fingerprint(pak_path: &Path) -> Result<String> {
    let metadata = fs::metadata(pak_path).map_err(|error| {
        Error::msg(format!("{} cannot be read: {error}.", pak_path.display()))
    })?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    Ok(format!("{:x}-{modified:x}", metadata.len()))
}

/// Remove every child folder of `root` except `keep`. Returns the bytes freed.
fn prune_stale_fingerprints(root: &Path, keep: &str) -> u64 {
    let Ok(children) = fs::read_dir(root) else {
        return 0;
    };
    let mut freed = 0;
    for child in children.flatten() {
        if child.file_name().to_string_lossy() == keep {
            continue;
        }
        let path = child.path();
        let mut files = Vec::new();
        collect_files(&path, &mut files);
        let size: u64 = files.iter().map(|(_, size, _)| *size).sum();
        let removed = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if removed.is_ok() {
            freed += size;
        }
    }
    freed
}

fn collect_files(root: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(children) = fs::read_dir(root) else {
        if let Ok(metadata) = fs::metadata(root)
            && metadata.is_file()
        {
            out.push((
                root.to_path_buf(),
                metadata.len(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
            ));
        }
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        let Ok(metadata) = child.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&path, out);
        } else {
            out.push((path, metadata.len(), metadata.modified().unwrap_or(UNIX_EPOCH)));
        }
    }
}

#[cfg(test)]
#[path = "game_assets_tests.rs"]
mod tests;
