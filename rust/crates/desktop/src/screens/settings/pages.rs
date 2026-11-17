//! Preferences that change other pages (Grimoire's Preferences section, MIT): the Locker,
//! Installed, Profiles, Overlap, installs and updates, plus script mods. Each value is read
//! by the page it changes through a getter in `options`.

use rune::{El, text};

use super::i18n::tr;
use super::options;
use super::{section, toggle_row};
use crate::app::State;
use crate::ui::{BODY, Ui};

/// Apply what the core reads from these options. At startup and on change.
pub fn apply() {
    deadlock_core::payload::set_allow_scripts(options::get().allow_scripts);
}

pub fn section_el(_s: &State, u: &Ui) -> El {
    let o = options::get();
    section(
        u,
        "Preferences",
        vec![
            toggle_row(
                u,
                "settings-pref-locker",
                "Expand Locker cards",
                "Every hero in the Locker opens with its skins showing.",
                o.locker_expanded,
                |st| options::edit(st, |o| o.locker_expanded = !o.locker_expanded),
            ),
            toggle_row(
                u,
                "settings-pref-open-page",
                "Open the mod page for multi-file mods",
                "In Installed, a press on a mod with several files opens its page instead of the file list.",
                o.installed_open_page,
                |st| options::edit(st, |o| o.installed_open_page = !o.installed_open_page),
            ),
            toggle_row(
                u,
                "settings-pref-profile-confirm",
                "Confirm before updating a profile",
                "Asks before a profile is overwritten with your current mods.",
                o.confirm_profile_update,
                |st| options::edit(st, |o| o.confirm_profile_update = !o.confirm_profile_update),
            ),
            toggle_row(
                u,
                "settings-pref-requirements",
                "Show what a mod needs",
                "Lists the other mods a GameBanana page says it needs, before you install.",
                o.prompt_requirements,
                |st| options::edit(st, |o| o.prompt_requirements = !o.prompt_requirements),
            ),
            toggle_row(
                u,
                "settings-pref-overlap",
                "Ignore overlap by default",
                "The Overlap page stays empty until you turn this off.",
                o.ignore_overlap_default,
                |st| options::edit(st, |o| o.ignore_overlap_default = !o.ignore_overlap_default),
            ),
            toggle_row(
                u,
                "settings-pref-variants",
                "Switch variants",
                "Installing another file of a mod turns the old one off and the new one on.",
                o.switch_variants,
                |st| options::edit(st, |o| o.switch_variants = !o.switch_variants),
            ),
            toggle_row(
                u,
                "settings-pref-match-files",
                "Match remaining update files",
                "When one installed file and one new upload are left over, treat them as an update.",
                o.match_update_files,
                |st| options::edit(st, |o| o.match_update_files = !o.match_update_files),
            ),
            toggle_row(
                u,
                "settings-pref-swap",
                "Replace first",
                "When updating by hand, Replace is the main button and Install the second.",
                o.swap_install_replace,
                |st| options::edit(st, |o| o.swap_install_replace = !o.swap_install_replace),
            ),
            toggle_row(
                u,
                "settings-pref-skip-confirm",
                "Replace without asking",
                "Replacing a mod with a new file skips the confirm. The old file is still backed up.",
                o.skip_replace_confirm,
                |st| options::edit(st, |o| o.skip_replace_confirm = !o.skip_replace_confirm),
            ),
            toggle_row(
                u,
                "settings-pref-scripts",
                "Allow script mods",
                "Installs mods that change gameplay data or skeletons. They break after game updates and can break animations. The install review lists each such file.",
                o.allow_scripts,
                |st| {
                    if options::get().allow_scripts {
                        options::edit(st, |o| o.allow_scripts = false);
                        apply();
                    } else {
                        st.settings.extra.dialog = Some(super::extra::Dialog::AllowScripts);
                    }
                },
            ),
        ],
    )
}

pub fn scripts_dialog(u: &Ui) -> El {
    super::extra::shell(u, "settings-scripts-dialog", tr("Allow script mods"))
        .child(
            text(tr("Script mods replace gameplay data or skeletons. They stop working or break heroes after a game update, and a mod you do not trust can change how the game plays. Sigil still refuses programs and archives inside a mod. Each install lists the script files before you confirm."))
                .font(BODY)
                .color(u.muted)
                .wrap()
                .w_full(),
        )
        .child(super::extra::buttons(u, "Allow", true, |st: &mut State| {
            options::edit(st, |o| o.allow_scripts = true);
            apply();
            super::extra::close(st);
        }))
}
