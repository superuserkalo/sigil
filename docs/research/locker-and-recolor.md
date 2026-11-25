# Locker and recolor

Status: active. Verified against the packaged art set, a fixture catalog with real
Deadlock folder names, and fixture addon VPKs. Not yet verified on a live Windows install.
Updated: 2026-11-25

The Locker is the hero surface, ported from Grimoire's Locker: a General tile and one
card per playable hero, a hero page with that hero's mods and recolor, and a General
page with the non-hero mods by type plus the user's own categories. Recolor work orders
are recorded as briefs. Art is read at runtime from the app's asset folder; it is never
copied into this repository.

The page also carries the owner's round 3 additions: starred heroes and search, a list
view, one skin per hero with a per-hero order, launch shuffle, ability sounds, custom
card art, an applied changes review and the Global priority folder. The 3D preview shipped in
0.1.16 to 0.1.18 on its own renderer (`rust/crates/sigil-viewer`, `docs/design/3d-viewer.md`).
Recolor is still briefs only (`deadlock_core::recolor`).

## Where heroes come from

The roster is curated (`ROSTER` in `deadlock-core/src/locker.rs`, ported from
Grimoire's `heroCodenames.ts` and `lockerUtils.ts`): one entry per playable hero with
the display name, the codenames the game and community packs use, and the face
position Grimoire calibrated for cropping the render. Every hero always shows.

The game catalog only adds facts. A folder after a `heroes_*` segment maps to a hero
by exact codename, then with `_vN` suffixes dropped (`inferno_v4`), then a display-name
folder (`lady_geist`), then the longest codename prefix (`atlas_detective` is Abrams).
Anything else (NPCs, rigs, `kali`, `trooper`) is ignored. Before 2026-11-17 the
roster was the raw folder list, which showed `Archer`, `Archer V2`, `Artist`, three
Abrams cards and NPCs, none of them matching an art file.

Art resolves from `DAH_LOCKER_ASSETS`, then `assets/locker`, `locker` or
`../Resources/locker` beside the executable (and beside the resolved executable),
then `assets/locker` under the working directory, then a development checkout at
`vendor/grimoire/public`. The loader reads each image's pixel size from its header so
cards crop on the face before the decode lands.

## Mods and General

Each installed addon is read once. A mod that ships a path under a hero folder counts
for that hero. A mod that touches no hero is classified into a General type (Soul
Containers, Spirit Urns, Hideout, Icon Packs, HUD, Effects, Animations, Maps,
Announcer / SFX, Killstreak Music) from its paths; Grimoire uses the GameBanana
category instead, which Sigil does not record yet. A mod that matches nothing shows
only in a user category.

Categories (Grimoire's `lockerCategories.ts`) are named piles, view only: filing never
enables, disables or moves a mod. Membership is keyed by a content hash of the VPK
directory (paths and CRCs), so it survives the enable rename and a slot move. The
store is `<data dir>/locker-categories.json`, written atomically; it never touches
the game folder.

## Project hero

The Locker pins the hero the open project targets. The codename comes from the
first target under a `heroes_*` folder, preferring `materials/` because a material
is the recolor surface. The pinned hero sorts first and its card says Project. A
project with no hero target pins nothing.

## Favorites, search and the list view

`locker-prefs.json` in the app data directory holds what the page remembers. A star on
a hero card, a list row or the hero page records the hero by display name and is stored
as the normalized art key (`lady_geist`), so `ghost`, `Lady Geist` and `lady_geist` are
the same star. The grid sorts the project hero first, then starred heroes, then the
roster.

The page head has a hero search (display name, codename and every roster alias) and an
All / Modded / Starred switch. The view switch turns the grid into Grimoire's list view:
one full-width row per hero with a small render, a star, a chevron that opens the row to
its skins and an action that opens the hero page. With Settings > Expand Locker cards on,
a loaded list starts with every row open. The hero page's chevrons walk the filtered
list, so Next after a search stays inside the filter.

## Skins, equip and shuffle

A hero's installed mods are its skins, listed in load order. **Equip** turns that skin on
and the hero's other skins off, so exactly one look loads. The up and down arrows move a
skin inside the hero's own numbers only (`deadlock_core::locker::move_hero_skin`), then
`load_order` writes the renames and the Installed library is remapped with the slots.
Both are game writes with `approve: true` from a button press.

**Shuffle on launch** is opt-in per hero, in the hero rail or the grid's shuffle dialog.
`shuffle_picks` is pure and seeded, so a given seed picks the same skins; it enables one
skin per chosen hero and turns that hero's other mods off. `data::shuffle_now` applies it
now with the wall clock as the seed (the dialog's "Shuffle now"). The launch handler must
call `data::shuffle_before_launch` before a modded launch; that hook is listed in the round
report because `screens/launch.rs` and `app.rs` are not this area's files.

## Ability sounds

Deadlock ability sounds live at `sounds/abilities/<codename>/...`. `locker_sound` maps
that folder to a roster hero through the same codename table as skins, and takes the
ability from an `aN` token in the rest of the path (`a4_ult/`, `a2_charge/`, `_a3_`). A
file with no token still belongs to the hero under Other sounds, never to a guessed
ability. The hero page's Sounds section groups the installed sound mods by ability and
offers Equip, which turns the chosen sound on and the other sounds for that ability off
(`equip_ability_sound`).

This is path evidence, not Grimoire's full display-name classifier: a pack that names the
ability only by its lore name falls under Other sounds. The list is honest about that
instead of guessing.

## Custom cards

A custom card is an image copied into `<data>/locker-cards/` and keyed by hero. The picker
accepts PNG, JPEG and WebP, reads the header for the aspect ratio, and offers a crop slider
for the horizontal face position. The copy is named `<hero>-<hash>.<ext>` from the source
path, size and modified time, written through a temp file and renamed, so choosing the same
image twice reuses one file and nothing is overwritten. Cards are app data only: the game
folder is never touched. Remove deletes the copy and the hero's own render shows again.

## Applied changes

The grid's Locker changes dialog lists what the page set up and can remove each row: custom
cards, shuffled heroes, enabled ability sound mods (turn off) and recolor briefs (delete the
workspace JSON). It reads from the last Locker load plus the live addon library, so a switch
disappears on the same redraw.

## Global priority folder

Grimoire mounts `citadel/grimoire` ahead of `citadel/addons` and puts its Global mods there.
Sigil's General page has a switch for that search path. `ensure_global_path` inserts
`Game\t\tcitadel/grimoire` above the first `Game citadel/addons` line (or above
`Game citadel`), creates `game/citadel/grimoire`, copies gameinfo.gi to
`gameinfo.gi.sigil-backup-<stamp>` and writes atomically. `remove_global_path` takes only
that line out. Both take `approve`, both are idempotent, and a file with no Game line is
refused instead of guessed at. Moving a mod into the folder from Installed is not built;
a VPK placed there by hand already loads first once the switch is on.

## Recolor briefs

A compiled `vtex_c` or `vmat_c` needs a decompile, an edit and a compile before it
can ship. That toolchain is external and often absent, so the Locker records a
**brief** instead of pretending to edit a game file.

```text
<workspace>/generated/recolors/<hero>.json
```

The brief carries:

- the hero codename and display name,
- the exact catalog targets (up to 16 materials and textures),
- a palette (`ember`, `frost`, `toxic`, `void`) with a hue shift, saturation and
  value multiplier,
- a note,
- the steps that remain: decompile, edit, material values, CSDK compile, pack and
  validate, deploy with approval, in-game check,
- `inGameReady: false`, always.

It refuses a hero with no catalog material. Without an indexed game there is nothing
to recolor, and the hero page says so instead of inventing a path.

## Commands

```text
dah locker list [--modded]
dah locker recolor <hero> --palette frost
dah locker recolor billy --hue 30 --saturation 1.2 --value 0.9 --note "winter event"
```

The agent has `locker_heroes` (read) and `recolor_brief` (workspace write). The
brief never writes the game install and never claims an asset exists.

## Limits

- No texture editing in this crate. The brief is the handoff, not the edit.
- Sound replacement is modelled for installed mods. Compiling a new sound still needs
  the same decompile and compile path with a different payload type.
- Ability sounds are slotted by an `aN` path token and the hero folder, not by the full
  display-name table Grimoire carries.
- The art set is Grimoire's, packed beside the executable by
  `scripts/pack-locker-art.sh` under its MIT licence. No render or name image is
  stored in this repository.
- The 3D viewer is built (`docs/design/3d-viewer.md`) but not yet checked against a real game
  install.
- The Global folder mounts and accepts hand-placed VPKs. Marking an installed mod Global
  and moving its files is not built yet.
