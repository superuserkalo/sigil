//! A GameBanana mod's requirements: the other mods or tools its page says it needs
//! (`_aRequirements`, pairs of a name and a link). Read only, GameBanana only.
//!
//! [`missing_on_enable`] and [`broken_on_disable`] are the two checks a profile runs around a
//! toggle. The first says what a mod needs before it is enabled, split into a mod that is
//! installed but off and a mod that is not installed. The second says which enabled mods would
//! lose something when a mod is disabled.
//!
//! [`order_problems`] is the third check. Deadlock lets the lowest addon slot win a path two
//! mods both ship, so a recolor or patch that sits in a higher slot than the skin it needs is
//! replaced by that skin and changes nothing. The check reports each such pair with the slot
//! moves that put the dependent directly before its base, which is Grimoire's "Recolor won't
//! show: the base skin is hiding it" fix. Deadlock Mod Manager reports missing requirements
//! only and has no load-order check.
//!
//! GameBanana's plain `_aRequirements` list has no importance column, so every entry counts as
//! required. An entry without a GameBanana mod id (a tool, a credit, a link to another site)
//! cannot be checked against the installed library and is reported as
//! [`RequirementState::Unknown`]. Deadlock Mod Manager treats the same entry as unverified and
//! leaves it out of its warning, so the UI can choose to show it quietly.
//!
//! [`ModRequirements`] groups one mod's id with the list its page returned. Build it from
//! [`fetch_requirements`] or [`parse_requirements`]:
//!
//! ```
//! use deadlock_core::requirements::{ModRequirements, parse_requirements};
//!
//! let body = r#"{"_aRequirements":[["Base pack","https://gamebanana.com/mods/552211"]]}"#;
//! let items = parse_requirements(body)?;
//! let target = ModRequirements::new(Some(715100), items);
//! # Ok::<(), deadlock_core::Error>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::load_order::{SlotMove, move_to_index, moves_for_order};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub name: String,
    pub url: String,
    /// The GameBanana mod id when the link is a GameBanana mod page.
    pub mod_id: Option<u64>,
}

/// `https://gamebanana.com/mods/123` and the like to 123.
pub fn mod_id_of(url: &str) -> Option<u64> {
    let rest = url.split("gamebanana.com/mods/").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|id| *id > 0)
}

pub fn parse_requirements(body: &str) -> Result<Vec<Requirement>> {
    let v: Value = serde_json::from_str(body).map_err(|e| Error::msg(format!("Not JSON: {e}")))?;
    let list = v.get("_aRequirements").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(list
        .iter()
        .filter_map(|pair| {
            let pair = pair.as_array()?;
            let name = pair.first()?.as_str()?.trim().to_string();
            let url = pair.get(1).and_then(Value::as_str).unwrap_or("").trim().to_string();
            (!name.is_empty()).then(|| Requirement {
                mod_id: mod_id_of(&url),
                name,
                url,
            })
        })
        .take(32)
        .collect())
}

pub fn fetch_requirements(mod_id: u64) -> Result<Vec<Requirement>> {
    let url = format!(
        "{}/Mod/{mod_id}?_csvProperties=_aRequirements",
        crate::gamebanana::GAMEBANANA_API_BASE
    );
    let mut r = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent()
        .get(&url)
        .header("User-Agent", crate::gamebanana::GAMEBANANA_USER_AGENT)
        .call()
        .map_err(|e| Error::Network(e.to_string()))?;
    let body = r.body_mut().read_to_string().map_err(|e| Error::Network(e.to_string()))?;
    parse_requirements(&body)
}

// ---------------------------------------------------------------------------
// Checks against the installed library and the active profile (pure)
// ---------------------------------------------------------------------------

/// One mod's requirements, with the mod's own GameBanana id.
///
/// The id lets the checks ignore a page that lists the mod itself as its own requirement.
/// When the id is not known, a self listing cannot be recognised and is checked like any other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModRequirements {
    /// The GameBanana id of the mod these requirements belong to.
    pub gamebanana_id: Option<u64>,
    /// The entries the page returned, in page order.
    pub requirements: Vec<Requirement>,
}

