//! The game's own art and sounds in the Locker (parity wave 4, UI C): shared state, the
//! thumbnail loader and the bridge to `deadlock_core::game_assets`.
//!
//! - Grimoire: Foundry's texture catalog and sound browser, sound swap, and a custom in-game hero
//!   card, each built by its `vpkmerge` binary and installed straight away.
//! - Deadlock Mod Manager: a per-skin Foundry with Assets, Cards and Sounds tabs; changes leave
//!   through an export sheet as a new mod in its library.
//! - Sigil: two hero page sections (Game art, Game sounds) and an In-game card dialog. Every read
//!   runs on a worker against the game's own VPKs; nothing copies the game's paks. A swap or a
//!   card is packed into a new VPK in app data and lands in the import review, switched off, like
//!   every install.
//!
//! Memory: thumbnails are at most 256 px RGBA, decoded on one worker, at most [`THUMB_CAP`] are
//! held, and all of them are let go when the Game art section, the card dialog or the Locker
//! closes.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use rune::TextEdit;

use crate::app::State;
use crate::jobs::Job;
use crate::screens::settings::i18n::{fill, tr};

use super::{HeroSection, Page};

/// The most game thumbnails held at once.
pub const THUMB_CAP: usize = 96;
/// The key prefix of a game texture thumbnail in `State::thumbnails`.
pub const THUMB_PREFIX: &str = "game-tex:";

/// A read that has not run, is running, or finished.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            _ => None,
        }
    }
}

// --- the model the screens draw ---------------------------------------------------

/// What a game texture is, from its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexKind {
    /// Hero cards and portraits (`panorama/images/heroes`).
    Card,
    /// HUD ability icons.
    Ability,
    /// The hero model's textures: the skin targets.
    Skin,
    /// Ability effect textures.
    Effect,
    Other,
}

impl TexKind {
    pub const ALL: [TexKind; 5] = [TexKind::Card, TexKind::Ability, TexKind::Skin, TexKind::Effect, TexKind::Other];

    pub fn label(self) -> &'static str {
        match self {
            TexKind::Card => tr("Cards and portraits"),
            TexKind::Ability => tr("Ability icons"),
            TexKind::Skin => tr("Skin textures"),
            TexKind::Effect => tr("Ability effects"),
            TexKind::Other => tr("Other"),
        }
    }
}

/// One game texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Tex {
    pub path: String,
    pub label: String,
    pub kind: TexKind,
    /// Native size, when the catalog read the header.
    pub size: Option<(u32, u32)>,
}

/// Where a sound belongs on the hero page.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SoundGroup {
    /// An ability by slot (1 to 4) and name.
    Ability(u8, String),
    Weapon,
    Movement,
    Melee,
    Voice,
    Other,
}

impl SoundGroup {
    pub fn title(&self) -> String {
        match self {
            SoundGroup::Ability(_, name) if !name.is_empty() => name.clone(),
            SoundGroup::Ability(..) => tr("Ability").to_string(),
            SoundGroup::Weapon => tr("Weapon").to_string(),
            SoundGroup::Movement => tr("Movement").to_string(),
            SoundGroup::Melee => tr("Melee").to_string(),
            SoundGroup::Voice => tr("Voice lines").to_string(),
            SoundGroup::Other => tr("Other sounds").to_string(),
        }
    }

    /// The slot chip: 1 to 3, ULT for the fourth.
    pub fn chip(&self) -> Option<String> {
        match self {
            SoundGroup::Ability(4, _) => Some(tr("ULT").to_string()),
            SoundGroup::Ability(slot, _) if *slot > 0 => Some(slot.to_string()),
            _ => None,
        }
    }
}

/// One sound event: what the game plays, and the clips it picks from.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundRow {
    pub event: String,
    pub label: String,
    pub group: SoundGroup,
    /// The `.vsndevts_c` file that declares the event; `None` for voice lines, which are
    /// swapped clip by clip.
    pub soundevents: Option<String>,
    /// Compiled clip paths, in pool order.
    pub clips: Vec<String>,
    pub seconds: Option<f32>,
}

