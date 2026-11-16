# Crosshair

Status: active. Verified against the vendored Grimoire tree and a live
`machine_convars.vcfg` fixture.
Updated: 2026-11-16

The crosshair is a set of `citadel_crosshair_*` convars the game reads at launch
and after a change in the in-game editor. The workbench edits them locally and
writes one managed block into `game/citadel/cfg/autoexec.cfg`.

## Convar surface

The old `citadel_crosshair_pip_border` bool no longer exists. Pip and dot
outlines are `(border, gap, opacity)` triples sharing an outline colour, and the
dot has an explicit size.

| Convar | Meaning |
| --- | --- |
| `citadel_crosshair_pip_gap` | distance from the centre, layout px |
| `citadel_crosshair_pip_gap_static` | gap does not grow when moving |
| `citadel_crosshair_pip_height` / `_width` | pip size |
| `citadel_crosshair_pip_opacity` | pip alpha, 0 to 1 |
| `citadel_crosshair_pip_outline_border` / `_gap` / `_opacity` | pip outline |
| `citadel_crosshair_dot_size` / `_opacity` | centre dot |
| `citadel_crosshair_dot_outline_border` / `_gap` / `_opacity` | dot outline |
| `citadel_crosshair_color_r` / `_g` / `_b` | pip and dot colour |
| `citadel_crosshair_outline_color_r` / `_g` / `_b` | outline colour |
| `citadel_crosshair_disable_hero_specific_crosshairs` | one reticle for every hero |

The full set is always written. A partial write leaves the rest of the previous
crosshair in place, which is why a preset that only sets a few values never
looked the same in game.

## Geometry

The preview mirrors the HUD, not an approximation:

- Sizes are composed in 1080p layout units and snapped once to device pixels
  (`scale` = display height / 1080).
- A pip centre sits `4 + pip_gap` from the screen centre. The `4` is
  `cppArcDefaultOffset` in `element_gun.vcss`.
- The left and right pips are the top and bottom pair rotated 90 degrees.
- Outline panels are sized to the outer box and paint inward, leaving
  `gap / 2` clearance per side.
- The dot and its outline are circles; the dot's border also paints inward.
- The HUD truncates sizes toward zero, so a dot size of `8.3` draws an 8px dot.

`Crosshair::layout(scale)` returns those rectangles, and the desktop preview
draws them directly. The unit tests assert the centre symmetry.

## Where the values come from

- `machine_convars.vcfg` in `game/citadel/cfg` is where the game persists the
  in-game editor. `parse_machine_convars` reads it; a file with no crosshair
  convars returns `None`, which is different from "uses the defaults".
- The editor defaults are deliberately tighter than the values a fresh install
  registers. Both are in `crosshair.rs` and the difference is intentional.

## Write path

```text
autoexec.cfg
  // dah:crosshair begin
  citadel_crosshair_pip_gap 8
  ...
  // dah:crosshair end
```

- The block is replaced in place; every other line is kept.
- A begin marker with no end marker is refused before anything is written.
- The write goes through `autoexec::write_autoexec`, so the old file is copied
  beside itself first and a NUL byte is refused.
- `dah crosshair clear` removes only the block.

## Limits

- No share codes. The community share code is a remote encode/decode service;
  the local model is the source of truth here.
- The preview is a 2D HUD match, not a 3D viewmodel render.
- The game must be closed when the autoexec is written, or the client may
  overwrite the file on exit. The app states this and keeps a backup.
