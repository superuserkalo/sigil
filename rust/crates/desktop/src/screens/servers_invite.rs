//! Join with invite (ADR 0015, second amendment): the joiner's side of a Sigil invite.
//!
//! 1. The player pastes the invite (`deadlock_core::servers::invite::parse_invite` refuses
//!    anything malformed; nothing happens on a bad paste).
//! 2. One dialog lists every GameBanana file the invite names and what Sigil does with each:
//!    already there, turn it on, download it, or replace another file of the same mod.
//! 3. After the player confirms, missing files go through the normal GameBanana install path
//!    (`browse::quick`, the download queue, the same checks and backups). Installs land switched
//!    off by owner rule; Sigil then turns on only the invite's items, through Installed's switch
//!    (its safety and hero questions included).
//! 4. Then the join. An `ip:port` invite goes through the usual join (A2S check, content plan,
//!    password prompt, `steam://connect`). A Steam code cannot be opened from a link yet (not
//!    verified on Windows), so Sigil shows the console line to paste in the game (F7).

use deadlock_core::servers::invite::{Invite, InviteItem, InviteTarget, item_installed, parse_invite};
use rune::{El, TextEdit, Tx, col, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::screens::browse::quick;
use crate::screens::settings::i18n::{fill, tr};
use crate::ui::{ActionKind, BODY, BODY_MED, H1, MONO, SMALL, Ui};

use super::{Dialog, buttons, cancel, close, direct_server, open_join, shell};

/// What Sigil does with one item of the invite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Installed and on.
    Ready(u16),
    /// Installed and off: Sigil turns it on.
    TurnOn(u16),
    /// Not installed: Sigil downloads it.
    Get,
    /// Another file of the same mod is installed in these slots: replaced in place.
    Replace(Vec<u16>),
    /// Sent to the install path; waiting for it.
    Working,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub item: InviteItem,
    pub name: String,
    pub plan: Plan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The paste field.
    Enter(Option<String>),
    /// The one confirm dialog listing what will happen.
    Review { invite: Invite, items: Vec<Item> },
    /// Installing and turning on.
    Installing { invite: Invite, items: Vec<Item> },
    /// Everything is in place: the console line for a Steam code.
    Join(Invite),
}

/// The paste field and the stage, in `servers::View`.
pub struct InviteState {
    pub text: TextEdit,
    pub stage: Stage,
    pub copied: bool,
}

impl Default for InviteState {
    fn default() -> Self {
        InviteState { text: TextEdit::new(""), stage: Stage::Enter(None), copied: false }
    }
}

/// Open the dialog, optionally with a pasted invite (from the Connect field).
pub fn open(st: &mut State, text: &str) {
    st.servers.invite = InviteState { text: TextEdit::new(text.trim()), ..InviteState::default() };
    st.servers.dialog = Some(Dialog::Invite);
    if !text.trim().is_empty() {
        read(st);
    }
}

fn name_for(st: &State, mod_id: u64) -> String {
    st.installed
        .library
        .mods
        .iter()
        .find(|m| m.gamebanana_id == Some(mod_id) && !m.name.is_empty())
        .map(|m| m.name.clone())
        .unwrap_or_else(|| fill(tr("GameBanana mod {}"), &[&mod_id.to_string()]))
}

fn slot_on(st: &State, slot: u16) -> bool {
    st.addons.addons.iter().any(|a| a.slot == slot && a.enabled)
}

/// What to do with each item, from the install journal, the library and the addon folder.
pub fn plan_items(st: &State, invite: &Invite) -> Vec<Item> {
    let library = &st.installed.library.mods;
    let by_slot = |slot: u16| library.iter().find(|m| m.slot == slot).and_then(|m| m.gamebanana_id);
    invite
        .items
        .iter()
        .map(|item| {
            let plan = match item_installed(&st.installed.updates.journal, item, &by_slot) {
                Some(slot) if slot_on(st, slot) => Plan::Ready(slot),
                Some(slot) => Plan::TurnOn(slot),
                None => match quick::owned_slot(st, item.mod_id) {
                    Some(slot) => Plan::Replace(vec![slot]),
                    None => Plan::Get,
                },
            };
            Item { item: *item, name: name_for(st, item.mod_id), plan }
        })
        .collect()
}