/// One in-game card variant the hero ships (card, vertical, low HP, ...).
#[derive(Clone, Debug, PartialEq)]
pub struct CardSlot {
    pub variant: String,
    /// The texture path the new art is packed at.
    pub path: String,
    pub size: (u32, u32),
}

/// One staged sound swap: the player's MP3 over every clip of an event, or with no file a
/// volume change of the game's own sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Swap {
    pub event: String,
    pub label: String,
    pub soundevents: Option<String>,
    pub clips: Vec<String>,
    pub file: Option<PathBuf>,
    /// `None` keeps the game clip's own loop flag.
    pub looped: Option<bool>,
    /// Volume change in decibels (the core applies it in steps of about 1.5 dB).
    pub gain_db: f64,
}

// --- state ------------------------------------------------------------------------

#[derive(Default)]
pub struct Assets {
    /// The hero codename the catalogs below were read for.
    pub hero: Option<String>,
    pub textures: Load<Vec<Tex>>,
    pub tex_job: Job<(String, Vec<Tex>)>,
    pub tex_kind: Option<TexKind>,
    pub tex_query: TextEdit,
    /// How many matches the grid shows; Show more adds a page.
    pub tex_shown: usize,
    /// The texture open in the lightbox.
    pub lightbox: Option<String>,
    pub save_job: Job<String>,

    pub sounds: Load<Vec<SoundRow>>,
    pub sound_job: Job<(String, Vec<SoundRow>)>,
    pub sound_query: TextEdit,
    /// Groups folded shut, by title.
    pub folded: HashSet<String>,
    pub clip_job: Job<(u64, Vec<u8>)>,
    pub clip_save_job: Job<String>,
    pub swaps: Vec<Swap>,
    pub swap_name: TextEdit,
    pub swap_job: Job<Vec<PathBuf>>,
    /// The shared-clip check before a build: which other heroes play a swap's clips.
    pub share_job: Job<Shared>,
    /// Its last answer, with the swaps it ran for (`game_sounds::signature`).
    pub shared: Option<Shared>,
    pub swap_error: Option<String>,

    pub card: Option<super::card_art::CardArt>,

    pub thumbs: Thumbs,
    /// The game's pak is open for an asset page (dropped when the last one closes).
    pub pak_open: bool,
}

/// The thumbnail worker: one thread, a queue, RGBA out.
#[derive(Default)]
pub struct Thumbs {
    tx: Option<Sender<String>>,
    rx: Option<Receiver<(String, Result<Preview, String>)>>,
    /// Asked for and not back yet.
    pub inflight: HashSet<String>,
    /// Held keys, oldest first.
    pub held: VecDeque<String>,
    /// Paths whose decode failed: shown as a quiet box, never asked again this session.
    pub failed: HashSet<String>,
    /// Native texture sizes, from the previews read so far.
    pub sizes: std::collections::HashMap<String, (u32, u32)>,
}

/// One decoded preview: width, height, RGBA, and the texture's native size.
pub type Preview = (u32, u32, Vec<u8>, (u32, u32));

/// The shared-clip check: the swaps it ran for, and per event the other heroes that play it.
pub type Shared = (Vec<String>, Vec<(String, Vec<String>)>);

