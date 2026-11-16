# Locker and recolor

Status: active. Verified against the vendored Grimoire art set and a fixture game
catalog.
Updated: 2026-11-16

The Locker is the hero surface: it lists heroes from the game catalog, resolves a
render and name art from a local asset root, and records recolor work orders. The
art is read at runtime from a configured folder; it is never copied into this
repository.

## Where heroes come from

Two sources, merged:

1. **Game catalog.** Hero folders under `models/heroes_*` and
   `materials/models/heroes_*` give the codename, the path count and the
   recolorable material and texture paths.
2. **Local art root.** Grimoire's `public/locker` set supplies the render and the
   name art. A hero that only exists in the art set is listed as *art only*: zero
   catalog paths, zero mods, and never presented as a game asset.

An art hero whose display name matches a catalog hero is merged into one card, so
`billy` (art) and `punkgoat` (catalog codename) both label Billy rather than
appearing twice. The art root is resolved from `DAH_LOCKER_ASSETS`, then a
development checkout at `vendor/grimoire/public`, then nothing.

## Project hero

The Locker pins the hero the open project targets. The codename comes from the
first target under a `heroes_*` folder, preferring `materials/` because a material
is the recolor surface. The pinned hero sorts first, is selected by default, and
carries a `PROJECT` badge. A project with no hero target pins nothing.

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

It refuses a hero with no catalog material. An art-only hero has nothing to
recolor, and the screen says so instead of inventing a path.

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
- Sound replacement is not modelled. It needs the same decompile and compile
  path with a different payload type.
- The art set is Grimoire's, read in place under its MIT licence. No render or
  name image is stored in this repository.