/// Read the pasted text: the review, or the reason it is not an invite.
fn read(st: &mut State) {
    let stage = match parse_invite(&st.servers.invite.text.text) {
        Ok(invite) => {
            let items = plan_items(st, &invite);
            Stage::Review { invite, items }
        }
        Err(error) => Stage::Enter(Some(error.to_string())),
    };
    st.servers.invite.stage = stage;
}

/// The player's confirm: install what is missing (landing off), then turn on the invite's items.
fn confirm(st: &mut State) {
    let Stage::Review { invite, mut items } = st.servers.invite.stage.clone() else {
        return;
    };
    for entry in &mut items {
        let update = match &entry.plan {
            Plan::Get => Vec::new(),
            Plan::Replace(slots) => slots.clone(),
            _ => continue,
        };
        if st.demo || rune::is_headless() {
            entry.plan = Plan::Working;
            continue;
        }
        quick::start(st, entry.item.mod_id, &entry.name, entry.item.download_target(), Some(entry.item.file_id), update);
        entry.plan = Plan::Working;
    }
    st.servers.invite.stage = Stage::Installing { invite, items };
    advance(st);
}

/// Follow the installs; when every item is in place, turn on the ones that are off and move to
/// the join. Called from the Servers poll.
pub fn advance(st: &mut State) -> bool {
    let Stage::Installing { invite, mut items } = st.servers.invite.stage.clone() else {
        return false;
    };
    let mut changed = false;
    for entry in &mut items {
        if entry.plan != Plan::Working {
            continue;
        }
        match st.browse_view.installs.items.get(&entry.item.mod_id).map(|e| e.step.clone()) {
            Some(quick::Step::Done) => {
                if let Some(slot) = quick::owned_slot(st, entry.item.mod_id) {
                    entry.plan = Plan::TurnOn(slot);
                    changed = true;
                }
            }
            Some(quick::Step::Failed(error)) => {
                entry.plan = Plan::Failed(error);
                changed = true;
            }
            _ => {}
        }
    }
    let settled = items.iter().all(|i| !matches!(i.plan, Plan::Working));
    if !settled {
        if changed {
            st.servers.invite.stage = Stage::Installing { invite, items };
        }
        return changed;
    }
    if items.iter().any(|i| matches!(i.plan, Plan::Failed(_))) {
        st.servers.invite.stage = Stage::Installing { invite, items };
        return changed;
    }
    // Only the invite's items are turned on; every other install stays as it is.
    let off: Vec<u16> = items
        .iter()
        .filter_map(|i| match i.plan {
            Plan::TurnOn(slot) if !slot_on(st, slot) => Some(slot),
            _ => None,
        })
        .collect();
    if !(st.demo || rune::is_headless()) {
        for slot in off {
            crate::screens::installed::enable::switch(st, slot, true);
        }
    }
    join(st, invite);
    true
}

