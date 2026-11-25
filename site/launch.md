# Launch Deadlock with or without mods

> Launch Deadlock modded or vanilla from Sigil. It checks your mods first, fixes gameinfo.gi after updates, and puts mods aside for a vanilla game.

Sigil has two buttons at the foot of the sidebar: Launch modded and Launch vanilla. Both start Deadlock through Steam.

![The Before you launch checklist: mods will not load, two mods replace the same files, and mods to check after the update.](https://sigilmods.app/img/launch.webp)

## Launch modded

Before a modded start, Sigil checks that your enabled mods still have their files, that `gameinfo.gi` still tells the game to load the addons folder, which mods overlap and which mods need a look after a game update. Anything that needs attention appears in a short checklist with a button to fix it, and you can still choose Launch anyway.

Sigil also writes the Steam launch options you saved on the Autoexec page, and can ask about map mods first.

## Launch vanilla

Launch vanilla moves your mods aside and starts the game without them. Use it to check whether a problem comes from a mod, or to play without mods for a while. Your mods come back by themselves when Deadlock closes, or the next time Sigil starts if you closed it meanwhile, so there is no Restore button to remember. A file that is already in the addons folder is kept, never overwritten. If they cannot be put back, Sigil says so with Try again and shows a banner on Installed until it works.

## The gameinfo.gi fix

Deadlock only reads the addons folder when `game/citadel/gameinfo.gi` lists it in its search paths. Every game update and every Steam file verification puts Valve's own file back, and your mods quietly stop loading. When that happens Sigil shows a banner on every page with a Fix button. The fix adds back the line that loads mods, keeps a copy of the file first and leaves alone the search paths another tool added.

![A banner over the Installed page: mods are off because a game update or a Steam file check reset gameinfo.gi, with a Fix button.](https://sigilmods.app/img/gameinfo-banner.webp)

## When the game will not start

While Deadlock runs, Launch modded becomes Stop Deadlock, which closes the game after you confirm. If the game crashes, Sigil reads the crash dump and the console log around it and, when it can, names the mod file that was loading. The last crash, with Turn off for that mod and Show dump, is in Settings, Logs and support. Installed, More, Find a broken mod halves your enabled mods across launches until it finds the one that breaks the game. If the game still fails with every mod off, verify Deadlock's files in Steam, start it once, and let Sigil fix `gameinfo.gi` again.

Launching and restoring on a Windows install is the newest part of Sigil; if something goes wrong, use Send feedback on the error so it can be fixed.

## Download Sigil

- [Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)
- [Download Sigil for Linux (AppImage, x86_64)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-linux-x86_64.AppImage)
- [Download Sigil for macOS (disk image, .dmg)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-macos.dmg)

Free and open source, for Windows, Linux and macOS. The Windows installer needs no administrator rights.

On Windows you may see “Windows protected your PC” because the installer is not code signed yet. Choose More info, then Run anyway.

## Related pages

- [Installed mods](https://sigilmods.app/installed): On and off, load order and overlaps
- [Performance](https://sigilmods.app/performance): Community configs, on and off
- [Mods not working after an update](https://sigilmods.app/deadlock-mods-not-working-after-update): gameinfo.gi resets and moved game files

---

Sigil is a fan project and is not affiliated with Valve. Deadlock is a trademark of Valve Corporation. Mods come from GameBanana and belong to their creators.