impl Thumbs {
    fn worker(&mut self, game_root: PathBuf, wake: Option<rune::Waker>) -> Option<&Sender<String>> {
        if self.tx.is_none() {
            let (tx, paths) = mpsc::channel::<String>();
            let (out, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("sigil-game-thumbs".to_string())
                .spawn(move || {
                    let cache = api::thumb_cache_dir();
                    while let Ok(path) = paths.recv() {
                        let result = api::thumbnail(&game_root, &cache, &path);
                        if out.send((path, result)).is_err() {
                            break;
                        }
                        if let Some(wake) = wake.as_ref() {
                            wake();
                        }
                    }
                });
            if spawned.is_err() {
                return None;
            }
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        self.tx.as_ref()
    }

    fn stop(&mut self) {
        // Dropping the sender ends the worker after its current decode.
        self.tx = None;
        self.rx = None;
        self.inflight.clear();
    }
}

pub fn thumb_key(path: &str) -> String {
    format!("{THUMB_PREFIX}{path}")
}

/// Ask for the thumbnails of these texture paths, skipping held, queued and failed ones.
pub fn want_thumbs(st: &mut State, paths: &[String]) {
    if st.demo || rune::is_headless() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    let wake = st.wake.clone();
    let fresh: Vec<String> = paths
        .iter()
        .filter(|path| {
            !st.thumbnails.contains_key(&thumb_key(path))
                && !st.locker.assets.thumbs.inflight.contains(*path)
                && !st.locker.assets.thumbs.failed.contains(*path)
        })
        .cloned()
        .collect();
    if fresh.is_empty() {
        return;
    }
    let thumbs = &mut st.locker.assets.thumbs;
    let Some(tx) = thumbs.worker(game_root, wake).cloned() else { return };
    for path in fresh {
        if tx.send(path.clone()).is_ok() {
            thumbs.inflight.insert(path);
        }
    }
}

/// Take decoded thumbnails, and keep the held set under the cap: the oldest that are not in
/// `keep` go first.
fn drain_thumbs(st: &mut State, keep: &HashSet<String>) -> bool {
    let mut changed = false;
    loop {
        let next = match st.locker.assets.thumbs.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(item)) => item,
            Some(Err(TryRecvError::Disconnected)) => {
                st.locker.assets.thumbs.stop();
                break;
            }
            _ => break,
        };
        let (path, result) = next;
        st.locker.assets.thumbs.inflight.remove(&path);
        match result {
            Ok((w, h, rgba, native)) if w > 0 && h > 0 && rgba.len() == (w * h * 4) as usize => {
                st.locker.assets.thumbs.sizes.insert(path.clone(), native);
                if let Some(cache) = st.images.as_ref() {
                    let key = thumb_key(&path);
                    let image = cache.rgba(w, h, rgba);
                    st.thumbnails.insert(key.clone(), image.pixels().clone());
                    st.locker.assets.thumbs.held.push_back(key);
                }
            }
            _ => {
                st.locker.assets.thumbs.failed.insert(path);
            }
        }
        changed = true;
    }
    let thumbs = &mut st.locker.assets.thumbs;
    let mut spins = thumbs.held.len();
    while thumbs.held.len() > THUMB_CAP && spins > 0 {
        spins -= 1;
        let Some(key) = thumbs.held.pop_front() else { break };
        if keep.contains(&key) {
            thumbs.held.push_back(key);
        } else {
            st.thumbnails.remove(&key);
        }
    }
    changed
}

/// Let go of every game thumbnail and stop the worker.
pub fn release_thumbs(st: &mut State) {
    let held: Vec<String> = st.locker.assets.thumbs.held.drain(..).collect();
    for key in held {
        st.thumbnails.remove(&key);
    }
    st.locker.assets.thumbs.stop();
}

/// Whether anything on screen draws game thumbnails.
fn thumbs_in_use(st: &State) -> bool {
    st.screen == crate::model::Screen::Locker
        && (st.locker.assets.card.is_some()
            || (matches!(st.locker.page, Page::Hero(_)) && st.locker.hero_section == HeroSection::GameArt))
}

/// Whether any asset page is open (the pak stays open while one is).
fn pak_in_use(st: &State) -> bool {
    thumbs_in_use(st)
        || (st.screen == crate::model::Screen::Locker
            && matches!(st.locker.page, Page::Hero(_))
            && st.locker.hero_section == HeroSection::GameSounds)
}

