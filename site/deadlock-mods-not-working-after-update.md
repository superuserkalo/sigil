# Deadlock mods not working after an update

> Why Deadlock mods stop loading after a game update or a Steam file check, how to fix gameinfo.gi by hand, and what to do when a mod's files moved.

If your mods worked yesterday and the game looks stock today, a Deadlock update is the usual cause. There are two different problems, and they need different fixes.

## Problem 1: the game no longer loads the addons folder

Deadlock only reads mods when `game/citadel/gameinfo.gi` lists `citadel/addons` in its `SearchPaths`, above `Game citadel`. Valve's own copy of the file does not have that line. Every game update, and every time you use Steam's "Verify integrity of game files", the file is put back to Valve's version. Your mods are still on disk, but the game skips all of them, with no error.

This is the most common cause, and it affects every mod at once.

### Fix it by hand

1. Close Deadlock.
2. Open `game/citadel/gameinfo.gi` in a text editor.
3. In the `SearchPaths` block, add `Game citadel/addons` directly above `Game citadel`.
4. Save, and start the game.

Only change the `SearchPaths` block. Editing other parts of the file can block matchmaking. You will need to do this again after the next update.

### Fix it with Sigil

When Sigil sees the reset it shows a banner on every page with a Fix button. The fix adds the line back, copies the old file aside first so it can be undone, and keeps search paths another tool added. Launch modded also checks the file before it starts the game.

![The Fix gameinfo.gi dialog: Sigil adds back the line that tells Deadlock to load mods, and copies the current file aside first.](https://sigilmods.app/img/gameinfo-fix.webp)

## Problem 2: some mods broke because game files changed

If only some mods stopped working, the update probably changed the files they replace. A mod replaces game files by exact path, so when the game moves a file, the mod's copy no longer matches and the game quietly uses its own. For example, Abrams' model moved from `models/heroes_staging/atlas_detective_v2/` to `models/heroes_wip/abrams/`, and skins made for the old path stopped working.

Updates can also change animations, skeletons and cameras, which can break older skins, and change interface layouts, which can make an outdated HUD mod crash the game at launch.

### What to do

- Check the mod's GameBanana page for an update made for the new build, and install it.
- If the game crashes at launch, turn off HUD and interface mods first.
- Turn mods off in halves to find the one that breaks the game.
- Fixing an old mod yourself means moving its files to the new paths, then recompiling and repacking it. The GameTracking-Deadlock file history shows what moved.

### With Sigil

Installed shows an update chip on each mod with a newer file. The compatibility review lists mods that use paths the game moved or removed, stale hero models or game data, with Repair, Fix all and Revert, and can turn off every mod with outdated data in one step. Find a broken mod halves your enabled mods across launches until it finds the culprit, and Launch vanilla starts a clean game to compare.

## Download Sigil

[Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)

Windows 10 and 11, 64-bit. Free and open source. No administrator rights needed.

Windows may show “Windows protected your PC” because the installer is not code signed yet. Choose More info, then Run anyway.

## Related pages

- [How to install Deadlock mods](https://sigilmods.app/how-to-install-deadlock-mods): The addons folder, VPK names and gameinfo.gi
- [Launching](https://sigilmods.app/launch): Modded or vanilla, with checks first
- [Installed mods](https://sigilmods.app/installed): On and off, load order and overlaps

---

Sigil is a fan project and is not affiliated with Valve. Deadlock is a trademark of Valve Corporation. Mods come from GameBanana and belong to their creators.
