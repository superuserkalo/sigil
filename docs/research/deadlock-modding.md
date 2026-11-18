# How Deadlock mods load

Status: active. Verify against the current game build after a patch.
Updated: 2026-11-16

This is the domain ground truth for the harness. It summarizes the community documentation and
names the sources at the bottom. When this document disagrees with the installed game, the game
wins; update the document and the code together.

## Engine and folders

Deadlock runs on Valve's Source 2 engine. The internal game name is `citadel`.

- `game/citadel/pak01_dir.vpk` is the main index. It lists roughly 150,000 files.
- The data is split across numbered chunks, `pak01_000.vpk` upward.
- The container layout and the parser rules are in `docs/research/vpk-format.md`.
- `game/core/pak01_dir.vpk` holds shared engine files.
- Maps are separate VPKs under `game/citadel/maps/`.

Important top-level folders inside the game VPK:

| Folder | Contents |
| --- | --- |
| `models/` | Models, materials, textures, animation clips, skeletons. Heroes live under `models/heroes_wip/` and `models/heroes_staging/`. |
| `materials/` | Shared materials and textures. |
| `sounds/` | Audio, mostly voice lines. |
| `soundevents/` | Scripts that pick sounds for game events. |
| `particles/` | Effects. Hero abilities are under `particles/abilities/`. |
| `panorama/` | UI images, layouts and styles. |
| `animgraphs/` | Animation graphs. |
| `scripts/` | Gameplay data such as hero and ability stats. Never ship these in a mod. |

## Compiled files

The game reads compiled binary formats. Compiled names end in `_c`; source files are ignored by the
game.

| Source | Compiled | Meaning |
| --- | --- | --- |
| `.vmdl` | `.vmdl_c` | Model |
| `.vmat` | `.vmat_c` | Material |
| `.png`, `.tga`, `.psd` | `.vtex_c` | Texture |
| `.wav`, `.mp3` | `.vsnd_c` | Sound |
| `.vsndevts` | `.vsndevts_c` | Sound event script |
| `.vpcf` | `.vpcf_c` | Particle effect |
| `.xml`, `.css`, `.js` | `.vxml_c`, `.vcss_c`, `.vjs_c` | UI |

You do not edit `_c` files. Decompile with Source 2 Viewer, edit the source, compile with CSDK 12.

## Addon loading

The game only sees mods when `game/citadel/gameinfo.gi` lists `citadel/addons` in `SearchPaths`,
above `Game citadel`. Valve's default file does not.

Load order:

1. `citadel/addons`
2. `citadel`
3. `core`

Every game update and every Steam "Verify integrity of game files" restores Valve's `gameinfo.gi`.
The mods stay on disk but stop loading. A manager adds the line back.

Only the `SearchPaths` part may change. Editing other parts of `gameinfo.gi` can block matchmaking.

## Addon slots

- The addons folder loads every `pak01_dir.vpk` through `pak99_dir.vpk`.
- Other names are ignored (`goldcase.vpk`, `pak1_dir.vpk`, `pak00_dir.vpk`, `pak100_dir.vpk`).
- Subfolders are ignored.
- Lower number wins when two mods ship the same file.
- The game loads at most 99 VPKs per folder. Managers add more folders to work around it: Sigil
  numbers slots 1 to 990 across `citadel/addons` and `citadel/sigil-addons2` to `sigil-addons10`
  (`deadlock_core::addons::slot_folder`), mounted after `citadel/addons` in a marked
  `gameinfo.gi` block (`deadlock_core::overflow`). Folder order wins first, then the `pakNN`
  number, so a lower slot still wins. Isolated profiles stay at 99.
- Two mods that each change part of the same shared file can still break each other; order does not
  fix that.

## Payload rules

A payload contains only the compiled files a mod changes, at their exact game paths and original
names.

Never ship:

- Source assets: `.vmat`, `.png`, `.psd`, `.wav`, `.mp3`, `.dmx`, and other source formats.
- Tool cache files: `cache_*.soc`, `tools_thumbnail_cache.bin`.
- Gameplay data: `scripts/*.vdata_c`. It goes stale after updates and changes gameplay.
- Skeletons: `.vnmskel_c`. They break the hero's animations, including other mods.
- Compiled files that were not changed. They clash with other mods and break after updates.
- Programs: `.exe`, `.dll`, `.bat`, `.ps1`, `.scr`, or any other executable.

