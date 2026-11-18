//! A GameBanana mod's requirements: the other mods or tools its page says it needs
//! (`_aRequirements`, pairs of a name and a link). Read only, GameBanana only.
//!
//! [`missing_on_enable`] and [`broken_on_disable`] are the two checks a profile runs around a
//! toggle. The first says what a mod needs before it is enabled, split into a mod that is
//! installed but off and a mod that is not installed. The second says which enabled mods would
//! lose something when a mod is disabled.
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

use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
