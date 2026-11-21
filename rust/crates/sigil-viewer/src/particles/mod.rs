//! Source 2 particles for the viewer: hero ambient and animation-event effects and Hideout
//! map particles.
//!
//! A faithful subset of Grimoire's particle system (MIT), itself a port of
//! ValveResourceFormat's renderer (MIT): the CPU simulation (emitters, initializers,
//! operators, force generators, pre-emission operators, children, control points, skinned
//! snapshots, VRF's seeding and operator strength), and `C_OP_RenderSprites`,
//! `C_OP_RenderTrails` and `C_OP_RenderRopes` drawn as instanced spritecards (ropes as strip
//! quads) in the scene's HDR pass. Everything is simulated in Source space
//! (inches, Z up) and placed into the world when the cards are built. Classes and inputs this
//! port lacks are skipped and listed in [`ParticleGpu::diagnostics`], never guessed.
//!
//! The bundle format is deadlock-core's `particles/bundle.json` (schema 1).

mod bundle;
mod fields;
pub mod fixture;
mod functions;
mod gpu;
mod host;
mod kv;
mod providers;
mod render;
mod smath;
mod snapshot;
mod state;
mod system;
#[cfg(test)]
mod tests;

pub use bundle::{Attachment, Bundle, ClipStop, Effect, EffectAttach, EffectClip, EffectCp, Influence, Sequence, Sheet, SheetFrame, SystemDef, Texture};
pub use gpu::{ParticleGpu, VIEWER_SUN};
pub use host::{Effects, SYSTEM_CAP, TOTAL_CAP};
pub use render::{Instance, View};
pub use smath::string_token;

/// The Source to glTF placement of a bone matrix: `source_to_gltf()^-1 * gltf_model_space`,
/// for turning the glb's posed node matrices (under its `skeleton` node) into the Source
/// model-space bones [`Bundle::attachment_frame`] reads.
pub fn source_bone(gltf_model_space: &crate::math::Mat4) -> crate::math::Mat4 {
    crate::fixture::source_to_gltf().inverse().mul(gltf_model_space)
}