A wrong folder or letter means the game silently uses its own file. Paths inside the VPK must match
the game paths exactly.

## Staging and packing

The staging folder is laid out like the game: its top level holds game folders such as `models/` or
`sounds/`, never the staging folder's own name.

Packing options:

- Source 2 Viewer: "Create VPK from folder", add the staging folder, check paths, save as
  `pakNN_dir.vpk`.
- CSDK 12: the CS2 Workshop Manager trick creates a VPK; the submit fails but the VPK is written.
- CLI: `Source2Viewer-CLI -i "goldcase" --vpk_create "pak05_dir.vpk" --vpk_create_chunk_size 100`
  writes 100 MB chunks next to the `_dir` file.
- `vpkmerge` merges several VPKs into one or splits one apart.

One CS2 Workshop Manager VPK is limited to 2 GB. Large mods use the Multichunk Workshop Manager or
the CLI chunk size. Keep the whole chunk set together with the same number.

## Compiling

Community CSDK 12 is Valve's Counter-Strike 2 workshop toolset adapted to Deadlock.

- Windows only.
- Compiles through `resourcecompiler.exe` in `game/bin_tools/win64/`.
- Flags: `-i <source file>`, `-r` for a folder with a wildcard, `-f` to force, and
  `-game <path to gameinfo.gi>` for files outside the content folder.
- The Asset Browser and launcher call the same compiler.
- CSDK 12 is pinned to an older game build. Newer formats such as AnimGraph 2 compile incorrectly.
  Hero model swaps may need CS2 Workshop Tools or `DeadlockTools` for the AnimGraph 2 step.

This is why the harness treats compile as a backend with availability, not a guaranteed step.

## Why updates break mods

- Models move. Abrams moved from `heroes_staging/atlas_detective_v2/` to `heroes_wip/abrams/`.
  A mod at the old path stops working with no error.
- Animations, skeletons and cameras change. Old skins can break the camera.
- UI layouts change. Outdated HUD mods can crash at launch.
- `gameinfo.gi` resets.

Repair is path migration plus recompile and repack. Use the `GameTracking-Deadlock` file history to
see what moved instead of guessing.

## Community tools

| Tool | Role | Platform | License |
| --- | --- | --- | --- |
| Source 2 Viewer / ValveResourceFormat | Inspect, export, create VPKs | Cross-platform | MIT |
| CSDK 12 | Compile assets | Windows | No license, do not redistribute |
| vpkeditcli | VPK create, edit, inspect | Cross-platform | MIT |
| vpkmerge | Merge or split VPKs | Cross-platform | MIT |
| DeadPacker | Compile, pack, copy, launch in one run | Windows | MIT |
| DeadlockTools | Add AnimGraph 2 references to compiled models | Windows, Linux | MIT |
| GameTracking-Deadlock | Per-build file history | Browser | Not stated |

## Publishing

GameBanana is the main distribution surface. Community content rules require permission before
sharing changed copies of other people's work. Keep mods cosmetic. Do not redistribute Valve
assets.

## Sources

- [How Deadlock mods work, Divine Skins Wiki](https://wiki.divineskins.gg/en/docs/deadlock/basics/how-mods-work)
- [Pack your mod into a VPK, Divine Skins Wiki](https://wiki.divineskins.gg/en/docs/deadlock/basics/pack-a-vpk)
- [CSDK 12, Divine Skins Wiki](https://wiki.divineskins.gg/en/docs/deadlock/tools/csdk-12)
- [Helper tools, Divine Skins Wiki](https://wiki.divineskins.gg/en/docs/deadlock/tools/helper-tools)
- [Packing an Addon, Deadlock Modding Notes](https://deadlockmodding.pages.dev/modding-guides/csdk12-packing)
- [How to install Mods, GameBanana tutorial](https://gamebanana.com/tuts/17942)
- [Make Your Own Deadlock Mods, deadlocktiers](https://deadlocktiers.com/guides/how-to-make-mods/)
- [Grimoire repository and docs](https://github.com/Slush97/grimoire)
- [vpkmerge](https://github.com/Slush97/vpkmerge)
- [GameTracking-Deadlock](https://github.com/SteamTracking/GameTracking-Deadlock)
