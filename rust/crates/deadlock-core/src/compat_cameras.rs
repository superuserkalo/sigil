//! The camera repair's keep rule (Deadlock Mod Manager PR #1399): when a model is repaired
//! because some of its `root_motion` attachments use the old frame, the attachments that were
//! already in the current frame keep the author's values.
//!
//! vpkmerge's [`AttachmentHealer`](vpkmerge_core::AttachmentHealer) writes the game's offset and
//! rotation over every attachment it shares with the game once any one of them is stale. An
//! author who moved `gunaim_00` a little, in the current frame, lost that tweak to the repair.
//! [`keep_current`] runs on the healed model and puts those attachments back as the mod shipped
//! them: same name, parented to `root_motion` on both sides, and a first rotation within
//! [`KEEP_DEGREES`] of the game's. Stale ones, restored camera inputs and attachments the game
//! does not have stay as vpkmerge wrote them.
//!
//! The attachment reader and the rotation gap follow vpkmerge's `attachment_heal` (MIT, see
//! `THIRD_PARTY_NOTICES.md`), so "current" means the same thing on both sides.

use std::collections::HashMap;

use morphic::kv3::{Seg, Value};
use morphic::resource::Resource;

use crate::error::{Error, Result};

const ROOT_BONE: &str = "root_motion";

/// Rotation gap from the game's value up to which an attachment counts as current: the same
/// line vpkmerge draws for stale ones.
pub const KEEP_DEGREES: f64 = vpkmerge_core::attachment_heal::STALE_DEGREES;

/// One attachment of an `MDAT` block.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// Position in the block's `m_attachments` array.
    pub index: usize,
    pub name: String,
    pub parent: String,
    pub offsets: Vec<Vec<f64>>,
    pub rotations: Vec<Vec<f64>>,
}

fn decode_error(error: morphic::DecodeError) -> Error {
    Error::msg(format!("The model could not be read: {error}"))
}

/// Angle in degrees between two `[x, y, z, w]` quaternions, sign-insensitive.
pub fn rotation_gap_degrees(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() != 4 || b.len() != 4 {
        return None;
    }
    let norm = |q: &[f64]| q.iter().map(|c| c * c).sum::<f64>().sqrt();
    let (na, nb) = (norm(a), norm(b));
    if na < 1e-9 || nb < 1e-9 {
        return None;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>() / (na * nb);
    Some(2.0 * dot.abs().min(1.0).acos().to_degrees())
}

/// Whether the mod's attachment is already in the game's current frame.
pub fn is_current(ours: &Attachment, game: &HashMap<String, Attachment>) -> bool {
    if ours.parent != ROOT_BONE {
        return false;
    }
    let Some(theirs) = game.get(&ours.name).filter(|g| g.parent == ROOT_BONE) else {
        return false;
    };
    match (ours.rotations.first(), theirs.rotations.first()) {
        (Some(a), Some(b)) => rotation_gap_degrees(a, b).is_some_and(|gap| gap <= KEEP_DEGREES),
        _ => false,
    }
}

/// The writes that put `original`'s offsets and rotations back onto the healed attachment at
/// `healed.index`. Empty when the healed one already holds them or the two differ in name.
pub fn restore_edits(original: &Attachment, healed: &Attachment) -> Vec<(Vec<Seg>, f64)> {
    if original.name != healed.name || (original.offsets == healed.offsets && original.rotations == healed.rotations) {
        return Vec::new();
    }
    let mut edits = Vec::new();
    for (field, ours, now) in [
        ("m_vInfluenceOffsets", &original.offsets, &healed.offsets),
        ("m_vInfluenceRotations", &original.rotations, &healed.rotations),
    ] {
        for (j, (want, have)) in ours.iter().zip(now).enumerate() {
            for (c, value) in want.iter().enumerate().take(have.len()) {
                edits.push((
                    vec![
                        Seg::Key("m_attachments".into()),
                        Seg::Index(healed.index),
                        Seg::Key("value".into()),
                        Seg::Key(field.into()),
                        Seg::Index(j),
                        Seg::Index(c),
                    ],
                    *value,
                ));
            }
        }
    }
    edits
}

/// A mesh block's attachments, or `None` when it has no `m_attachments`. Entries missing a
/// name, parent or transform are skipped.
pub fn attachments(mdat: &Value) -> Option<Vec<Attachment>> {
    let list = mdat.get("m_attachments")?.as_array()?;
    Some(
        list.iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let value = item.get("value")?;
                Some(Attachment {
                    index,
                    name: item.get("key")?.as_str()?.to_string(),
                    parent: value.get("m_influenceNames")?.as_array()?.first()?.as_str()?.to_string(),
                    offsets: vectors(value.get("m_vInfluenceOffsets")?)?,
                    rotations: vectors(value.get("m_vInfluenceRotations")?)?,
                })
            })
            .collect(),
    )
}

fn vectors(v: &Value) -> Option<Vec<Vec<f64>>> {
    v.as_array()?.iter().map(|vec| vec.as_array()?.iter().map(Value::as_f64).collect()).collect()
}

fn mdat_indices(resource: &Resource<'_>) -> Vec<usize> {
    resource
        .blocks()
        .iter()
        .enumerate()
        .filter(|(_, block)| &block.kind == b"MDAT")
        .map(|(index, _)| index)
        .collect()
}