impl ModRequirements {
    pub fn new(gamebanana_id: Option<u64>, requirements: Vec<Requirement>) -> ModRequirements {
        ModRequirements {
            gamebanana_id,
            requirements,
        }
    }
}

/// Why a requirement is not met, or why it cannot be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementState {
    /// The required mod is installed but switched off.
    InstalledOff,
    /// The required mod is not in the library.
    NotInstalled,
    /// The entry is not a GameBanana mod page, so it cannot be checked.
    Unknown,
}

/// One requirement that stands in the way of enabling a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingRequirement {
    /// The id to match against installed and enabled mods, when the entry named one.
    pub gamebanana_id: Option<u64>,
    pub name: String,
    /// The link from the page, when there was one.
    pub url: Option<String>,
    pub state: RequirementState,
}

/// One installed mod, as [`order_problems`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderedMod {
    /// The GameBanana mod id, which is what requirements name.
    pub gamebanana_id: u64,
    /// The addon slot. The lower number loads first and wins a path two mods both ship.
    pub slot: u16,
    /// A disabled mod is not loaded, so it neither hides a mod nor is hidden.
    pub enabled: bool,
    /// The player keeps this mod first (`ModExtra::always_first`), so nothing can be moved
    /// above it.
    pub always_first: bool,
}

/// A mod that loads after the mod it needs, so the needed mod hides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderProblem {
    /// GameBanana id of the mod that needs the other one, for example a recolor.
    pub overlay: u64,
    pub overlay_slot: u16,
    /// GameBanana id of the mod it needs, for example the base skin.
    pub base: u64,
    pub base_slot: u16,
    /// The moves that put the dependent directly before its base. Empty when the base is
    /// always first, because no move can put anything above it. Apply with
    /// [`crate::load_order::plan_reorder`].
    pub moves: Vec<SlotMove>,
    /// Whether [`OrderProblem::moves`] fixes it. False means the player has to clear "always
    /// first" on the base first, since that mod is moved back to the front on the next reorder.
    pub fixable: bool,
}

/// What `target` needs that the active profile does not already have on.
///
/// A requirement is left out when its mod is enabled, and when it names the target itself:
/// enabling the target satisfies that entry. Everything else is kept, in page order, one entry
/// per requirement. [`RequirementState::InstalledOff`] means the user can fix it with one
/// toggle; [`RequirementState::NotInstalled`] means a download. `enabled_ids` and
/// `installed_ids` are GameBanana mod ids, from the library metadata
/// ([`crate::import::ModMeta::gamebanana_id`]) and the active profile.
pub fn missing_on_enable(
    target: &ModRequirements,
    enabled_ids: &[u64],
    installed_ids: &[u64],
) -> Vec<MissingRequirement> {
    target
        .requirements
        .iter()
        .filter_map(|requirement| {
            let state = match requirement.mod_id {
                Some(id) if target.gamebanana_id == Some(id) => return None,
                Some(id) if enabled_ids.contains(&id) => return None,
                Some(id) if installed_ids.contains(&id) => RequirementState::InstalledOff,
                Some(_) => RequirementState::NotInstalled,
                None => RequirementState::Unknown,
            };
            Some(MissingRequirement {
                gamebanana_id: requirement.mod_id,
                name: requirement.name.clone(),
                url: (!requirement.url.is_empty()).then(|| requirement.url.clone()),
                state,
            })
        })
        .collect()
}

/// The enabled mods that require `target_id` and would lose it when it is disabled.
///
/// Returns each mod id once, in the order given. The target itself is left out: it is the mod
/// being switched off, so it cannot be one of its own dependents. A requirement without a
/// GameBanana mod id cannot be matched, so it never counts as a dependent.
pub fn broken_on_disable(target_id: u64, enabled: &[(u64, ModRequirements)]) -> Vec<u64> {
    let mut broken = Vec::new();
    for (id, requirements) in enabled {
        if *id == target_id || broken.contains(id) {
            continue;
        }
        if requirements
            .requirements
            .iter()
            .any(|requirement| requirement.mod_id == Some(target_id))
        {
            broken.push(*id);
        }
    }
    broken
}

