# Privacy and mod safety in Sigil

> What Sigil sends and when, the switches that keep it off until you agree, and how it checks mods for programs and scripts before they run.

Sigil has no account and no third-party analytics. It talks only to the services a feature needs, and everything that would send data about you is off until you agree.

![Settings, Privacy: one switch per kind of data, with What Sigil sends, What Sigil shared and Delete my data.](https://sigilmods.app/img/privacy.webp)

## What Sigil talks to

- GameBanana, for browsing, mod pages, files and updates.
- GitHub, for Sigil updates, performance configs and the contributor list.
- Steam, for the official Deadlock patch notes and Valve's relay list.
- deadlock-api, for Stats when Player stats is on, match data sharing you turn on, and crosshair values.
- Sigil's own service, for the community crosshair library, the search index and the opt-ins below.

Offline mode in Settings, Network stops Sigil's own catalog, download and service requests.

## Switches, all off until you agree

Settings, Privacy has one switch per kind: match data sharing, player stats, usage events, crash reports and unknown-mod lookup. Usage events carry no mod names, search text or file names. Crash reports and diagnostics have your user name, paths, keys, emails and Steam ids removed on your PC before sending, and diagnostics are only sent when you press Send after a preview.

Help build Sigil is asked once during setup with Share and Not now. With Share, Sigil sends which GameBanana mods you install or turn on, your Browse filters and short search text, your most played heroes and rank tier, benchmark frame rates with your GPU and CPU model, and which pages you use. It never sends your Steam id, files or paths, local mods, keys or IP address. What Sigil shared lists the last records sent, and Delete my data removes them. The [privacy notes](https://github.com/superuserkalo/sigil/blob/main/docs/privacy.md) list every field.

## Mod safety

Before an install, Sigil lists the files in the archive and checks what it is about to write. A mod that contains programs, loose scripts or script code that reaches the network is flagged. Such a mod waits switched off until you review it, and Installed, More, Mod safety lists every flagged mod with its reasons and files. Sigil never switches off a mod you approved without asking.

![A mod safety note: 3 of 214 files are worth a look, including a program and a script that reaches the network.](https://sigilmods.app/img/safety.webp)

Deadlock never runs a program shipped inside a mod, so a mod has no reason to include one. The warning is a prompt to look, not a block.

## Download Sigil

- [Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)
- [Download Sigil for Linux (AppImage, x86_64)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-linux-x86_64.AppImage)
- [Download Sigil for macOS (disk image, .dmg)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-macos.dmg)

A free, open-source mod manager for Deadlock.

On Windows you may see “Windows protected your PC” because the installer is not code signed yet. Choose More info, then Run anyway.

## Related pages

- [Questions and answers](https://sigilmods.app/faq): Cost, safety, updates, data and more
- [Installed mods](https://sigilmods.app/installed): On and off, load order and overlaps
- [Browse and search](https://sigilmods.app/browse): Find and install mods from GameBanana

---

Sigil is a fan project and is not affiliated with Valve. Deadlock is a trademark of Valve Corporation. Mods come from GameBanana and belong to their creators.
