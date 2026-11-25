# Sigil questions and answers

> Answers about Sigil, the free Deadlock mod manager: cost, safety, where mods come from, updates, Windows warnings, data it sends, files, uninstalling and bugs.

## Is Sigil free?

Yes. Sigil is free and open source under the MIT licence, and the code is on GitHub.

## Is it safe? Can mods get me banned?

Sigil puts mod files in the game's addons folder, the same place a manual install does, and edits gameinfo.gi so the game reads that folder, keeping a backup first. It cannot speak for how Valve treats mods. Launch vanilla starts the game with your mods set aside.

## Where do the mods come from?

From GameBanana. Sigil browses and downloads them from there, and every mod belongs to its creator.

## My mods stopped working after a Deadlock update. What now?

A game update or a Steam file check resets gameinfo.gi, so the game stops loading your mods. Sigil shows a banner with a Fix button. Mods made for an older build may also need an update from their creator; the guide [Deadlock mods not working after an update](https://sigilmods.vercel.app/deadlock-mods-not-working-after-update) explains both cases.

## Does it work on Mac, Linux or Steam Deck?

The download on this site is for Windows 10 and 11, 64-bit. There is no Mac, Linux or Steam Deck release here.

## Why does Windows warn me when I run the installer?

The installer is not code signed yet, so Windows may show "Windows protected your PC". Choose More info, then Run anyway.

## What does Sigil send over the internet?

It talks to GameBanana for mods and, if you allow it, to GitHub for app updates. Usage events, crash reports and product data are each off until you agree, and there is no third-party analytics. The [privacy page](https://sigilmods.vercel.app/privacy) lists every host.

## Do I need an account?

No. Sigil has no account and no sign-in. Your favourites, profiles and settings are kept on your PC.

## Can I use it with mods I installed by hand or with another manager?

Yes. Installed lists every mod in the game's addons folder, however it got there, and the gameinfo.gi fix keeps search paths another tool added. Installed, More, Analyze the addons folder shows leftovers, doubled slots and files it cannot match to a mod.

## Where are my mods, settings and backups?

Mods live in the game's own folder, game/citadel/addons, plus sigil-addons2 to sigil-addons10 next to it once you have more than 99. Sigil's settings, backups, trash and the mods Launch vanilla set aside are in %APPDATA%\DeadlockAgentHarness.

## How does Sigil update itself?

If you allow update checks, Sigil looks at its GitHub releases and shows a banner when a new version is out. Install downloads it and Restart to update puts it in place. If the new version fails to start, Sigil goes back to the one that worked.

## How do I uninstall it?

Open Windows Settings, then Apps, then Installed apps, and uninstall Sigil. Your mods stay in the game's addons folder and Sigil's settings folder is left in place. If you used Launch vanilla, check that the Game page shows the vanilla stash as empty first, so your mods are back where the game expects them.

## Where do I report a bug?

Every error message in Sigil has Send feedback, which shows the scrubbed details and can send them with a reference code or open a GitHub issue. You can also open an issue on [GitHub](https://github.com/superuserkalo/sigil/issues) directly.

## Download Sigil

[Download Sigil for Windows (installer, .exe)](https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe)

Windows 10 and 11, 64-bit. Free and open source. No administrator rights needed.

Windows may show “Windows protected your PC” because the installer is not code signed yet. Choose More info, then Run anyway.

## Related pages

- [Privacy and safety](https://sigilmods.vercel.app/privacy): What it sends, and how it checks mods
- [Mods not working after an update](https://sigilmods.vercel.app/deadlock-mods-not-working-after-update): gameinfo.gi resets and moved game files
- [How to install Deadlock mods](https://sigilmods.vercel.app/how-to-install-deadlock-mods): The addons folder, VPK names and gameinfo.gi

---

Sigil is a fan project and is not affiliated with Valve. Deadlock is a trademark of Valve Corporation. Mods come from GameBanana and belong to their creators.
