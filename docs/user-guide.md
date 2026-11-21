# Sigil player guide

Status: active, 2026-11-21. Describes Sigil 0.1.22. Parts that have only been checked on stand-ins
(the 3D viewer, the live match bridge, relay region blocking and hosting a server) are marked as
new and not yet checked with a real game.

Sigil is a desktop app for playing Deadlock with mods. It finds your game, installs and organises
mods, shows what they replace, helps you switch whole setups, previews hero skins in 3D, edits your
crosshair and console settings, reads your match history, and can start the game for you.

## Install and first run

1. Download the build for your computer from Sigil's GitHub releases page. Windows and macOS builds
   are the main ones. A Linux AppImage and a Debian package are also built, but they have not been
   checked on a Linux desktop yet.
2. Unzip it where you keep apps and start Sigil. On Windows and macOS there is no separate
   installer and no browser window inside the app.
3. The first time you start, a short setup asks six things:
   - **Find Deadlock.** Sigil looks for the game and shows the folder it found. Use Choose if it
     picked the wrong one.
   - **Let the game load mods.** Deadlock only reads the addons folder when `gameinfo.gi` points at
     it. Press the fix. Sigil keeps a backup of the file first.
   - **Your connection.** Choose whether to use a proxy, or work offline.
   - **Mods you already have.** Sigil counts what is in the addons folder and offers to open
     Installed.
   - **Your autoexec.** Create `autoexec.cfg` if you do not have one.
   - **Updates and privacy.** Choose whether Sigil may check for updates, and what optional
     information you are willing to share. Everything is off until you agree.

You can skip setup. To see it again, open Settings > General > Setup and tips > Run setup.

On later starts Sigil checks the game folder, `gameinfo.gi` and your mods. If a Deadlock update
reset the file, a banner offers to fix it.

## The window

The sidebar holds the pages: Installed, Browse, Overlap, Profiles, Locker, Crosshair, Autoexec,
Performance, Stats, Servers and Launch. Settings and the agent workbench sit at the foot. Pages you
do not use can be turned off in Settings > General > Pages, including a Mods only mode.

The top bar has search for the current page. The status bar shows whether the game was found,
whether the mod catalog is syncing, downloads, an app update, the active profile and the preview
volume. Cmd or Ctrl+B collapses the sidebar. Cmd or Ctrl+comma opens Settings. Cmd or Ctrl+plus,
minus and 0 change the interface size. Alt+Enter or F11 switches full screen. The mouse back and
forward buttons walk back and forward through pages, mod pages and creator pages.

## Browse

Browse reads GameBanana live. The tabs are Mods, Sounds, WiPs and All. Pick a category, a
subcategory or a hero on the left, or search by name. A hero pick also finds that hero's sounds
and HUD work. Filters cover mature content, outdated mods, whether a mod is installed, featured
mods and a category match mode. You can sort by newest, updated, most liked, viewed, downloaded,
discussed, latest comment or A to Z, and limit the results to a date window. Active filters appear
as removable chips.

The landing page has a top-this-week strip, trending rails, a Browse by hero rail and featured
picks. Favorites are saved on this computer. Hidden mods and creators are kept in Hidden items,
with the date you hid them, a picture and a note.

Opening a mod shows its pictures at full size, its description, files, changelog, comments,
requirements, videos and creator. Use the arrow keys to walk to the previous or next mod. A creator
name opens a creator page with their uploads and totals.

To install:

1. Press Install on the card or open the mod and choose a file.
2. Sigil downloads the archive, lists the mod files inside and checks what it is about to write.
   If the mod runs scripts or programs, the review says so.
3. Choose the file or variant you want, then confirm. Sigil puts it in the first free slot.
4. If Settings > Mods > New mods start turned off is on, the mod arrives switched off. Otherwise it
   lands on, unless the safety check asks first.
5. If the install adds a second skin for a hero who already has one switched on, Sigil asks whether
   to keep both or only the new one.

A mod that is already installed shows Update or Replace instead. Updates keep the same slot and
your on or off choice. The old files go to Sigil's trash or backups.

You can paste a GameBanana link or id into the search to open that mod. You can also paste many
links at once to queue several mods, skip mature or duplicate ones, and save the result as a
profile. Custom mod sources can be added in Settings > Mods > Sources.

## Installed

Installed is the list of every mod in your addons folder. Switch a mod on or off from its card.
Search by name, author or slot. Filter by on or off, by source, hero, category or tag. Sort by load
order, name, hero, install date or size. Cards, small cards and a list view are available, and the
list can be grouped into sections per mod list.

Select several mods to switch them on or off, delete them, group them as one mod, add them to a
list or tag them. A mod's menu also has:

- **Manage files** for a mod with several files, with one switch per file. Turning every file off
  turns the mod off.