/// The enabled mods that load after the mod they need.
///
/// For every entry in `requirements` whose [`ModRequirements::gamebanana_id`] names an enabled
/// mod, each link that names another enabled mod is checked against the slots. When the
/// dependent sits in a higher slot than its base, the base wins every path the two share and
/// the dependent changes nothing, so the pair is reported:
///
/// - [`OrderProblem::moves`] puts the dependent directly before its base;
///   [`crate::load_order::plan_reorder`] turns them into the two rename passes. It is empty
///   when the base is [`OrderedMod::always_first`], because the app would move that mod back to
///   the front on the next reorder;
/// - [`OrderProblem::fixable`] says whether those moves exist.
///
/// A requirement without a GameBanana mod link is left out, as it is in [`missing_on_enable`]:
/// it cannot be matched. So is a link to the dependent itself, and any base that is not
/// installed and enabled. Each pair is reported once and the result is sorted by the
/// dependent's slot, then the base's slot.
///
/// The slot check does not read the VPKs. Grimoire also confirms from the two file lists that
/// the dependent replaces textures the base ships, which keeps a required mod that shares no
/// path out of the warning: [`order_problems_with_files`] does that narrowing.
///
/// `mods` holds one entry per installed mod; a mod without a GameBanana id cannot be matched
/// and is left out by the caller.
pub fn order_problems(mods: &[OrderedMod], requirements: &[ModRequirements]) -> Vec<OrderProblem> {
    let enabled_slots: Vec<u16> = mods
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.slot)
        .collect::<BTreeSet<u16>>()
        .into_iter()
        .collect();

    let mut seen = BTreeSet::new();
    let mut problems = Vec::new();
    for target in requirements {
        let Some(overlay_id) = target.gamebanana_id else {
            continue;
        };
        let Some(overlay) = mods.iter().find(|m| m.gamebanana_id == overlay_id && m.enabled) else {
            continue;
        };
        for requirement in &target.requirements {
            let Some(base_id) = requirement.mod_id else {
                continue;
            };
            if base_id == overlay_id {
                continue;
            }
            let Some(base) = mods.iter().find(|m| m.gamebanana_id == base_id && m.enabled) else {
                continue;
            };
            if overlay.slot <= base.slot || !seen.insert((overlay_id, base_id)) {
                continue;
            }
            let moves = if base.always_first {
                Vec::new()
            } else {
                let at = enabled_slots
                    .iter()
                    .position(|slot| *slot == base.slot)
                    .unwrap_or(0);
                moves_for_order(&enabled_slots, &move_to_index(&enabled_slots, overlay.slot, at))
            };
            problems.push(OrderProblem {
                overlay: overlay_id,
                overlay_slot: overlay.slot,
                base: base_id,
                base_slot: base.slot,
                fixable: !moves.is_empty(),
                moves,
            });
        }
    }
    problems.sort_by_key(|problem| {
        (
            problem.overlay_slot,
            problem.base_slot,
            problem.overlay,
            problem.base,
        )
    });
    problems
}

/// Files a mod may ship next to its resources that say nothing about what it replaces.
const OVERLAY_IGNORED: [&str; 6] = [
    "addoninfo.txt",
    "grimoire_meta.json",
    "readme.txt",
    "readme.md",
    "license",
    "license.txt",
];