/// Forget the catalogs when the hero changes.
pub fn for_hero(st: &mut State, codename: &str) {
    let a = &mut st.locker.assets;
    if a.hero.as_deref() == Some(codename) {
        return;
    }
    a.hero = Some(codename.to_string());
    a.textures = Load::Idle;
    a.sounds = Load::Idle;
    a.tex_shown = super::catalog::PAGE;
    a.lightbox = None;
    a.swaps.clear();
    a.shared = None;
    a.swap_error = None;
    a.folded.clear();
    a.swap_name = TextEdit::new("");
}

/// Start the catalog read the open section needs.
pub fn ensure_loaded(st: &mut State) {
    let Page::Hero(name) = st.locker.page.clone() else { return };
    let Some(hero) = st.locker.heroes.iter().find(|h| h.display_name == name).cloned() else { return };
    for_hero(st, &hero.codename);
    if st.demo || rune::is_headless() {
        return;
    }
    let section = st.locker.hero_section;
    let Some(game_root) = st.game_root.clone() else {
        let message = tr("Set the game folder in Settings first.").to_string();
        match section {
            HeroSection::GameArt => st.locker.assets.textures = Load::Failed(message),
            HeroSection::GameSounds => st.locker.assets.sounds = Load::Failed(message),
            _ => {}
        }
        return;
    };
    let wake = st.wake.clone();
    let a = &mut st.locker.assets;
    match section {
        HeroSection::GameArt if a.textures == Load::Idle => {
            a.textures = Load::Loading;
            let code = hero.codename.clone();
            a.tex_job.start(wake, move || api::textures(&game_root, &code).map(|list| (code, list)));
        }
        HeroSection::GameSounds if a.sounds == Load::Idle => {
            a.sounds = Load::Loading;
            let code = hero.codename.clone();
            a.sound_job.start(wake, move || api::sounds(&game_root, &code).map(|list| (code, list)));
        }
        _ => {}
    }
}

/// A stable id for the shared player, from a clip or file path.
pub fn play_id(path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "sigil-game-clip".hash(&mut hasher);
    path.hash(&mut hasher);
    hasher.finish() | (1 << 63)
}

/// Play a game clip through the shared preview player, or stop it.
pub fn play_clip(st: &mut State, clip: &str) {
    let id = play_id(clip);
    let audio = &mut st.browse_view.audio;
    if audio.playing == Some(id) || audio.loading == Some(id) {
        audio.stop();
        return;
    }
    if st.locker.assets.clip_job.is_running() || st.demo || rune::is_headless() {
        return;
    }
    let Some(game_root) = st.game_root.clone() else { return };
    audio.stop();
    audio.loading = Some(id);
    let clip = clip.to_string();
    st.locker.assets.pak_open = true;
    st.locker.assets.clip_job.start(st.wake.clone(), move || api::clip_audio(&game_root, &clip).map(|(bytes, _)| (id, bytes)));
}

/// Save a game clip as MP3 or WAV where the player picks (Grimoire's Save preview as MP3).
pub fn save_clip(st: &mut State, tx: &mut rune::Tx, clip: String, label: String) {
    let Some(game_root) = st.game_root.clone() else { return };
    let options = rune::FileDialogOptions {
        title: tr("Save the sound").to_string(),
        file_name: Some(format!("{}.mp3", api::base_name(&label))),
        ..rune::FileDialogOptions::default()
    }
    .filter("Audio", &["mp3", "wav"]);
    tx.save_file(options, move |st: &mut State, result, _tx| {
        let Ok(Some(out)) = result else { return };
        let (game_root, clip) = (game_root.clone(), clip.clone());
        st.locker.assets.clip_save_job.start(st.wake.clone(), move || {
            let (bytes, extension) = api::clip_audio(&game_root, &clip)?;
            let out = out.with_extension(extension);
            std::fs::write(&out, bytes).map_err(|error| error.to_string())?;
            Ok(out.display().to_string())
        });
    });
}