- **Load first**, earlier and later to move it in the load order.
- **Reinstall**, **Delete** and **Delete and reinstall**. Deletes move the files to Sigil's trash
  with Undo.
- **Backups** for the copy an update replaced, with Put this version back.
- **View imprint** for the file's fingerprint, its embedded addon information and where it came
  from.
- **Check what is inside** and the mod safety list (see below).
- **Edit local mod** for the name, a picture and the mature flag.
- **Add to a Locker category** and tags.
- **Find which mod this is** for an unknown file in Analyze.

Installed checks for updates when you open it, at most every six hours. An update chip appears on
each mod with a newer file, and Update all can take all mods, only enabled ones or only disabled
ones. A download can be cancelled from its chip or with Cancel all. Ignore an update from its menu
and ask for it again later from the same place.

The Mod safety notice appears when a mod runs programs, loose scripts or script code that reaches
the network. More > Mod safety lists every flagged mod as Review or Allowed, with the reasons and
files. Check installed mods scans the folder again. Allow N script mods trusts the versions you
choose. Disallow switches a mod off and takes the trust back. Sigil never switches an approved mod
off at startup without asking; an enabled mod nobody allowed is listed with Keep it off.

More > Analyze the addons folder lists what each addon folder holds, leftovers, slots with no
library entry, doubled slots and missing or read-only files. It can search GameBanana for an
unknown file, remove leftovers to the trash, and reinstall a mod whose files are missing. More >
Find a broken mod starts a halving search across launches so you can find the mod that breaks the
game.

## Load order and overlaps

Deadlock loads mods from numbered files. A lower number loads first and wins when two mods replace
the same file. Sigil uses slots 01 to 99 in the normal addons folder and adds more folders for
slots 100 to 990. Drag a card to reorder, or use Load first, earlier and later in its menu. Sigil
checks the whole move and puts everything back if a rename fails.

Overlap shows every pair of mods that touch the same files and which one wins. Pick a losing mod to
swap the two slot numbers, or turn one off. Ignore a pair, a file or a whole mod, and restore it
from the bottom of the page. Ignores can be kept per profile. Cards carry a quiet mark when they
share files with another mod. The sidebar shows the number of overlaps.

## Profiles

A profile is a saved set of switched-on mods, plus optional autoexec lines, crosshair, mod
favourites and the Locker's shuffle settings. Save what you have now, then switch to it in one
step. Profiles can be renamed, updated, shared as a code or a file, and imported. Import looks up
each mod and shows which ones are missing or need a different file. A restore can replace a
profile or merge into it. Snapshot recovery points are taken before applies, updates and imports.
Undo after applying a profile puts the earlier setup back.

A profile can keep its mods in its own addons folder instead of the normal one. That is useful for
trying a setup without affecting the others. The titlebar has a quick switch between saved
profiles.

## Locker

The Locker shows every hero with the skins and other mods you have for them. Each hero page lists
their skins in load order. Equip switches the others off. If several skins per hero is on, Equip
adds the skin instead and the hero can shuffle on launch. The shuffle can be set per hero and per
skin, with a Shuffle now button and a pool of skins.

Hero pages also have sections for weapons and voice, ability sounds, the hero's own game sounds,
the game's art, and an in-game card editor. General covers everything that is not tied to one hero:
soul containers, spirit urns, patrons, the Sinner's Sacrifice vault, boxes, the Hideout, icons,
HUD, effects, animations, maps, announcer and killstreak music.

Other Locker tools:

- **In-game card** puts your own image on a hero's card art, or uses the hero page's 3D view.
- **Game sounds** plays the game's own sound events and can replace one with your audio, built as
  a normal mod that arrives switched off.
- **Game art** previews the game's textures and can replace a hero or ability icon.
- **Cursors** imports a cursor pack from a folder or archive, applies it, restores the stock
  cursors and reports when Steam put the stock files back. You can also create a pack from your own
  images with a guessed click point that you nudge with arrows. There is no free cropper yet.

## The 3D viewer

Preview in 3D appears in a mod's details, in Installed's card menu and file list, on a Locker skin,
and as a full screen Hideout. The viewer draws the hero with the mod on and, with Split or the
before and after switch, against the hero without it. It has controls for animation clips,
lighting, background, cloth, particles, exposure and wireframe, plus orbit, zoom, capture and full
screen. A preview waits until the hero and room are ready and shows a loading state instead of a
half-built scene.

You can use the posed view as the mod's cover or as in-game card art. The Hideout shows the menu
map around the hero with your mods applied. This part has been checked on stand-ins only, not yet
with a real Deadlock install.

## Crosshair and autoexec

