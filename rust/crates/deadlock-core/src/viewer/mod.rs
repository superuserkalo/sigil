//! Asset pipeline for the 3D viewer: hero previews and the Hideout scene, as files.
//!
//! The renderer (`sigil-viewer`) never reads a VPK. It opens what this module writes:
//!
//! - a hero preview: `<codename>.glb` (binary glTF 2.0, textured, skinned, animated) with a
//!   `<codename>.json` sidecar ([`HeroSidecar`]) beside it,
//! - the Hideout: a folder holding a scene file ([`HideoutScene`]) and the glbs it names.
//!
//! The file contract (fields, units, axes) is written down in `docs/design/3d-viewer.md`.
//! Everything emitted is in glTF space: Y up, meters, right handed. Source 2 data is Z up in
//! inches; [`space`] holds the one conversion.
//!
//! A preview reads the same files the game would. The VPK stack ([`sources`]) is the game's own
//! precedence: the Global priority folder, then enabled addon slots by load order (lower slot
//! wins), optionally plus disabled slots the player wants to preview before switching them on,
//! then `citadel/pak01_dir.vpk` and `core/pak01_dir.vpk`. Nothing here writes to the game
//! install: every VPK is opened read only, and output goes to the cache folder the caller
//! passes (the app uses `<data>/viewer-cache`).
//!
//! Decoding and glTF writing are vpkmerge's `morphic` crate (MIT), linked as a library. The
//! hero export, clip choice and Hideout scene build are ported from Grimoire (MIT):
//! `heroPoseModels.ts`, `heroAnimationCatalog.ts`, `heroRiggedClips.ts` (the UI AnimGraph2
//! reader), `heroAnimationMetadata.ts`, `heroViewerPlayback.ts` (clip labels and groups),
//! `hideoutScene.ts`, `hideoutAtmosphere.ts` and `source2Entity.ts`. The per-source usage
//! report and the one-decode-at-a-time slot follow Deadlock Mod Manager's Foundry preview
//! (behaviour only, no code).
//!
//! Every long call takes a cancel flag, checks it between stages and returns an error that
//! [`is_cancelled`] recognises. All of it is plain blocking code meant for a worker thread.

mod cache;
mod clips;
mod hero;
mod hideout;
mod kv;
mod particles;
pub mod space;
mod stack;
mod staged;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub use cache::{PruneReport, prune_cache};
pub use clips::{
    Action, ClipCandidate, ClipChoice, ClipGroup, ClipShowcase, Playback, Showcase, ShowcaseState,
    clip_group, clip_label, select_clips,
};
pub use hero::{
    ClipMeta, ClipOrigin, Exported, HeroEntry, HeroExportRequest, HeroSidecar, export_hero,
    export_hero_with, find_hero, heroes, heroes_in_vpk,
};
pub use hideout::{
    HideoutBloom, HideoutCamera, HideoutColorCorrection, HideoutExportRequest, HideoutFog,
    HideoutFogVolume, HideoutHero, HideoutLight, HideoutLightmap, HideoutModel, HideoutPost,
    HideoutProbe, HideoutScene, HideoutSky, HideoutSpawn, HideoutTonemap, HideoutVolume,
    export_hideout, export_hideout_with,
};
pub use stack::{Source, SourceKind, SourceUsage, name_sources, sources};

/// Bumped whenever the files this module writes change shape or content, so old cache entries
/// are never served to a newer renderer.
pub const PIPELINE_VERSION: &str = "sigil-viewer-3";

/// The vpkmerge revision `Cargo.toml` pins. Part of every cache key: a new exporter re-exports.
pub const VPKMERGE_REV: &str = "df4158673f4e700b9dda4db76ed9f193440b24ba";

/// The file contract version written as `schema` in every sidecar and scene file.
pub const SCHEMA: u32 = 1;

/// Which look a preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// The game as the player has it: enabled mods (plus any disabled slots asked for).
    Modded,
    /// The unmodded game: base VPKs only.
    Original,
}

impl Variant {
    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Modded => "modded",
            Variant::Original => "original",
        }
    }
}

const CANCELLED: &str = "Preview cancelled.";

/// True when `error` came from a cancel flag rather than a failure.
pub fn is_cancelled(error: &Error) -> bool {
    matches!(error, Error::Message(message) if message == CANCELLED)
        || matches!(error, Error::Cancelled)
}

pub(crate) fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::msg(CANCELLED))
    } else {
        Ok(())
    }
}

/// `game/citadel` under a game root.
pub(crate) fn citadel_dir(game_root: &std::path::Path) -> PathBuf {
    game_root.join("game").join("citadel")
}

/// A vpkmerge error as a domain error, with its context chain.
pub(crate) fn exporter_error(
    context: impl std::fmt::Display,
    error: impl std::fmt::Debug,
) -> Error {
    Error::msg(format!("{context}: {error:?}"))
}