/// Play the player's own file (a staged swap) through the same player.
pub fn play_file(st: &mut State, file: &Path) {
    let id = play_id(&file.display().to_string());
    if st.browse_view.audio.playing == Some(id) {
        st.browse_view.audio.stop();
        return;
    }
    match std::fs::read(file) {
        Ok(bytes) => st.browse_view.audio.toggle_bytes(id, bytes),
        Err(_) => st.status = tr("The file could not be read.").to_string(),
    }
}

/// Take worker results. Runs every frame from the app poll.
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if let Some(result) = st.locker.assets.tex_job.try_take() {
        st.locker.assets.textures = match result {
            Ok((code, list)) if st.locker.assets.hero.as_deref() == Some(&code) => Load::Ready(list),
            Ok(_) => Load::Idle,
            Err(error) => Load::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = st.locker.assets.sound_job.try_take() {
        st.locker.assets.sounds = match result {
            Ok((code, list)) if st.locker.assets.hero.as_deref() == Some(&code) => Load::Ready(list),
            Ok(_) => Load::Idle,
            Err(error) => Load::Failed(error),
        };
        changed = true;
    }
    if let Some(result) = st.locker.assets.clip_job.try_take() {
        match result {
            Ok((id, bytes)) => {
                st.browse_view.audio.loading = None;
                st.browse_view.audio.toggle_bytes(id, bytes);
            }
            Err(error) => {
                st.browse_view.audio.loading = None;
                st.browse_view.audio.error = Some(error);
            }
        }
        changed = true;
    }
    if let Some(result) = st.locker.assets.save_job.try_take() {
        match result {
            Ok(path) => crate::notify::push(st, crate::notify::Notice::success(tr("Preview saved")).detail(path)),
            Err(error) => crate::notify::push(st, crate::notify::Notice::error(tr("The preview could not be saved")).detail(error)),
        }
        changed = true;
    }
    if let Some(result) = st.locker.assets.clip_save_job.try_take() {
        match result {
            Ok(path) => crate::notify::push(st, crate::notify::Notice::success(tr("Sound saved")).detail(path)),
            Err(error) => crate::notify::push(st, crate::notify::Notice::error(tr("The sound could not be saved")).detail(error)),
        }
        changed = true;
    }
    changed |= super::game_sounds::poll(st);
    changed |= super::card_art::poll(st);

    if thumbs_in_use(st) {
        if st.locker.page != Page::Grid {
            ensure_loaded(st);
        }
        let shown: Vec<String> = super::catalog::shown_paths(st).into_iter().chain(super::card_art::shown_paths(st)).collect();
        want_thumbs(st, &shown);
        let keep: HashSet<String> = shown.iter().map(|path| thumb_key(path)).collect();
        changed |= drain_thumbs(st, &keep);
    } else if !st.locker.assets.thumbs.held.is_empty() || st.locker.assets.thumbs.tx.is_some() {
        release_thumbs(st);
        changed = true;
    }
    if pak_in_use(st) {
        st.locker.assets.pak_open = true;
        if st.locker.hero_section == HeroSection::GameSounds && st.locker.assets.card.is_none() {
            ensure_loaded(st);
        }
    } else if st.locker.assets.pak_open {
        st.locker.assets.pak_open = false;
        api::release();
    }
    changed
}

// --- the core bridge ----------------------------------------------------------------

/// The calls into `deadlock_core::game_assets`, mapped into the model above. Kept in one place
/// so the screens never see the core types. The game's pak directory is opened once and shared
/// by every worker while an asset page is open; `release` drops it.
pub mod api {
    use std::sync::{Arc, Mutex};

    use deadlock_core::game_assets::{
        GameAssets, GameSound, LoopMode, SoundKind, SoundQuery, SoundSwapRequest, SoundSwapTarget,
        TextureKind, TextureQuery, TextureUpload, DEFAULT_ASSET_CACHE_BYTES,
    };

    use super::*;

    /// The longest side of a preview, in pixels: sharp on a 2x screen at the grid's tile size,
    /// about 150 KB decoded.
    pub const THUMB_EDGE: u32 = 192;

    static PAK: Mutex<Option<(PathBuf, Arc<GameAssets>)>> = Mutex::new(None);

    /// The game's pak, opened on first use (a directory read on the calling worker).
    fn pak(game_root: &Path) -> Result<Arc<GameAssets>, String> {
        let mut slot = PAK.lock().map_err(|_| tr("The game files could not be opened.").to_string())?;
        if let Some((root, pak)) = slot.as_ref()
            && root == game_root
        {
            return Ok(pak.clone());
        }
        let pak = Arc::new(GameAssets::open(game_root).map_err(|error| error.to_string())?);
        *slot = Some((game_root.to_path_buf(), pak.clone()));
        Ok(pak)
    }

    /// Drop the pak directory and trim the preview cache on a thread. Runs when the last asset
    /// page closes; a running worker keeps its own handle until it finishes.
    pub fn release() {
        let taken = PAK.lock().ok().and_then(|mut slot| slot.take());
        if let Some((_, pak)) = taken {
            let _ = std::thread::Builder::new().name("sigil-asset-prune".to_string()).spawn(move || {
                let _ = pak.prune_cache(&cache_dir(), DEFAULT_ASSET_CACHE_BYTES);
            });
        }
    }

    /// The previews and sound index cache (core layout under it), in app data.
    pub fn cache_dir() -> PathBuf {
        crate::data::data_dir().join("asset-cache")
    }

    pub fn thumb_cache_dir() -> PathBuf {
        cache_dir()
    }

    /// Where built swap and card mods are written before the import review copies them: one
    /// fresh folder per build, so the file keeps a readable name.
    pub fn out_dir() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        crate::data::data_dir().join("game-asset-mods").join(stamp.to_string())
    }

    /// A file stem from a mod name: no path or reserved characters.
    pub fn base_name(name: &str) -> String {
        let clean: String = name
            .chars()
            .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { ' ' } else { c })
            .collect();
        let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() { "sigil-mod".to_string() } else { clean.chars().take(80).collect() }
    }

    pub fn tex_kind(kind: TextureKind) -> TexKind {
        match kind {
            TextureKind::HeroCard | TextureKind::HeroPortrait => TexKind::Card,
            TextureKind::AbilityIcon => TexKind::Ability,
            TextureKind::SkinTexture => TexKind::Skin,
            TextureKind::AbilityVfx => TexKind::Effect,
            TextureKind::ItemIcon | TextureKind::Other => TexKind::Other,
        }
    }

    pub fn textures(game_root: &Path, hero: &str) -> Result<Vec<Tex>, String> {
        let pak = pak(game_root)?;
        let query = TextureQuery { hero: Some(hero.to_string()), ..TextureQuery::default() };
        Ok(pak
            .texture_catalog(&query)
            .into_iter()
            .map(|t| Tex { label: t.label, kind: tex_kind(t.kind), path: t.path, size: None })
            .collect())
    }

    /// A preview: (width, height, RGBA, native size).
    pub fn thumbnail(game_root: &Path, cache: &Path, path: &str) -> Result<Preview, String> {
        let pak = pak(game_root)?;
        let thumb = pak.texture_thumbnail(path, THUMB_EDGE, cache).map_err(|error| error.to_string())?;
        Ok((thumb.width, thumb.height, thumb.rgba, (thumb.source_width, thumb.source_height)))
    }

    /// The preview as a PNG (Save preview).
    pub fn preview_png(game_root: &Path, path: &str) -> Result<Vec<u8>, String> {
        let pak = pak(game_root)?;
        let thumb = pak
            .texture_thumbnail(path, deadlock_core::game_assets::THUMBNAIL_MAX_EDGE, &cache_dir())
            .map_err(|error| error.to_string())?;
        let image = image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba)
            .ok_or_else(|| tr("The texture could not be read.").to_string())?;
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).map_err(|error| error.to_string())?;
        Ok(out.into_inner())
    }

    pub fn sound_group(sound: &GameSound) -> SoundGroup {
        match sound.kind {
            SoundKind::Ability => SoundGroup::Ability(sound.slot.unwrap_or(0), sound.ability.clone().unwrap_or_default()),
            SoundKind::Weapon => SoundGroup::Weapon,
            SoundKind::Movement => SoundGroup::Movement,
            SoundKind::Melee => SoundGroup::Melee,
            SoundKind::Voice => SoundGroup::Voice,
            SoundKind::Other => SoundGroup::Other,
        }
    }

    pub fn sounds(game_root: &Path, hero: &str) -> Result<Vec<SoundRow>, String> {
        let pak = pak(game_root)?;
        let query = SoundQuery { hero: hero.to_string(), ..SoundQuery::default() };
        let list = pak.sound_catalog(&cache_dir(), &query).map_err(|error| error.to_string())?;
        Ok(list
            .into_iter()
            .map(|sound| SoundRow {
                group: sound_group(&sound),
                label: sound.caption.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| sound.label.clone()),
                event: sound.event,
                soundevents: sound.soundevents,
                clips: sound.clips,
                seconds: sound.duration.map(|d| d as f32),
            })
            .collect())
    }

    /// A clip as playable bytes, and its file extension (`mp3` or `wav`).
    pub fn clip_audio(game_root: &Path, clip: &str) -> Result<(Vec<u8>, &'static str), String> {
        let pak = pak(game_root)?;
        let audio = pak.clip_audio(clip).map_err(|error| error.to_string())?;
        Ok((audio.bytes, audio.extension))
    }

    pub fn card_slots(game_root: &Path, hero: &str) -> Result<Vec<CardSlot>, String> {
        let pak = pak(game_root)?;
        let slots = pak.hero_card_slots(hero).map_err(|error| error.to_string())?;
        Ok(slots
            .into_iter()
            .map(|slot| CardSlot { variant: variant_label(&slot.variant), path: slot.entry, size: (slot.width, slot.height) })
            .collect())
    }

    /// The player-facing name of a card variant (Grimoire's labels).
    pub fn variant_label(variant: &str) -> String {
        let (wolf, rest) = match variant.strip_prefix("wolf_") {
            Some(rest) => (true, rest),
            None => (false, variant),
        };
        let base = match rest {
            "card" => tr("Card").to_string(),
            "vertical" => tr("Vertical").to_string(),
            "card_critical" => tr("Low HP").to_string(),
            "card_gloat" => tr("Gloat").to_string(),
            "minimap" => tr("Minimap").to_string(),
            "small" | "sm" => tr("Small").to_string(),
            "background" => tr("Background").to_string(),
            "weapon" => tr("Weapon").to_string(),
            other => {
                let spaced = other.replace('_', " ");
                let mut chars = spaced.chars();
                chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
            }
        };
        if wolf { fill(tr("Wolf {}"), &[&base]) } else { base }
    }

    fn loop_mode(looped: Option<bool>) -> LoopMode {
        match looped {
            None => LoopMode::Auto,
            Some(true) => LoopMode::On,
            Some(false) => LoopMode::Off,
        }
    }

    /// Build each staged swap into its own mod VPK (the core mints one target per mod). One
    /// swap takes the typed name; several are named after their sound.
    pub fn build_swaps(game_root: &Path, name: &str, swaps: &[Swap]) -> Result<Vec<PathBuf>, String> {
        let pak = pak(game_root)?;
        let mut out = Vec::new();
        for swap in swaps {
            let audio = match &swap.file {
                Some(file) => Some(std::fs::read(file).map_err(|_| tr("The file could not be read.").to_string())?),
                None => None,
            };
            let target = match &swap.soundevents {
                Some(soundevents) => SoundSwapTarget::Event { soundevents: soundevents.clone(), event: swap.event.clone() },
                None => SoundSwapTarget::Clips { clips: swap.clips.clone() },
            };
            let request = SoundSwapRequest {
                target: Some(target),
                audio,
                audio_name: swap
                    .file
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
                loop_mode: loop_mode(swap.looped),
                trim_ms: None,
                gain_db: (swap.gain_db != 0.0).then_some(swap.gain_db),
            };
            let stem = if swaps.len() == 1 { base_name(name) } else { base_name(&format!("{name} ({})", swap.label)) };
            let built = pak.build_sound_swap(&request, &out_dir(), &stem).map_err(|error| error.to_string())?;
            out.push(built.vpk_path);
        }
        Ok(out)
    }

    /// A clip path in one form for comparing: lowercase, `/`, without the compiled `_c`.
    pub fn clip_key(clip: &str) -> String {
        let lower = clip.to_ascii_lowercase().replace('\\', "/");
        lower.strip_suffix("_c").map(str::to_string).unwrap_or(lower)
    }

    /// A clip under a folder the game shares between heroes.
    pub fn shared_folder(clip: &str) -> bool {
        let key = clip_key(clip);
        key.starts_with("sounds/player/") || key.contains("/shared/") || key.contains("/common/")
    }

    /// For each swap, the other heroes whose gameplay sounds play any of its clips. A clip in a
    /// shared folder with no other hero found still warns (an empty hero list). Voice lines are
    /// left out of the scan: they are the hero's own.
    pub fn shared_heroes(game_root: &Path, hero: &str, swaps: &[Swap]) -> Result<Vec<(String, Vec<String>)>, String> {
        let pak = pak(game_root)?;
        let cache = cache_dir();
        let wanted: Vec<(String, HashSet<String>)> =
            swaps.iter().map(|w| (w.event.clone(), w.clips.iter().map(|c| clip_key(c)).collect())).collect();
        let mut found: Vec<(String, Vec<String>)> = swaps.iter().map(|w| (w.event.clone(), Vec::new())).collect();
        let heroes = pak.sound_heroes(&cache).map_err(|error| error.to_string())?;
        let me = hero.to_ascii_lowercase();
        for other in heroes {
            if other.name.to_ascii_lowercase() == me || other.key.to_ascii_lowercase() == me {
                continue;
            }
            let query = SoundQuery {
                hero: other.key.clone(),
                kinds: vec![SoundKind::Ability, SoundKind::Weapon, SoundKind::Movement, SoundKind::Melee, SoundKind::Other],
                ..SoundQuery::default()
            };
            let Ok(list) = pak.sound_catalog(&cache, &query) else { continue };
            for (n, (_, clips)) in wanted.iter().enumerate() {
                let hit = list.iter().any(|sound| sound.clips.iter().any(|c| clips.contains(&clip_key(c))));
                if hit && !found[n].1.contains(&other.name) {
                    found[n].1.push(other.name.clone());
                }
            }
        }
        Ok(found
            .into_iter()
            .zip(swaps)
            .filter(|((_, heroes), swap)| !heroes.is_empty() || swap.clips.iter().any(|c| shared_folder(c)))
            .map(|(entry, _)| entry)
            .collect())
    }

    /// Pack the chosen art (template path and PNG bytes) into one mod VPK in app data.
    pub fn build_card(game_root: &Path, name: &str, picks: Vec<(String, Vec<u8>)>) -> Result<PathBuf, String> {
        let pak = pak(game_root)?;
        let uploads: Vec<TextureUpload> = picks.into_iter().map(|(entry, png)| TextureUpload { entry, png }).collect();
        pak.build_hero_card_mod(&uploads, &out_dir(), &base_name(name))
            .map(|built| built.vpk_path)
            .map_err(|error| error.to_string())
    }
}