fn join(st: &mut State, invite: Invite) {
    match &invite.target {
        InviteTarget::Address(addr) => {
            st.servers.invite.stage = Stage::Enter(None);
            let address = addr.to_string();
            open_join(st, direct_server(&address, &address, ""));
        }
        InviteTarget::Steam(_) => {
            st.servers.password = TextEdit::new("");
            st.servers.invite.stage = Stage::Join(invite);
            st.servers.dialog = Some(Dialog::Invite);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The dialog
// ---------------------------------------------------------------------------------------------

fn plan_line(plan: &Plan) -> (String, bool) {
    match plan {
        Plan::Ready(slot) => (fill(tr("Installed (pak{})"), &[&format!("{slot:02}")]), false),
        Plan::TurnOn(slot) => (fill(tr("Installed but off (pak{}): Sigil turns it on"), &[&format!("{slot:02}")]), false),
        Plan::Get => (tr("Sigil downloads it from GameBanana and turns it on").to_string(), false),
        Plan::Replace(_) => (tr("You have another version: Sigil installs the host's file in its place").to_string(), false),
        Plan::Working => (tr("Installing...").to_string(), false),
        Plan::Failed(error) => (fill(tr("Not installed: {}"), &[error]), true),
    }
}

fn item_rows(u: &Ui, items: &[Item]) -> El {
    let mut list = col().w_full().gap(6.0);
    for entry in items {
        let (line, bad) = plan_line(&entry.plan);
        list = list.child(
            row()
                .key(format!("invite-item-{}", entry.item.mod_id))
                .w_full()
                .gap(10.0)
                .items_start()
                .padding(10.0)
                .radius(crate::ui::r(8.0))
                .fill(u.well)
                .border(1.0, u.hair)
                .child(icon(&icons::PACKAGE, 16.0).color(if bad { u.rose } else { u.accent }).mt(2.0).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text(entry.name.clone()).font(BODY_MED).color(u.ink).truncate())
                        .child(
                            text(fill(tr("Mod {}, file {}"), &[&entry.item.mod_id.to_string(), &entry.item.file_id.to_string()]))
                                .font(SMALL)
                                .color(u.faint),
                        )
                        .child(text(line).font(SMALL).color(if bad { u.rose } else { u.muted }).wrap().w_full()),
                ),
        );
    }
    list
}

fn target_line(invite: &Invite) -> String {
    match &invite.target {
        InviteTarget::Steam(code) => fill(tr("Joins {} through Steam (no address needed)."), &[code]),
        InviteTarget::Address(addr) => fill(tr("Joins {}."), &[&addr.to_string()]),
    }
}

pub fn dialog(s: &State, u: &Ui) -> El {
    let state = &s.servers.invite;
    let mut body = col().w_full().gap(12.0).child(text(tr("Join with invite")).font(H1).color(u.ink));
    match &state.stage {
        Stage::Enter(error) => {
            body = body
                .child(text(tr("Paste the invite a friend copied from Sigil. It starts with sigil-invite:.")).font(BODY).color(u.muted).wrap())
                .child(
                    row()
                        .w_full()
                        .h(32.0)
                        .px(10.0)
                        .center_y()
                        .radius(crate::ui::r(8.0))
                        .fill(u.well)
                        .border(1.0, if error.is_some() { u.rose.alpha(0.6) } else { u.hair })
                        .child(
                            rune::text_input(&state.text, MONO)
                                .key("servers-invite-input")
                                .label(tr("Invite"))
                                .single_line()
                                .grow(1.0)
                                .min_w(0.0)
                                .color(u.ink)
                                .placeholder("sigil-invite:v1:...".to_string(), u.faint)
                                .caret(u.accent, u.accent.alpha(0.20))
                                .on_edit(|st: &mut State| &mut st.servers.invite.text),
                        ),
                );
            if let Some(error) = error {
                body = body.child(text(error.clone()).font(SMALL).color(u.rose).wrap());
            }
            body = body.child(buttons(vec![
                cancel(u, tr("Cancel")),
                u.icon_action(&icons::LOG_IN, tr("Continue"), ActionKind::Primary)
                    .key("servers-invite-read")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(read),
            ]));
        }
        Stage::Review { invite, items } => {
            body = body.child(text(target_line(invite)).font(BODY).color(u.prose).wrap());
            let work = items.iter().any(|i| matches!(i.plan, Plan::Get | Plan::Replace(_) | Plan::TurnOn(_)));
            if items.is_empty() {
                body = body.child(text(tr("The server uses no content from GameBanana.")).font(SMALL).color(u.muted).wrap());
            } else {
                body = body
                    .child(text(tr("The server uses these GameBanana files. Sigil installs what you do not have and turns on only these:")).font(SMALL).color(u.muted).wrap())
                    .child(item_rows(u, items));
            }
            if invite.password {
                body = body.child(text(tr("The server asks for a password. Ask your friend for it.")).font(SMALL).color(u.amber).wrap());
            }
            body = body.child(buttons(vec![
                cancel(u, tr("Cancel")),
                u.icon_action(&icons::LOG_IN, if work { tr("Install and join") } else { tr("Join") }, ActionKind::Primary)
                    .key("servers-invite-go")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(confirm),
            ]));
        }
        Stage::Installing { invite, items } => {
            let failed = items.iter().any(|i| matches!(i.plan, Plan::Failed(_)));
            body = body
                .child(text(target_line(invite)).font(BODY).color(u.prose).wrap())
                .child(item_rows(u, items))
                .child(
                    text(if failed {
                        tr("Some files could not be installed, so Sigil did not join. Try again, or install them from Browse.")
                    } else {
                        tr("Downloads show on the Downloads page. Sigil joins when everything is in place.")
                    })
                    .font(SMALL)
                    .color(if failed { u.rose } else { u.muted })
                    .wrap(),
                )
                .child(buttons(vec![cancel(u, tr("Close"))]));
        }
        Stage::Join(invite) => {
            let password = invite.password.then(|| s.servers.password.text.trim()).filter(|p| !p.is_empty());
            let line = invite.target.console_line(password);
            body = body
                .child(text(tr("Everything is in place. Start Deadlock, open the console (F7), paste this line and press Enter:")).font(BODY).color(u.prose).wrap());
            if invite.password {
                body = body.child(super::password_field(s, u));
            }
            match line {
                Ok(line) => {
                    let copied = state.copied;
                    let copy = line.clone();
                    body = body.child(
                        row()
                            .key("servers-invite-line")
                            .w_full()
                            .h(34.0)
                            .pl(10.0)
                            .pr(2.0)
                            .gap(8.0)
                            .center_y()
                            .radius(crate::ui::r(8.0))
                            .fill(u.well)
                            .border(1.0, u.hair)
                            .child(text(line).font(MONO).color(u.ink).grow(1.0).min_w(0.0).truncate().selectable(true))
                            .child(
                                u.icon_button(if copied { &icons::CHECK } else { &icons::COPY }, if copied { tr("Copied") } else { tr("Copy") })
                                    .key("servers-invite-copy")
                                    .cursor(rune::Cursor::Pointer)
                                    .on_press_tx(move |st: &mut State, tx: &mut Tx| {
                                        tx.copy(copy.clone());
                                        st.servers.invite.copied = true;
                                    }),
                            ),
                    );
                }
                Err(error) => body = body.child(text(error.to_string()).font(SMALL).color(u.rose).wrap()),
            }
            body = body
                .child(text(tr("Steam cannot open a relay code from a link, so the game console is the way in. The code changes when the host restarts; ask for a new invite then.")).font(SMALL).color(u.faint).wrap())
                .child(buttons(vec![u.action(tr("Done"), ActionKind::Primary).key("servers-invite-done").cursor(rune::Cursor::Pointer).on_press(close)]));
        }
    }
    shell(u, "servers-invite", tr("Join with invite").to_string(), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_paste_stays_on_the_field_with_the_reason() {
        let mut st = State::default();
        open(&mut st, "connect [A:1:5:6]");
        let Stage::Enter(Some(error)) = &st.servers.invite.stage else { panic!("not refused") };
        assert!(error.contains("not a Sigil invite"), "{error}");
        assert!(matches!(st.servers.dialog, Some(Dialog::Invite)));
    }

    #[test]
    fn an_invite_lists_every_item_to_get_and_joins_after() {
        let mut st = State { demo: true, ..State::default() };
        open(&mut st, "sigil-invite:v1:[A:1:5:6]:p:715100/11,600009/22");
        let Stage::Review { invite, items } = st.servers.invite.stage.clone() else { panic!("no review") };
        assert!(invite.password);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.plan == Plan::Get), "{items:?}");
        confirm(&mut st);
        let Stage::Installing { items, .. } = st.servers.invite.stage.clone() else { panic!("not installing") };
        assert!(items.iter().all(|i| i.plan == Plan::Working));
    }

    #[test]
    fn an_invite_with_nothing_to_install_goes_to_the_console_line() {
        let mut st = State { demo: true, ..State::default() };
        open(&mut st, "sigil-invite:v1:[A:1:5:6]:o:-");
        confirm(&mut st);
        let Stage::Join(invite) = &st.servers.invite.stage else { panic!("not joining") };
        assert_eq!(invite.target.console_line(None).unwrap(), "connect [A:1:5:6]");
    }
}
