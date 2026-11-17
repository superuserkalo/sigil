//! A GameBanana mod's requirements: the other mods or tools its page says it needs
//! (`_aRequirements`, pairs of a name and a link). Read only, GameBanana only.

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
    let mut r = crate::net::with_proxy(ureq::Agent::config_builder())
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
}