/// The game model's `root_motion` attachments, from the first `MDAT` block that has any.
fn game_attachments(game: &[u8]) -> Result<HashMap<String, Attachment>> {
    let resource = Resource::parse(game).map_err(decode_error)?;
    for index in mdat_indices(&resource) {
        let raw = resource.get_block_by_index(index).ok_or_else(|| Error::msg("MDAT out of range"))?;
        if let Some(list) = attachments(&morphic::kv3::decode(raw).map_err(decode_error)?) {
            return Ok(list.into_iter().filter(|a| a.parent == ROOT_BONE).map(|a| (a.name.clone(), a)).collect());
        }
    }
    Ok(HashMap::new())
}

/// Put back, in `healed`, every attachment `original` already had in the current frame.
/// Returns the new bytes and the kept names, or `None` when nothing needed keeping.
pub fn keep_current(original: &[u8], healed: &[u8], game: &[u8]) -> Result<Option<(Vec<u8>, Vec<String>)>> {
    let game = game_attachments(game)?;
    if game.is_empty() {
        return Ok(None);
    }
    let source = Resource::parse(original).map_err(decode_error)?;
    let mut out = healed.to_vec();
    let mut kept: Vec<String> = Vec::new();
    for index in mdat_indices(&source) {
        let Some(raw) = source.get_block_by_index(index) else { continue };
        let Some(ours) = attachments(&morphic::kv3::decode(raw).map_err(decode_error)?) else { continue };
        let current: Vec<&Attachment> = ours.iter().filter(|a| is_current(a, &game)).collect();
        if current.is_empty() {
            continue;
        }
        let resource = Resource::parse(&out).map_err(decode_error)?;
        let Some(healed_raw) = resource.get_block_by_index(index) else { continue };
        let Some(now) = attachments(&morphic::kv3::decode(healed_raw).map_err(decode_error)?) else { continue };
        let mut edits = Vec::new();
        let mut names = Vec::new();
        for original in current {
            let Some(healed) = now.get(original.index) else { continue };
            let restore = restore_edits(original, healed);
            if !restore.is_empty() {
                edits.extend(restore);
                names.push(original.name.clone());
            }
        }
        if edits.is_empty() {
            continue;
        }
        let patched = morphic::kv3::set_reals(healed_raw, &edits).map_err(decode_error)?;
        out = resource.rebuild_with_block(index, &patched).map_err(decode_error)?;
        for name in names {
            if !kept.contains(&name) {
                kept.push(name);
            }
        }
    }
    Ok((!kept.is_empty()).then_some((out, kept)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_FRAME: [f64; 4] = [-0.5, -0.5, -0.5, 0.5];
    const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
    // Ten degrees of yaw: an author's tweak in the current frame.
    const TWEAK: [f64; 4] = [0.0, 0.0, 0.087_156, 0.996_195];

    fn att(index: usize, name: &str, parent: &str, off: [f64; 3], rot: [f64; 4]) -> Attachment {
        Attachment { index, name: name.into(), parent: parent.into(), offsets: vec![off.to_vec()], rotations: vec![rot.to_vec()] }
    }

    fn game() -> HashMap<String, Attachment> {
        [
            att(0, "standing_pivot", ROOT_BONE, [0.0, 0.0, 94.0], IDENTITY),
            att(1, "gunaim_00", ROOT_BONE, [43.0, 21.0, 32.0], IDENTITY),
            att(2, "palm_R", "hand_R", [1.0, 0.0, 0.0], IDENTITY),
        ]
        .into_iter()
        .map(|a| (a.name.clone(), a))
        .collect()
    }

    #[test]
    fn an_old_pivot_is_moved_but_a_current_tweak_is_kept() {
        let game = game();
        let pivot = att(0, "standing_pivot", ROOT_BONE, [0.0, 94.0, 0.0], OLD_FRAME);
        let gunaim = att(1, "gunaim_00", ROOT_BONE, [21.0, 32.0, 43.0], TWEAK);
        assert!(!is_current(&pivot, &game), "the old frame is 120 degrees off");
        assert!(is_current(&gunaim, &game), "ten degrees is the author's tweak");
    }

    #[test]
    fn other_parents_and_attachments_the_game_lacks_are_not_kept() {
        let game = game();
        assert!(!is_current(&att(2, "palm_R", "hand_R", [1.0, 0.0, 0.0], IDENTITY), &game));
        assert!(!is_current(&att(3, "extra_pose", ROOT_BONE, [0.0, 0.0, 1.0], IDENTITY), &game));
    }

    #[test]
    fn restore_puts_the_authors_offset_back_at_the_healed_index() {
        let original = att(1, "gunaim_00", ROOT_BONE, [21.0, 32.0, 43.0], TWEAK);
        let healed = att(4, "gunaim_00", ROOT_BONE, [43.0, 21.0, 32.0], IDENTITY);
        let edits = restore_edits(&original, &healed);
        assert_eq!(edits.len(), 7, "three offset and four rotation components");
        assert!(edits.iter().all(|(path, _)| path[1] == Seg::Index(4)));
        let offsets: Vec<f64> = edits.iter().take(3).map(|(_, v)| *v).collect();
        assert_eq!(offsets, vec![21.0, 32.0, 43.0]);
    }

    #[test]
    fn nothing_to_restore_when_unchanged_or_renamed() {
        let original = att(1, "gunaim_00", ROOT_BONE, [21.0, 32.0, 43.0], TWEAK);
        assert!(restore_edits(&original, &original).is_empty());
        let renamed = att(1, "gunaim_01", ROOT_BONE, [0.0, 0.0, 0.0], IDENTITY);
        assert!(restore_edits(&original, &renamed).is_empty());
    }
}