The Crosshair page draws the reticle at true scale over a game scene while you edit it. Set lines,
gap, dot, colour, outline colour and thickness, and watch the preview update. Presets can be saved,
renamed and applied. Copy code shares a crosshair, and Import takes a share code, a console line or
a pasted config. Apply writes the commands into a marked block in `autoexec.cfg` and keeps a
backup. Remove from autoexec clears the block. Import from game reads the crosshair Deadlock keeps
in its own hero settings. Apply to hero settings writes it there. Copy commands puts the console
lines on your clipboard. Pin window keeps the crosshair window above other windows.

The Autoexec page edits `autoexec.cfg` directly. Sigil's own blocks are marked and its lines are
kept separate from yours. The command library is searchable by category and inserts a command with
one press. Save keeps a backup first. You can open the file in your chosen editor. Game > Edit
opens it too. Steam launch options are saved separately and written at each modded launch.

## Performance

Performance applies community configurations to `gameinfo.gi`. Pick a config and a version, then
apply it. Your own edits and the author's optional switches are kept as separate overrides. Turn it
off to put the stock values back, or restore from a backup.

You can add presets from a public GitHub repository. Sigil reads the repository at a commit, keeps
a copy so the preset never changes under you, and lets you choose a gameinfo file, a video file or
both. Hide built-in presets if you only want your own, and delete a custom preset when you are
done. Apply as written uses a whole file's values in the author's order; comments and sections are
not kept yet. GameBanana configs are available too, with a file review before anything is written.

The convar editor searches the game's console variables, edits values and keeps a history per
config. Video settings can be adopted from the game, forgotten, or restored. Settings files that
arrive inside a mod are shown in a review with what each would change; nothing is copied into the
game until you apply it. Import and export move a config as a file or text.

### Benchmark

Performance > Benchmark runs the same replay section in Deadlock and measures how it draws. It can
compare all your mods against vanilla, measure one mod, find the costliest mod by halving the
enabled set, or compare any two runs. A run records the profile, config, video settings, driver and
game build. Results show average FPS, 1% and 0.1% lows, frame times, a frame time graph, stutters
and the machine. History keeps runs, and a result can be shared as a PNG or exported as JSON or
CSV. If Sigil or the game stops mid-run, the next start offers to restore your setup. This has been
tested on recorded data, not yet with a real game.

## Stats and live match

Stats reads your Deadlock matches from deadlock-api. Open the player panel on Stats and paste your
Steam profile link, SteamID or Deadlock account id, or pick a Steam account Sigil found on this
computer. Keep Player stats on in Settings > Privacy for the reads to happen. The sections are:

- **Overview**: rank, win rate, matches and heroes.
- **Heroes**: your record on each hero.
- **Matches**: the recent list, with Street Brawl and abandoned marks. Street Brawl games are
  tagged and never count as ranked or in win rate.
- **Match detail**: both teams, builds, souls over time, damage and healing, and who abandoned.
- **Items** and **Item meta**: your item win rates and buy times, and everyone's best items.
- **Party**: rates the lineups a party can queue from pair and trio records.
- **Leaderboard**: region and season, with your place marked.
- **Fun run**: a random hero, lane, item build and ability order from one code, to share and race.

Filters scope the sections by ranked or all matches and by season or date range. Tracked players
can be added by hand, and the primary account switches between them. Statlocker links open in your
browser.

While Deadlock runs, Stats > Live match reads the game's console log. It shows the queue, when a
match is found, and, once you are in, the hero, mode, party and match clock. The optional live
bridge is a small HUD mod built from your own game files. It adds both teams with levels, K/D/A,
souls and objective damage, team totals, your souls a minute and a soul lead curve. The last board
stays until the next queue. Settings > Game > Live match turns the bridge on and can rebuild it
after a game update once Deadlock closes. The console log needs `-condebug`, which Sigil can add to
your launch options. This part has been checked on recorded data, not yet with a real match.

## Servers

Servers lists Deadworks servers with ping, players and password state. Search by name or map, and
filter by space, password, region, game mode and map. Every column sorts. Join checks the server and
lists any content you need before it connects. Favorites, recent servers and a direct address dialog
sit on the same page. Maps opens the custom maps page, with How to play and Launch map. Sigil reads
the custom map's console name from its files or its description, writes a marked autoexec block and
starts the game for you.

**Invites.** Host a server or a friend can send a Sigil invite. Join with invite lists every mod the
invite names and what Sigil will do with it: keep it, switch it on, download it or replace an older
file. After you confirm, missing mods download through the normal install path and only the
invite's items are switched on. A Steam join code shows a console line to paste in the game (F7).

**Hosting.** Servers > Host sets up a Deadworks server on a Windows PC. Sigil checks for the .NET
10 runtime, free space, SteamCMD and the pinned server release. SteamCMD opens in its own window so
you sign in to Steam there; Sigil never sees the password. Choose a map and settings, then Start.
The live console shows the server's output and takes commands. Plugins can be turned on or off.
Steam relay mode is the default and needs no router forwarding; it gives you a join code. Direct
mode uses an address and port. An opt-in UPnP switch can open the port automatically. Sigil can
also invite friends with the mods they need, or start a map in your own game so friends join through
Steam with no setup. Hosting has been checked in test scenes, not yet on a Windows PC.