/// Grimoire's texture-overlay test (`isTextureOverlay` in `src/lib/requirementLoadOrder.ts`,
/// MIT, ported): `overlay` is a recolor of `base` when it ships at least one texture
/// (`.vtex_c`), every resource it ships is a texture or material (`.vtex_c`, `.vmat_c`) that
/// `base` also ships, and `base` is a model mod (it ships a `.vmdl_c`). Readme, license and
/// addon info files are ignored. Paths compare without case and with either slash.
pub fn is_texture_overlay<O: AsRef<str>, B: AsRef<str>>(overlay: &[O], base: &[B]) -> bool {
    let norm = |path: &str| path.to_ascii_lowercase().replace('\\', "/");
    let resources: Vec<String> = overlay
        .iter()
        .map(|path| norm(path.as_ref()))
        .filter(|path| !OVERLAY_IGNORED.contains(&path.as_str()))
        .collect();
    let base: BTreeSet<String> = base.iter().map(|path| norm(path.as_ref())).collect();
    !resources.is_empty()
        && resources.iter().any(|path| path.ends_with(".vtex_c"))
        && resources.iter().all(|path| {
            (path.ends_with(".vtex_c") || path.ends_with(".vmat_c")) && base.contains(path)
        })
        && base.iter().any(|path| path.ends_with(".vmdl_c"))
}

/// [`order_problems`] narrowed the way Grimoire narrows it: a pair whose two file lists are
/// both in `files` (keyed by GameBanana id, game paths from each mod's VPK) is kept only when
/// [`is_texture_overlay`] says the dependent recolors the base. A pair with a missing list
/// stays, as [`order_problems`] reports it, so an unreadable VPK never hides a real problem.
pub fn order_problems_with_files(
    mods: &[OrderedMod],
    requirements: &[ModRequirements],
    files: &BTreeMap<u64, Vec<String>>,
) -> Vec<OrderProblem> {
    order_problems(mods, requirements)
        .into_iter()
        .filter(|problem| match (files.get(&problem.overlay), files.get(&problem.base)) {
            (Some(overlay), Some(base)) => is_texture_overlay(overlay, base),
            _ => true,
        })
        .collect()
}

