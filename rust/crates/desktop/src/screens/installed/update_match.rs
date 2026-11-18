//! Which new upload updates which installed file, for a mod with several files installed (a
//! model and its recolor, a 4K and a 2K set). Ported from Grimoire's `classifyModFiles`
//! (`src/lib/updateFileMatch.ts`, MIT), cut down to what Sigil's journal knows: the file id each
//! slot came from and when it was installed.
//!
//! A slot whose file is still a current upload is up to date. Every other slot looks for its
//! successor among the newer uploads nobody has installed: the one upload whose name keeps every
//! word of the old name, or a clear best overlap (at least 0.6, ahead of the runner-up by 0.2).
//! Two slots that want the same upload: the stronger match wins, and a tie leaves both alone.
//! Settings > Mods > "Match remaining update files" then pairs one slot left over with one
//! upload left over, even when their names differ. A slot without a match gets no update, so
//! two variants are never both replaced by the same file.

use std::collections::{BTreeMap, BTreeSet};

use deadlock_core::mod_updates::{RemoteFile, RemoteMod};

/// One installed file of the mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub slot: u8,
    /// The GameBanana file id it was installed from, when the journal knows it.
    pub file_id: Option<u64>,
    /// When it was installed, unix seconds.
    pub installed_at: Option<i64>,
}

/// The words of a file name that say what it is: lowercase letters, no extension, no numbers
/// (versions and dates change between uploads).
pub fn words(name: &str) -> BTreeSet<String> {
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name);
    stem.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 2 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !(w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit())))
        .collect()
}

fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f32 / union as f32
}

/// The upload that replaces each installed slot. Slots that are current or have no confident
/// match are left out.
pub fn pair(installed: &[Installed], remote: &RemoteMod, match_remaining: bool) -> BTreeMap<u8, RemoteFile> {
    let current: Vec<&RemoteFile> = remote.files.iter().filter(|f| !f.archived).collect();
    let installed_ids: BTreeSet<u64> = installed.iter().filter_map(|i| i.file_id).collect();
    let stale: Vec<&Installed> = installed
        .iter()
        .filter(|i| i.file_id.is_none_or(|id| !current.iter().any(|f| f.file_id == id)))
        .collect();
    // Uploads newer than the stale file and nobody's install already.
    let newer = |i: &Installed| -> Vec<&RemoteFile> {
        let old_added = i.file_id.and_then(|id| remote.files.iter().find(|f| f.file_id == id)).map(|f| f.added);
        current
            .iter()
            .copied()
            .filter(|f| !installed_ids.contains(&f.file_id))
            .filter(|f| old_added.is_none_or(|at| f.added > at))
            .filter(|f| i.installed_at.is_none_or(|at| f.added > at) || old_added.is_some())
            .collect()
    };

    // Each stale slot's proposal: (target file id, strength).
    let mut proposals: Vec<(u8, u64, f32)> = Vec::new();
    for slot in &stale {
        let Some(old) = slot.file_id.and_then(|id| remote.files.iter().find(|f| f.file_id == id)) else {
            continue;
        };
        let old_words = words(&old.file_name);
        if old_words.is_empty() {
            continue;
        }
        let candidates = newer(slot);
        let keeping: Vec<&&RemoteFile> =
            candidates.iter().filter(|f| old_words.is_subset(&words(&f.file_name))).collect();
        if keeping.len() == 1 {
            proposals.push((slot.slot, keeping[0].file_id, 2.0));
            continue;
        }
        let mut scored: Vec<(u64, f32)> =
            candidates.iter().map(|f| (f.file_id, overlap(&old_words, &words(&f.file_name)))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let best = scored.first().copied();
        let second = scored.get(1).map(|s| s.1).unwrap_or(0.0);
        if let Some((id, score)) = best
            && score >= 0.6
            && score - second >= 0.2
        {
            proposals.push((slot.slot, id, score));
        }
    }

    // Settle claims on one upload: the strongest wins, a tie leaves both unmatched.
    let mut winners: BTreeMap<u8, u64> = BTreeMap::new();
    let targets: BTreeSet<u64> = proposals.iter().map(|p| p.1).collect();
    for target in targets {
        let mut claims: Vec<&(u8, u64, f32)> = proposals.iter().filter(|p| p.1 == target).collect();
        claims.sort_by(|a, b| b.2.total_cmp(&a.2));
        if claims.len() == 1 || claims[0].2 > claims[1].2 {
            winners.insert(claims[0].0, target);
        }
    }

    if match_remaining {
        let unmatched: Vec<&&Installed> = stale.iter().filter(|s| !winners.contains_key(&s.slot)).collect();
        let claimed: BTreeSet<u64> = winners.values().copied().collect();
        if let [only] = unmatched.as_slice() {
            let left: Vec<&RemoteFile> = newer(only).into_iter().filter(|f| !claimed.contains(&f.file_id)).collect();
            if let [file] = left.as_slice() {
                winners.insert(only.slot, file.file_id);
            }
        }
    }

    winners
        .into_iter()
        .filter_map(|(slot, id)| remote.files.iter().find(|f| f.file_id == id).map(|f| (slot, f.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, name: &str, added: i64, archived: bool) -> RemoteFile {
        RemoteFile { file_id: id, file_name: name.to_string(), added, archived }
    }

    fn slot(slot: u8, file_id: u64) -> Installed {
        Installed { slot, file_id: Some(file_id), installed_at: Some(150) }
    }

    #[test]
    fn each_variant_finds_its_own_successor() {
        let remote = RemoteMod {
            files: vec![
                file(1, "haze_skin_4k_v1.zip", 100, true),
                file(2, "haze_skin_2k_v1.zip", 100, true),
                file(3, "haze_skin_4k_v2.zip", 200, false),
                file(4, "haze_skin_2k_v2.zip", 200, false),
            ],
        };
        let pairs = pair(&[slot(5, 1), slot(6, 2)], &remote, false);
        assert_eq!(pairs[&5].file_id, 3);
        assert_eq!(pairs[&6].file_id, 4, "the 2K slot is not given the 4K file");
    }

    #[test]
    fn a_current_file_is_left_alone() {
        let remote = RemoteMod { files: vec![file(1, "a_red.zip", 100, false), file(2, "a_blue.zip", 300, false)] };
        assert!(pair(&[slot(5, 1)], &remote, true).is_empty());
    }

    #[test]
    fn the_remaining_pair_needs_the_switch() {
        let remote = RemoteMod {
            files: vec![file(1, "old_name.zip", 100, true), file(2, "totally_different.zip", 200, false)],
        };
        assert!(pair(&[slot(5, 1)], &remote, false).is_empty(), "names differ: no match without the switch");
        assert_eq!(pair(&[slot(5, 1)], &remote, true)[&5].file_id, 2);
    }

    #[test]
    fn two_slots_wanting_one_upload_with_equal_claims_get_nothing() {
        let remote = RemoteMod {
            files: vec![
                file(1, "pack_red.zip", 100, true),
                file(2, "pack_red.zip", 100, true),
                file(3, "pack_red_new.zip", 200, false),
            ],
        };
        assert!(pair(&[slot(5, 1), slot(6, 2)], &remote, false).is_empty());
    }

    #[test]
    fn version_numbers_do_not_count_as_words() {
        assert_eq!(words("Haze_Skin_v12-2026.zip").into_iter().collect::<Vec<_>>(), vec!["haze", "skin"]);
    }
}
