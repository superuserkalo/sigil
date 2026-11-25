# Sigil: a free, open-source Deadlock mod manager

> Sigil is a free, open-source Deadlock mod manager. It browses and installs mods from GameBanana, searches the catalog on your PC, keeps load order and overlapping mods in view, fixes gameinfo.gi after game updates, and launches the game modded or vanilla. It also has a Locker for skins per hero, a crosshair editor with a community library, profiles you can share, an autoexec editor, performance configs and match stats. Nothing about the player leaves the PC without their yes.

Install Deadlock mods and keep them working. Sigil is a free open-source Deadlock mod manager.

- [Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)
- [Download Sigil for Linux (AppImage, x86_64)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-linux-x86_64.AppImage)
- [Download Sigil for macOS (disk image, .dmg)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-macos.dmg)

Source code: https://github.com/superuserkalo/sigil

## See what is on and which wins

Every mod in your addons folder sits in one list, including the ones you put there by hand. Drag a card to set the load order, and Overlap shows each pair of mods that replace the same files and which one the game will use. After a game update, Sigil lists the mods that ship paths the game moved and offers a repair for each.

[More about load order and overlaps](https://sigilmods.app/installed)

## Browse and install from GameBanana

Browse GameBanana inside Sigil and install with one press. Search runs on your PC, forgives typos and knows what a mod is based on, so a series name finds skins whose title never mentions it. Sigil lists the files in an archive before it writes them, and new installs land switched off until you turn them on.

[More about browsing and search](https://sigilmods.app/browse)

## Trade looks for frames, and back

Apply community performance configs, grouped from Light to Potato, and turn one off to get the stock values back. Your own edits stay separate, so a new version of a config keeps them, and the convar editor covers everything else. When a game update resets gameinfo.gi, Sigil notices and offers to fix it again.

[More about performance configs](https://sigilmods.app/performance)

## Your rank, heroes and matches

Turn on Player stats and Sigil reads your matches from deadlock-api: rank, win rate, your record on each hero, match details with builds and souls over time, item win rates and the leaderboard. Filters scope it to ranked games and a season. Nothing is fetched until you switch it on.

[More about match stats](https://sigilmods.app/stats)

## Everything Sigil does

### Mods

- [Browse and search](https://sigilmods.app/browse): Find and install mods from GameBanana
- [Installed mods](https://sigilmods.app/installed): On and off, load order and overlaps
- [Profiles and packs](https://sigilmods.app/profiles): Save a setup, switch, share as a code

### Play

- [Launching](https://sigilmods.app/launch): Modded or vanilla, with checks first
- [Performance](https://sigilmods.app/performance): Community configs, on and off

### Customise

- [Locker](https://sigilmods.app/locker): Skins per hero, equip and shuffle
- [Crosshair](https://sigilmods.app/crosshair): Edit at true scale, share and import
- [Autoexec](https://sigilmods.app/autoexec): autoexec.cfg, commands and launch options

### Insight

- [Stats](https://sigilmods.app/stats): Rank, heroes, matches and items
- [Privacy and safety](https://sigilmods.app/privacy): What it sends, and how it checks mods

### Guides

- [How to install Deadlock mods](https://sigilmods.app/how-to-install-deadlock-mods): The addons folder, VPK names and gameinfo.gi
- [Mods not working after an update](https://sigilmods.app/deadlock-mods-not-working-after-update): gameinfo.gi resets and moved game files
- [Questions and answers](https://sigilmods.app/faq): Cost, safety, updates, data and more
- [Contact and support](https://sigilmods.app/contact): Report a bug, ask a question, suggest a change

## How it keeps your game working

### Checks before it installs

Sigil lists the files in an archive before writing them, and says so when a mod contains programs or scripts.

### Waits for the game

While Deadlock runs, turning off or deleting a mod waits until the game closes.

### Keeps a way back

Deleted mods go to Sigil's trash with Undo, and gameinfo.gi is backed up before every fix.

### Asks before it shares

Nothing about you leaves your PC without your yes. No account, and no third-party analytics in the app.

## Questions

### Is Sigil free?

Yes. Sigil is free and open source under the MIT licence, and the code is on GitHub.

### Is it safe? Can mods get me banned?

Sigil puts mod files in the game's addons folder, the same place a manual install does, and edits gameinfo.gi so the game reads that folder, keeping a backup first. It cannot speak for how Valve treats mods. Launch vanilla starts the game with your mods set aside.

### Where do the mods come from?

From GameBanana. Sigil browses and downloads them from there, and every mod belongs to its creator.

### My mods stopped working after a Deadlock update. What now?

A game update or a Steam file check resets gameinfo.gi, so the game stops loading your mods. Sigil shows a banner with a Fix button. Mods made for an older build may also need an update from their creator; the guide [Deadlock mods not working after an update](https://sigilmods.app/deadlock-mods-not-working-after-update) explains both cases.

### Does it work on Linux, Mac or Steam Deck?

There are downloads for Windows (an installer for Windows 10 and 11, 64-bit), Linux (an AppImage for x86_64) and macOS (a disk image). Windows is where Sigil has been used most; the Linux and macOS builds are newer, so please report anything that does not work. Deadlock itself runs on Windows, and on Linux through Proton. There is no separate Steam Deck build.

### Why does Windows warn me when I run the installer?

The installer is not code signed yet, so Windows may show "Windows protected your PC". Choose More info, then Run anyway.

### What does Sigil send over the internet?

It talks to GameBanana for mods and, if you allow it, to GitHub for app updates. Usage events, crash reports and product data are each off until you agree, and there is no third-party analytics. The [privacy page](https://sigilmods.app/privacy) lists every host.

[All questions and answers](https://sigilmods.app/faq)

## Get Sigil

- [Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)
- [Download Sigil for Linux (AppImage, x86_64)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-linux-x86_64.AppImage)
- [Download Sigil for macOS (disk image, .dmg)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-macos.dmg)

A free, open-source mod manager for Deadlock.

---

Sigil is a fan project and is not affiliated with Valve. Deadlock is a trademark of Valve Corporation. Mods come from GameBanana and belong to their creators.