**Regions.** Servers > Regions lists Valve's relay locations grouped by region, with ping and
health. Tick locations to block them, or use Block all and Allow all. Apply writes Windows Firewall
rules for Deadlock only; Reset removes them. Presets can be saved, loaded and deleted. Where I play
loads the locations where you had no recent matches, using your match history if player stats are
on. Search by city, country or id. While a region is blocked, the status bar says so on every page.
macOS and Linux can list and ping locations but cannot block them. This has not been run on a
Windows PC yet.

## Launch and fixing a game that won't start

Launch has Launch modded and Launch vanilla. Modded applies your mods, fixes `gameinfo.gi` if needed,
checks overlaps and updates, and can ask about map mods first. Vanilla moves your mods aside, starts
the game without them and puts them back later. A checklist above the buttons shows anything that
needs a fix.

Before a modded launch Sigil checks that enabled mods still have files. Stop game closes a running
Deadlock. If the game crashes, Sigil reads the crash dump and the console around it and names the
mod file that was loading, when it can. It may offer to switch that mod off. If a Deadlock update
resets `gameinfo.gi`, a banner on every page offers Fix, which keeps mounts another tool added.

If the game will not start:

1. Read the banner or the Launch checklist and fix what it names.
2. Open Installed > More > Analyze the addons folder. Remove leftovers, reinstall missing files and
   clear read-only files.
3. Run Installed > More > Mod safety > Check installed mods, and turn off anything you do not
   trust.
4. Use Installed > More > Find a broken mod. Start the game each time Sigil asks, and say whether it
   worked. Sigil halves the enabled set until it finds the culprit.
5. Turn off mods with outdated data from the compatibility review, or run Repair, Fix all and
   Revert there.
6. If the game still fails with every mod off, use Launch vanilla. If that fails too, verify
   Deadlock's files in Steam and start the game once, then let Sigil fix `gameinfo.gi` again.

Settings > Game > Check for crashes after a launch raises a notification when a new crash dump
appears. Game > While the game runs can take the addons folder out of `gameinfo.gi` when Deadlock
closes, and every switch or delete is refused while the game runs.

## Updates

Sigil checks GitHub releases. When an update is out, a banner appears over every page with Install
or View update. The download shows bytes and a percentage. Restart to update installs the signed
update, and a failed start rolls back to the version that worked. Settings > About and updates has
Check now, the current version, release notes and the release channel (stable or nightly). Going
back explains how to return to an earlier release. If Sigil was installed by winget, Scoop,
Chocolatey, Homebrew, Nix, Snap, Flatpak or a system package, the page says updates come from that
manager and turns the built-in installer off. What's new shows once after an update.

## Privacy and telemetry

Sigil has no account and no third-party analytics. It contacts only the services it needs:

- GameBanana, for browsing, mod pages, files and updates.
- GitHub, for Sigil updates, performance configs and the contributor list.
- Steam, for the official Deadlock patch notes and Valve's relay list.
- deadlock-api, for Stats, match data sharing you turn on, crosshair values and relay match counts.
- Deadworks, when you use Servers.
- A model provider, only when you chat with the agent, using your own key.
- A mod source you added, only for that source.
- Discord, over the app on your PC, when Rich Presence is on.
- Sigil's own telemetry service, only for the opt-ins you agree to.

Settings > Network > Offline mode stops Sigil's own catalog, download and service requests. Chatting
with the agent still uses your model provider, because that is your own connection. Settings >
Privacy has one switch per kind: match data sharing (sends match salts to deadlock-api), player
stats, usage events, crash reports and unknown-mod lookup. Each is off until you agree. Usage
events carry no mod names, search text or file names. Crash reports and diagnostics have your user
name, paths, keys, emails and Steam ids removed on your PC before sending. Diagnostics are sent only
when you press Send diagnostics, after a preview. Keys live in your operating system's keychain.
Sigil does not read your Steam login. Settings > Privacy lists every host inline, and About carries
the third-party notices.

## Sending feedback

Every error toast and inline error box has Send feedback. It shows the scrubbed details, takes a
note, and can send them with a reference code, copy them, or open a GitHub issue with them filled
in. Settings > Logs and support has the same tools plus:

- **Diagnostic report**: a summary of your setup, with an option to include the full log.
- **Crash dumps**: refresh, read details, show the newest file and attach a crash to a report.
- **Ask Sigil for help**: puts the masked logs into Sigil's own agent, and nothing is sent until
  you press Send.
- **Open an issue**: opens Sigil's GitHub issue page.

Tell us what you were doing, what you expected and what happened. A screenshot or the crash's
reference code helps most.