/// The game paths inside each VPK, keyed by GameBanana id, for [`order_problems_with_files`].
/// A VPK that cannot be read is left out.
pub fn file_lists(vpks: &[(u64, std::path::PathBuf)]) -> BTreeMap<u64, Vec<String>> {
    let options = crate::vpk::ReadVpkOptions { keep_preload: false };
    vpks.iter()
        .filter_map(|(id, path)| {
            let directory = crate::vpk::read_vpk_directory_with(path, &options).ok()?;
            Some((*id, directory.entries.into_iter().map(|entry| entry.path).collect()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_overlays_match_grimoire() {
        let texture = "materials/skin_color.vtex_c";
        let model = "models/hero.vmdl_c";
        let material = "materials/skin.vmat_c";
        assert!(is_texture_overlay(&[texture], &[texture, model]));
        assert!(is_texture_overlay(&["MATERIALS\\SKIN_COLOR.VTEX_C"], &[texture, model]));
        assert!(is_texture_overlay(&[texture, "README.txt"], &[texture, model]));
        assert!(!is_texture_overlay(&[texture, "scripts/settings.txt"], &[texture, model]));
        assert!(!is_texture_overlay(&[texture, model], &[texture, model]));
        assert!(!is_texture_overlay(&[texture], &["materials/other.vtex_c", model]));
        assert!(!is_texture_overlay::<&str, &str>(&[], &[texture, model]));
        assert!(!is_texture_overlay(&[material], &[material, model]));
        assert!(!is_texture_overlay(&[texture], &[texture]));
    }

    #[test]
    fn file_lists_narrow_order_problems() {
        let mods = [
            OrderedMod { gamebanana_id: 1, slot: 1, enabled: true, always_first: false },
            OrderedMod { gamebanana_id: 2, slot: 2, enabled: true, always_first: false },
            OrderedMod { gamebanana_id: 3, slot: 3, enabled: true, always_first: false },
        ];
        let needs_base = |id| {
            ModRequirements::new(
                Some(id),
                vec![Requirement {
                    name: "Base".into(),
                    url: "https://gamebanana.com/mods/1".into(),
                    mod_id: Some(1),
                }],
            )
        };
        let requirements = [needs_base(2), needs_base(3)];
        assert_eq!(order_problems(&mods, &requirements).len(), 2);

        let base = vec!["materials/a.vtex_c".to_string(), "models/a.vmdl_c".to_string()];
        let mut files = BTreeMap::new();
        files.insert(1, base);
        files.insert(2, vec!["materials/a.vtex_c".to_string()]);
        files.insert(3, vec!["sounds/a.vsnd_c".to_string()]);
        let narrowed = order_problems_with_files(&mods, &requirements, &files);
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].overlay, 2);

        files.remove(&3);
        assert_eq!(order_problems_with_files(&mods, &requirements, &files).len(), 2, "an unknown list keeps the pair");
    }

    #[test]
    fn requirements_are_name_and_link_pairs() {
        let body = r#"{"_aRequirements":[["Base pack","https://gamebanana.com/mods/552211"],["A tool",""],[""]]}"#;
        let list = parse_requirements(body).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].mod_id, Some(552_211));
        assert_eq!(list[1].mod_id, None);
        assert!(parse_requirements("{}").unwrap().is_empty());
    }

    fn requirement(name: &str, id: Option<u64>) -> Requirement {
        Requirement {
            name: name.to_string(),
            url: id.map(|id| format!("https://gamebanana.com/mods/{id}")).unwrap_or_default(),
            mod_id: id,
        }
    }

    #[test]
    fn no_requirements_warns_about_nothing() {
        let target = ModRequirements::new(Some(1), Vec::new());
        assert!(missing_on_enable(&target, &[], &[]).is_empty());
    }

    #[test]
    fn met_requirements_are_not_reported() {
        let target = ModRequirements::new(
            Some(1),
            vec![requirement("Base pack", Some(2)), requirement("Extra pack", Some(3))],
        );
        assert!(missing_on_enable(&target, &[2, 3], &[2, 3]).is_empty());
    }

    #[test]
    fn an_installed_mod_that_is_off_is_reported_off() {
        let target = ModRequirements::new(Some(1), vec![requirement("Base pack", Some(2))]);
        let missing = missing_on_enable(&target, &[], &[2]);
        assert_eq!(
            missing,
            vec![MissingRequirement {
                gamebanana_id: Some(2),
                name: "Base pack".into(),
                url: Some("https://gamebanana.com/mods/2".into()),
                state: RequirementState::InstalledOff,
            }]
        );
    }

    #[test]
    fn a_mod_that_is_not_installed_is_reported_missing() {
        let target = ModRequirements::new(Some(1), vec![requirement("Base pack", Some(2))]);
        let missing = missing_on_enable(&target, &[], &[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].state, RequirementState::NotInstalled);
    }

    #[test]
    fn an_entry_that_is_not_a_gamebanana_mod_is_unknown() {
        let body = r#"{"_aRequirements":[["A tool","https://example.com/tool"],["Credits",""]]}"#;
        let target = ModRequirements::new(Some(1), parse_requirements(body).unwrap());
        let missing = missing_on_enable(&target, &[], &[]);
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0].state, RequirementState::Unknown);
        assert_eq!(missing[0].url.as_deref(), Some("https://example.com/tool"));
        assert_eq!(missing[1].state, RequirementState::Unknown);
        assert_eq!(missing[1].url, None);
    }

    #[test]
    fn disabling_a_mod_names_every_dependent() {
        let enabled = vec![
            (1, ModRequirements::new(Some(1), vec![requirement("Base pack", Some(9))])),
            (2, ModRequirements::new(Some(2), Vec::new())),
            (3, ModRequirements::new(Some(3), vec![requirement("Base pack", Some(9))])),
        ];
        assert_eq!(broken_on_disable(9, &enabled), vec![1, 3]);
        assert!(broken_on_disable(2, &enabled).is_empty());
    }

    #[test]
    fn a_mod_requiring_itself_is_met() {
        let target = ModRequirements::new(Some(7), vec![requirement("Itself", Some(7))]);
        assert!(missing_on_enable(&target, &[], &[]).is_empty());
        assert!(broken_on_disable(7, &[(7, target)]).is_empty());
    }

    fn ordered(gamebanana_id: u64, slot: u16) -> OrderedMod {
        OrderedMod {
            gamebanana_id,
            slot,
            enabled: true,
            always_first: false,
        }
    }

    #[test]
    fn a_recolor_above_its_base_gets_the_moves_that_fix_it() {
        // Base skin in slot 2, recolor in slot 5, an unrelated mod in slot 7.
        let mods = vec![ordered(1, 2), ordered(2, 5), ordered(3, 7)];
        let requirements = vec![ModRequirements::new(
            Some(2),
            vec![requirement("Base skin", Some(1))],
        )];
        let problems = order_problems(&mods, &requirements);
        assert_eq!(
            problems,
            vec![OrderProblem {
                overlay: 2,
                overlay_slot: 5,
                base: 1,
                base_slot: 2,
                moves: vec![SlotMove { from: 5, to: 2 }, SlotMove { from: 2, to: 5 }],
                fixable: true,
            }]
        );
    }

    #[test]
    fn a_dependent_already_before_its_base_is_left_alone() {
        let mods = vec![ordered(2, 1), ordered(1, 4)];
        let requirements = vec![ModRequirements::new(
            Some(2),
            vec![requirement("Base skin", Some(1))],
        )];
        assert!(order_problems(&mods, &requirements).is_empty());
    }

    #[test]
    fn only_enabled_installed_mods_count() {
        let requirements = vec![ModRequirements::new(
            Some(2),
            vec![
                requirement("A base", Some(1)),
                requirement("A mod that is not installed", Some(9)),
                requirement("A tool", None),
            ],
        )];

        // The base is installed but off.
        let mut mods = vec![ordered(2, 5), ordered(1, 2)];
        mods[1].enabled = false;
        assert!(order_problems(&mods, &requirements).is_empty());

        // The dependent is off.
        let mut mods = vec![ordered(2, 5), ordered(1, 2)];
        mods[0].enabled = false;
        assert!(order_problems(&mods, &requirements).is_empty());

        // The dependent is not in the list at all, so its page cannot be matched.
        let mods = vec![ordered(1, 2)];
        assert!(order_problems(&mods, &requirements).is_empty());
    }

    #[test]
    fn a_base_that_is_always_first_cannot_be_moved() {
        let mut mods = vec![ordered(2, 5), ordered(1, 2)];
        mods[1].always_first = true;
        let requirements = vec![ModRequirements::new(
            Some(2),
            vec![requirement("Base skin", Some(1))],
        )];
        let problems = order_problems(&mods, &requirements);
        assert_eq!(problems.len(), 1);
        assert!(!problems[0].fixable);
        assert!(problems[0].moves.is_empty());
    }

    #[test]
    fn a_pair_is_reported_once_and_in_slot_order() {
        let mods = vec![ordered(5, 1), ordered(4, 2), ordered(3, 4), ordered(6, 7)];
        let requirements = vec![
            ModRequirements::new(
                Some(3),
                vec![requirement("Base A", Some(5)), requirement("Base B", Some(4))],
            ),
            ModRequirements::new(
                Some(3),
                vec![requirement("Base A again", Some(5))],
            ),
            ModRequirements::new(Some(4), vec![requirement("Base B", Some(5))]),
        ];
        let problems = order_problems(&mods, &requirements);
        let pairs: Vec<(u64, u64)> = problems.iter().map(|p| (p.overlay, p.base)).collect();
        // Sorted by the dependent's slot, then the base's: overlay 4 sits in slot 2.
        assert_eq!(pairs, vec![(4, 5), (3, 5), (3, 4)]);
        assert_eq!(
            problems[1].moves,
            vec![
                SlotMove { from: 4, to: 1 },
                SlotMove { from: 1, to: 2 },
                SlotMove { from: 2, to: 4 },
            ]
        );
    }

    #[test]
    fn a_page_without_a_mod_id_for_the_dependent_is_skipped() {
        let mods = vec![ordered(1, 2), ordered(2, 5)];
        let requirements = vec![ModRequirements::new(
            None,
            vec![requirement("Base skin", Some(1))],
        )];
        assert!(order_problems(&mods, &requirements).is_empty());
    }
}
