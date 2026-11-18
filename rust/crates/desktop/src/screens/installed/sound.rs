//! Sound preview on Installed cards (DMM's play button on sound mods).
//!
//! - Deadlock Mod Manager: a play button on installed sound mods.
//! - Grimoire: sound previews in Browse only.
//! - Sigil: a sound mod's card (GameBanana category Sound, Audio or Music) gets Play. The first
//!   press reads the mod's GameBanana page on a worker for its sample (`audio_url`, the Sounds
//!   section first, then Mods) and hands it to Browse's preview player, which plays at the shared
//!   preview volume. One sample plays at a time across the app.

use std::collections::HashMap;

use deadlock_core::gamebanana::{self, BrowseSection};
use rune::El;

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::AddonRow;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::Ui;

#[derive(Default)]
pub struct Sounds {
    /// The sample by GameBanana id; `None` when the page has none.
    pub urls: HashMap<u64, Option<String>>,
    pub job: Job<(u64, Option<String>)>,
}

/// A sound mod by its GameBanana category.
pub fn is_sound(root: Option<&str>, leaf: Option<&str>) -> bool {
    [root, leaf].into_iter().flatten().any(|name| {
        let name = name.to_ascii_lowercase();
        name.contains("sound") || name.contains("audio") || name.contains("music")
    })
}

fn id_of(s: &State, addon: &AddonRow) -> Option<u64> {
    let extra = super::notes::extra(s, addon);
    if !is_sound(extra.root_category.as_deref(), extra.category.as_deref()) {
        return None;
    }
    super::meta_for(s, addon).and_then(|m| m.gamebanana_id)
}

/// Play, or stop the one playing.
fn press(st: &mut State, id: u64) {
    if let Some(Some(url)) = st.installed.sounds.urls.get(&id).cloned() {
        st.browse_view.audio.toggle(id, url);
        return;
    }
    if st.installed.sounds.job.is_running() || st.demo || rune::is_headless() || st.prefs.offline {
        return;
    }
    st.browse_view.audio.loading = Some(id);
    st.installed.sounds.job.start(st.wake.clone(), move || {
        let url = [BrowseSection::Sounds, BrowseSection::Mods]
            .into_iter()
            .find_map(|section| gamebanana::fetch_profile(section, id).ok().and_then(|p| p.audio_url));
        Ok((id, url))
    });
}

/// Take a page read: play its sample, or say there is none.
pub fn poll(st: &mut State) -> bool {
    let Some(result) = st.installed.sounds.job.try_take() else { return false };
    st.browse_view.audio.loading = None;
    match result {
        Ok((id, Some(url))) => {
            st.installed.sounds.urls.insert(id, Some(url.clone()));
            st.browse_view.audio.toggle(id, url);
        }
        Ok((id, None)) => {
            st.installed.sounds.urls.insert(id, None);
            st.status = tr("This mod's GameBanana page has no sound sample.").to_string();
        }
        Err(error) => st.status = error,
    }
    true
}

/// The Play button on a sound mod's card, or `None`.
pub fn button(s: &State, u: &Ui, addon: &AddonRow) -> Option<El> {
    let id = id_of(s, addon)?;
    if s.installed.sounds.urls.get(&id).is_some_and(Option::is_none) {
        return None;
    }
    let playing = s.browse_view.audio.playing == Some(id);
    let loading = s.browse_view.audio.loading == Some(id);
    let name = super::display_name(s, addon);
    Some(
        u.icon_button(
            if playing { &icons::SQUARE_STOP } else if loading { &icons::LOADER } else { &icons::PLAY },
            &fill(if playing { tr("Stop the sample of {}") } else { tr("Play the sample of {}") }, &[&name]),
        )
        .key(format!("installed-sound-{}", addon.slot))
        .cursor(rune::Cursor::Pointer)
        .on_press(move |st: &mut State| press(st, id)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_mods_are_found_by_category() {
        assert!(is_sound(Some("Sound"), None));
        assert!(is_sound(None, Some("Killstreak Music")));
        assert!(!is_sound(Some("Skins"), Some("Haze")));
    }
}
