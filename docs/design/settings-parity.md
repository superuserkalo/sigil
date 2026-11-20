# Settings parity: Grimoire and Deadlock Mod Manager

Status: audit done 2026-11-17; smalls round 2026-11-19 (rows G47, G99, G100, D129, D194 built); parity round built 2026-11-17; wave 3 rows updated 2026-11-18 (pages, preview volume, map mods, fix before launch, Remote Install backlogged); wave 4 UI B rows updated 2026-11-18 (SOCKS5, checklist, catalog, logs, AI help, launch options, vanilla, update marks, channel notes, seasons). Every settings item in both apps, from their source, with what Sigil does about it.

Sources: Grimoire (`vendor/grimoire`, MIT) `src/pages/Settings.tsx`, `src/components/settings/**`, `electron/main/services/settings.ts`; Deadlock Mod Manager (`vendor/dmm`, GPL-3.0, behaviour only) `apps/desktop/src/pages/settings.tsx`, `apps/desktop/src/components/settings/*.tsx`, `plugins/themes`.

Status words: **Had it** (Sigil had it before this round), **Built now**, **Built, hook pending** (the setting works and persists; a page owned elsewhere still has to read it), **Elsewhere in Sigil** (a page outside Settings owns it), **Needs a backend** (see `settings-social.md`), **Declined (rule)** (conflicts with AGENTS.md hard rules or the house style, reason given), **Not built yet** (buildable, reason given).

## Counts

| | Grimoire | Deadlock Mod Manager |
| --- | --- | --- |
| Items | 159 | 232 |
| Had it | 27 | 42 |
| Built now | 84 | 117 |
| Built, hook pending | 0 | 3 |
| Elsewhere in Sigil | 35 | 27 |
| Needs a backend | 8 | 6 |
| Declined (rule) | 5 | 38 |
| Not built yet | 1 | 2 |
| **Covered** (had, built, elsewhere) | **141** | **186** |
| Covered or deliberately declined | 146 | 224 |

The parity round (2026-11-17) built every row that was **Not built yet** except the two 3D model previews (no 3D renderer in Rune yet) and the two update rows the updater owns. **Built, hook pending** means the setting, its storage and its getter work, and one line in a page another agent owns still has to read it; `.scratch/sigil-redesign/` lists the lines.

## Sigil's categories

Grouped by task, thirteen categories in three groups, a search field over all of them:

- **Game and mods:** Game (folder, gameinfo.gi, addons, autoexec, Steam launch options), Backups, Mods (installing, Browse order, mature content, hidden mods and creators).
- **Sigil:** General (language, dates, start with the computer, start minimized, open on, launching), Appearance (theme, accent, your colours, size, transparent sidebar), Integrations (Discord, Steam, GameBanana, sharing), Privacy (match data sharing, optional features, every host), Models and sign-in.
- **Advanced:** Network (offline mode, background checks, proxy), Logs and support (log level and file, crash dumps, diagnostic report, issue link), Storage (folders, caches, leftover archives, maintenance, reset), Experimental (flags), About and updates (release channel, credits).

## Grimoire

| # | Area | Item | What it does | Sigil | Where in Sigil, or why not |
| --- | --- | --- | --- | --- | --- |
| 1 | Header | Buy me a coffee | Opens the author's Ko-fi page | Declined (rule) | Sigil has no donation page; About links the source |
| 2 | Nav | Search settings | Filters settings across categories | Built now | Settings, search field over the category list; wave 4: the caret shows only once the field is focused (issue #2), Cmd or Ctrl+F focuses it, Enter opens the first match |
| 3 | Nav | Clear search | Clears the query | Built now | Settings, x in the search field and Escape |
| 4 | Game | Deadlock installation path | Type or keep the game path | Had it | Game > Deadlock folder |
| 5 | Game | Valid / invalid badge | Path validation | Had it | Game > Deadlock folder line |
| 6 | Game | Auto-detect | Finds the install | Had it | Game > Detect |
| 7 | Game | Browse | Folder picker | Had it | Game > Choose |
| 8 | Game | gameinfo.gi status | Is the file wired for mods | Had it | Game > gameinfo.gi |
| 9 | Game | Open gameinfo.gi | Opens it in an editor | Built now | Game > gameinfo.gi pen button |
| 10 | Game | Fix configuration | Rewrites search paths | Had it | Game > gameinfo.gi Repair (confirm, backup) |
| 11 | Game | Open game folder | Reveals the folder | Had it | Game > folder button |
| 12 | Game | Autoexec status | Present or missing | Had it | Game > autoexec.cfg |
| 13 | Game | Create autoexec file | Creates it when missing | Elsewhere in Sigil | Autoexec page creates it on first save; Game > Edit opens it |
| 14 | Performance | On / off | Apply community config | Elsewhere in Sigil | Performance page |
| 15 | Performance | Preset | Pick a config tier | Elsewhere in Sigil | Performance page |
| 16 | Performance | Shadows | Shadows variant | Elsewhere in Sigil | Performance page |
| 17 | Performance | Notice actions | Restore, update, re-add | Elsewhere in Sigil | Performance page |
| 18 | Performance | Pinned value switch | Toggle one override | Elsewhere in Sigil | Performance page |
| 19 | Performance | Edit pin | Edit an override | Elsewhere in Sigil | Performance page |
| 20 | Performance | Pin value field, Save, Cancel | Edit an override | Elsewhere in Sigil | Performance page |
| 21 | Performance | Remove pin | Remove an override | Elsewhere in Sigil | Performance page |
| 22 | Performance | Convar | New pin name | Elsewhere in Sigil | Performance page |
| 23 | Performance | Value | New pin value | Elsewhere in Sigil | Performance page |
| 24 | Performance | Pin value | Add override | Elsewhere in Sigil | Performance page |
| 25 | Performance | Advanced | Expand advanced | Elsewhere in Sigil | Performance page |
| 26 | Performance | Version | Pin a config release | Elsewhere in Sigil | Performance page |
| 27 | Performance | Always use newest | Track updates | Elsewhere in Sigil | Performance page |
| 28 | Performance | Put them back | Re-apply video settings | Elsewhere in Sigil | Performance page |
| 29 | Performance | Author gameplay convars | Opt into gameplay convars | Elsewhere in Sigil | Performance page (owner exception, per convar) |
| 30 | Performance | Edit the file by hand | Open in editor | Elsewhere in Sigil | Performance page |
| 31 | Performance | Change editor | Pick an editor | Built now | General > Files > Text editor: system default, editors found on this computer, or Browse |
| 32 | Performance | Discard overrides | Reset hand edits | Elsewhere in Sigil | Performance page |
| 33 | Performance | Source / Mods link | Author's mods in Browse | Elsewhere in Sigil | Performance page |
| 34 | Appearance | Theme | Dark, OLED, light | Had it | Appearance > Theme (six themes) |
| 35 | Appearance | Accent presets | Accent colour | Had it | Appearance > Accent colour |
| 36 | Appearance | Your colors | Saved custom accents | Built now | Appearance > Your colours |
| 37 | Appearance | Remove your color | Delete a saved accent | Built now | Appearance > Your colours x |
| 38 | Appearance | Custom accent picker | Any hex colour | Had it | Appearance > hex field and Use |
| 39 | Appearance | Background glow | Tint the background | Built now | Appearance > Personalize > Glow (off by default) |
| 40 | Appearance | Your glow | Saved glows | Built now | Appearance > Personalize > Glow colours, your own pairs |
| 41 | Appearance | Remove your glow | Delete a glow | Built now | Appearance > Personalize > Glow colours x |
| 42 | Appearance | Custom glow | Two-colour glow | Built now | Appearance > Personalize > Glow colours, two hex fields |
| 43 | Appearance | Combine launch buttons | One launch button with a swap | Built now | General > One launch button |
| 44 | Appearance | Art: active tab | Art behind the active tab | Built now | Appearance > Personalize > Art > Active tab (accent glow, hero or image) |
| 45 | Appearance | Art: Launch vanilla | Button art | Built now | Appearance > Personalize > Art > Launch vanilla (Grimoire's art, hero or image) |
| 46 | Appearance | Art: Launch modded | Button art | Built now | Appearance > Personalize > Art > Launch modded (Grimoire's art, hero or image) |
| 47 | Appearance | Art: volume bar | Bar art | Built now | Appearance > Personalize > Sidebar art > Volume bar: Off (default), Sigil's art (Grimoire's `preview-volume-bg.jpg`), a hero or your image, with framing; paints behind the status bar's volume control (`personalize::volume_art`) |
| 48 | Appearance | Art source | Default, hero, image | Built now | Appearance > Personalize > Art: Off, default, hero, your image |
| 49 | Appearance | Hero grid | Pick a hero render | Built now | Appearance > Personalize > Art > Hero (chip grid) |
| 50 | Appearance | Adjust framing | Crop the render | Built now | Appearance > Personalize > Art: Across, Down, Zoom, Reset framing |
| 51 | Appearance | Upload image | Custom art | Built now | Appearance > Personalize > Art > Your image (copied and scaled on a worker) |
| 52 | Appearance | Art Cancel / Apply | Save art | Built now | Appearance > Personalize > Art applies live; Reset framing undoes |
| 53 | Appearance | Transparent sidebar | Sidebar on the page background | Built now | Appearance > Transparent sidebar |
| 54 | Appearance | Language | App language | Built now | General > Language (System, English, Deutsch) |
| 55 | Appearance | Date format | MM/DD or DD/MM | Built now | General > Dates (ISO, day first, month first) |
| 56 | Preferences | Expand Locker cards | Locker list expanded | Built now | Mods > Preferences; the Locker opens expanded (`options::locker_expanded()`) |
| 57 | Preferences | Open mod page on multi-file click | Installed click behaviour | Built now | Mods > Preferences; a multi-file card in Installed opens its mod page |
| 58 | Preferences | Confirm before updating a profile | Profile overwrite confirm | Built now | Mods > Preferences; Profiles skips the overwrite confirm when it is off |
| 59 | Preferences | Prompt for optional requirements | Requirements dialog | Built now | Mods > Preferences > Show what a mod needs; Browse's install panel lists GameBanana `_aRequirements` |
| 60 | Preferences | Ignore overlap by default | Hide overlaps | Built now | Mods > Preferences; the Overlap page shows an empty state with Show overlap |
| 61 | Preferences | Keep removed files 24 h | Backup replaced mods | Had it | Every replace keeps a backup; Backups lists it |
| 62 | Preferences | Switch variants | Disable old variant | Built now | Mods > Preferences; Browse installs turn off other files of the same mod (`mod_data::switch_variants`) |
| 63 | Preferences | Enable mods after download | Auto-enable | Had it | Mods > New mods start turned off |
| 64 | Preferences | Match remaining update files | Update pairing | Built now | Mods > Preferences; updates pair the remaining files of a multi-file mod |
| 65 | Preferences | Skip replacement confirmation | No confirm on replace | Built now | Mods > Preferences > Replace without asking; Browse replaces with no confirm |
| 66 | Preferences | Swap Install and Replace | Button order | Built now | Mods > Preferences > Replace first; Browse shows Replace before Install |
| 67 | Preferences | DeadlockForge 1-click | Localhost bridge | Built now | Integrations > DeadlockForge: 127.0.0.1 listener (Grimoire's protocol), every install confirmed in Sigil, off by default |
| 68 | Preferences | Forge listening status | Bridge status | Built now | Integrations > DeadlockForge status line (port or Not listening) |
| 69 | Preferences | Discord Rich Presence | Activity on Discord | Built now | Integrations > Discord (needs the app id) |
| 70 | Privacy | Privacy page link | Privacy doc | Built now | Privacy > Where Sigil connects lists every host inline |
| 71 | Privacy | NSFW Show / Blur / Hide | Mature content | Had it | Mods > Mature content |
| 72 | Privacy | Reveal NSFW on hover | Hover unblur | Built now | Mods > Mature content > Reveal on hover (Browse cards) |
| 73 | Privacy | Always show 18+ tags | Tag visibility | Built now | Mods > Mature content > Always show the 18+ tag (Browse cards) |
| 74 | Privacy | Contribute match data | Salts to deadlock-api | Built now | Privacy > Match data sharing |
| 75 | Privacy | Contribution status | Count, failure | Built now | Privacy > Status and log |
| 76 | Privacy | Hidden creators count | Count | Had it | Mods > Hidden mods and creators |
| 77 | Privacy | Manage hidden creators | Manager | Built now | Mods > one row per hidden creator |
| 78 | Privacy | Show creator | Unhide | Built now | Mods > Show |
| 79 | Privacy | Hidden mods count | Count | Had it | Mods > Hidden mods and creators |
| 80 | Privacy | Manage hidden mods | Manager | Built now | Mods > one row per hidden mod |
| 81 | Privacy | Show mod | Unhide | Built now | Mods > Show, Open on GameBanana |
| 82 | Mod safety | Why review mods | Explainer | Elsewhere in Sigil | Sigil validates every payload before install (validate rules) |
| 83 | Mod safety | Check installed mods | Rescan | Elsewhere in Sigil | Installed and the payload validator |
| 84 | Mod safety | Allow script mods | Trust all | Built now | Mods > Preferences > Allow script mods: off by default, warning dialog; gameplay data and skeletons become review warnings, programs and archives stay refused |
| 85 | Mod safety | Review card | Per-mod trust | Built now | Browse install panel lists each script file as a warning before Install |
| 86 | Mod safety | Scan status | Status | Elsewhere in Sigil | Installed |
| 87 | Social | Sign in with Steam | Grimoire Social login | Needs a backend | Needs a Sigil community service; settings-social.md |
| 88 | Social | Cancel login | Cancel | Needs a backend | With 87 |
| 89 | Social | Account status | Avatar, name | Needs a backend | With 87; Integrations shows the local Steam account |
| 90 | Social | Session only warning | Keychain notice | Needs a backend | With 87 |
| 91 | Social | Sign out | Logout | Needs a backend | With 87 |
| 92 | Social | Delete account | Delete | Needs a backend | With 87 |
| 93 | Social | Dismiss error | Clear banner | Needs a backend | With 87 |
| 94 | Updates | Update channel | Stable or nightly | Built now | About and updates > Release channel, and Going back to an earlier version with the Releases link (wave 4) |
| 95 | Updates | Check automatically | Startup check | Had it | Network > Check for Sigil updates, Privacy > Update check |
| 96 | Updates | Current version | Version and state | Had it | About and updates |
| 97 | Updates | What's new | Release notes | Built now | About and updates > Release notes when newer |
| 98 | Updates | Check / Download / Install | Contextual update button | Had it | Check now; download and install need signed builds (later) |
| 99 | Updates | Download progress | Progress | Built now | About > Updates and the update banner show bytes and percent while Sigil downloads its update (`settings/smalls.rs::progress_line`, `shell_w4::update_banner`) |
| 100 | Updates | Package manager note | Managed installs | Built now | A copy under winget, Scoop, Chocolatey, Homebrew's Caskroom, Nix, Snap, Flatpak or `/usr`/`/opt` (path only, `package_manager.rs`) says updates come from that manager; the installer row and the automatic download are off |
| 101 | Updates | Manual download note | Ad-hoc signed macOS | Had it | Release notes opens the release page |
| 102 | Experimental | Stats dashboard | Flag | Had it | Stats is a main page; Privacy > Player stats turns its network off |
| 103 | Experimental | Crosshair designer | Flag | Had it | Crosshair is a main page |
| 104 | Experimental | Grimoire Social | Flag | Needs a backend | With 87 |
| 105 | Experimental | Fix unknown mods | Match local VPKs | Elsewhere in Sigil | Installed > Identify runs GameBanana name search, and the CRC probe behind its own "Also compare checksums" opt-in (off by default, remembered), C8, wave 4 |
| 106 | Experimental | Imprint installed mods | Embed identity | Built now | Experimental > Tools: Imprint installed mods (Browse and DeadlockForge installs) and Imprint now; writes addoninfo.txt and modinfo.json into the VPK |
| 107 | Experimental | Mod safety | Flag | Elsewhere in Sigil | Always on: payload validator |
| 108 | Experimental | Deadworks servers | Flag | Built now | Experimental > Servers |
| 109 | Experimental | Door Stuck | Foundry flag | Elsewhere in Sigil | Agent page builds mods; Experimental > Agent |
| 110 | Experimental | Hideout 3D models | Flag | Not built yet | Needs a 3D model renderer; Rune has no mesh pipeline yet, so this cannot be built this round |
| 111 | Experimental | Developer mode | Dummy game dir | Elsewhere in Sigil | DAH_GAME_DIR and demo scenes; Game > Choose any folder |
| 112 | Experimental | Dev Deadlock path | Info | Elsewhere in Sigil | Game > Deadlock folder line (Set by DAH_GAME_DIR) |
| 113 | Experimental | Copy mods to dev mode | Copy library | Built now | Experimental > Tools > Copy mods to a developer folder |
| 114 | Maintenance | Cleanup addons folder | Remove leftover archives | Built now | Storage > Leftover archives (confirm) |
| 115 | Maintenance | Reset setup wizard | Wizard again | Built now | General > Setup and tips > Run setup |
| 116 | Maintenance | Automatic server badge | Download server | Built now | Mods > Downloads > Download server (Automatic or a file server) |
| 117 | Maintenance | Selected server | Info | Built now | Mods > Downloads > Download server rows |
| 118 | Maintenance | GameBanana status | Server health | Built now | Mods > Downloads > GameBanana file servers (online count, checked when) |
| 119 | Maintenance | Refresh status | Button | Built now | Mods > Downloads > Refresh status |
| 120 | Maintenance | Test now | Speed test | Built now | Mods > Downloads > Speed test (3 servers, 256 KB each) |
| 121 | Maintenance | Tested on this PC | Results | Built now | Mods > Downloads > Tested here, per server |
| 122 | Maintenance | Use download proxy | Proxy on | Built now | Network > HTTP proxy |
| 123 | Maintenance | Proxy URL | Address | Built now | Network > HTTP proxy field |
| 124 | Maintenance | Save proxy | Save | Built now | Network > Save |
| 125 | Maintenance | Clear saved proxy | Clear | Built now | Network > Clear |
| 126 | Maintenance | Credentials stored securely | Info | Built now | Network > Proxy sign-in: password in the OS keychain, never a file |
| 127 | Maintenance | Cached mods count, last sync | Catalog mirror | Built now | Storage > Mod catalog > Saved catalog: mods saved, last sync, size on disk, Sync now (wave 4) |
| 128 | Maintenance | Wipe cache | Delete mirror | Built now | Storage > Mod catalog > Saved catalog > Wipe (wave 4); Storage > Preview images, Downloads |
| 129 | Maintenance | Sync database | Resync mirror | Built now | Network > Refresh Browse |
| 130 | Maintenance | Preview cache size | Info | Had it | Storage > Preview images |
| 131 | Maintenance | Clear preview cache | Clear | Had it | Storage > Clear |
| 132 | Support | GitHub issues | Link | Built now | Logs and support > Open an issue; wave 4: every error toast and inline error box has Send feedback (scrubbed details, a note, Send with a reference code, Copy details, Open a GitHub issue prefilled) |
| 133 | Support | Join Discord | Link | Declined (rule) | Sigil has no Discord server |
| 134 | Support | Crashes refresh | Rescan dumps | Built now | Logs and support > Crash dumps refresh |
| 135 | Support | Include crash in report | Checkbox | Built now | Logs and support > tick a crash; the report adds its exception and error text |
| 136 | Support | Crash row details | Modal | Built now | Logs and support > crash Details dialog |
| 137 | Support | Show file | Reveal dump | Built now | Logs and support > Show newest |
| 138 | Support | Save original dump | Copy dump | Elsewhere in Sigil | Reveal the folder and copy |
| 139 | Support | Show more crashes | Paging | Built now | Logs and support > Show more (50 at a time) |
| 140 | Support | Bug description | Text | Built now | Logs and support > What went wrong (report and GitHub issue) |
| 141 | Support | Generate report | Build report | Built now | Logs and support > Diagnostic report |
| 142 | Support | Include full log | Checkbox | Built now | Logs and support > Report a problem > Include the full log (up to 5 MB, wave 4) |
| 143 | Support | Verbose mod logging | Checkbox | Built now | Logs and support > Log level Debug |
| 144 | Support | Generated report view | Review | Built now | Copied to the clipboard for review |
| 145 | Support | Copy report | Copy | Built now | Copy report |
| 146 | Support | Open Discord | Link | Declined (rule) | No Discord server |
| 147 | Support | Open GitHub issue | Link | Built now | New issue |
| 148 | Schema | lastDialogDirectory | Remember dialog folder | Built now | File dialogs start in the last folder picked (options.json `last_dialog_dir`) |
| 149 | Schema | hideOutdatedMods | Browse filter | Elsewhere in Sigil | Browse |
| 150 | Schema | steamLaunchOptions | Launch options | Had it | Game > Steam launch options |
| 151 | Schema | activeProfileId | Active profile | Elsewhere in Sigil | Profiles |
| 152 | Schema | hasCompletedSetup | Wizard flag | Built now | options.json `onboarded`; first-run setup |
| 153 | Schema | ignoredConflicts | Overlap ignores | Elsewhere in Sigil | Overlap page |
| 154 | Schema | sidebarHeroHighlight | Legacy art | Built now | Appearance > Personalize > Art > Active tab |
| 155 | Schema | zoomFactor | Zoom | Had it | Appearance > Interface size |
| 156 | Schema | deadworksRelayUrl | Relay override | Elsewhere in Sigil | Servers (env override) |
| 157 | Schema | performanceConfig* | Stored choices | Elsewhere in Sigil | Performance page |
| 158 | Schema | externalEditorPath | Editor | Built now | options.json `editor` |
| 159 | Schema | windowBounds | Window position | Elsewhere in Sigil | Rune restores window state |

## Deadlock Mod Manager

| # | Area | Item | What it does | Sigil | Where in Sigil, or why not |
| --- | --- | --- | --- | --- | --- |
| 1 | All | Settings search | Search and jump | Built now | Settings search |
| 2 | Launch | -condebug toggle | Console log for presence | Built now | Game > Console log (-condebug): written to Steam launch options through the usual confirm |
| 3 | Launch | Disable presence first warning | Coupling notice | Built now | Turning the console log off while game presence is on asks to turn presence off first |
| 4 | Launch | Curated launch options | Server list of options | Built now | Game > Launch options > Common launch options: Sigil's own list of ten, press to add or take out (wave 4; DMM's list is server side) |
| 5 | Launch | Copy command | Copy option | Elsewhere in Sigil | Steam launch options field is selectable |
| 6 | Launch | Create custom option | Own option | Had it | Game > Steam launch options |
| 7 | Launch | Edit custom option | Edit | Had it | Same field |
| 8 | Launch | Delete custom option | Delete | Had it | Same field |
| 9 | Autoexec | Editor | Edit autoexec.cfg | Elsewhere in Sigil | Autoexec page |
| 10 | Autoexec | Managed sections | Read-only blocks | Elsewhere in Sigil | Autoexec page |
| 11 | Autoexec | Open in editor | External editor | Elsewhere in Sigil | Autoexec page |
| 12 | Autoexec | Open in folder | Reveal | Elsewhere in Sigil | Autoexec page |
| 13 | Autoexec | Clear | Empty file | Elsewhere in Sigil | Autoexec page |
| 14 | Autoexec | Save changes | Write | Elsewhere in Sigil | Autoexec page (backup) |
| 15 | Autoexec | Command library search | Filter | Elsewhere in Sigil | Autoexec page |
| 16 | Autoexec | Category chips | Filter | Elsewhere in Sigil | Autoexec page |
| 17 | Autoexec | Add command | Insert | Elsewhere in Sigil | Autoexec page |
| 18 | Autoexec | Community warning | Info | Elsewhere in Sigil | Autoexec page |
| 19 | Game | Current game path | Info | Had it | Game > Deadlock folder |
| 20 | Game | Browse game path | Picker | Had it | Game > Choose |
| 21 | Game | Auto-detect | Find | Had it | Game > Detect |
| 22 | Game | Use custom Steam path | Manual Steam root | Built now | Game > Steam > Steam folder: Choose or Automatic |
| 23 | Game | Steam path info | Info | Built now | Game > Steam folder line |
| 24 | Game | Browse Steam folder | Picker | Built now | Game > Steam folder > Choose |
| 25 | Game | gameinfo.gi status rows | Health | Had it | Game > gameinfo.gi |
| 26 | Game | Create gameinfo backup | Backup | Had it | Every repair backs up; Backups |
| 27 | Game | Restore gameinfo backup | Restore | Had it | Backups > Restore |
| 28 | Game | Reset gameinfo to vanilla | Remove edits | Elsewhere in Sigil | Launch vanilla sets mods aside; Backups restores the original |
| 29 | Game | Validate configuration | Check | Had it | Game > Check |
| 30 | Game | Open gameinfo.gi in editor | Editor | Built now | Game > pen button |
| 31 | Game | Remove paths when game closes | Cleanup on exit | Built now | Game > While the game runs: takes citadel/addons out of gameinfo.gi (backup first) when the game closes; a modded launch puts it back |
| 32 | Game | Block changes while game runs | Protection | Built, hook pending | Game > While the game runs: Browse, source and DeadlockForge installs, backup restores, Clear all mods and imprint wait; Installed toggles and deletes call `watch::refuse_while_running` once their owner adds the hook |
| 33 | Game | Check for crashes after launch | Crash diff | Built now | Game > Check for crashes after a launch: new dump or crash exit code raises a notification |
| 34 | Game | Hero parser status | VPK hero index | Elsewhere in Sigil | Game Index (Agent tools) |
| 35 | Game | Start or stop background scan | Index | Elsewhere in Sigil | Game Index |
| 36 | Game | Index unindexed | Index | Elsewhere in Sigil | Game Index |
| 37 | Game | Force rescan | Index | Elsewhere in Sigil | Game Index, Storage > Rescan |
| 38 | Game | Clear index | Index | Elsewhere in Sigil | Game Index |
| 39 | Mods | GameBanana source refresh | Resync catalog | Built now | Network > Refresh Browse |
| 40 | Mods | Add source | Third-party sources | Built now | Mods > Sources > Add a source (dmm-source 1.0; shows every origin before adding) |
| 41 | Mods | Enable source | Toggle | Built now | Mods > Sources switch |
| 42 | Mods | Refresh source | Resync | Built now | Mods > Sources > Sync again |
| 43 | Mods | Remove source | Remove | Built now | Mods > Sources > Remove |
| 44 | Mods | Source status | Info | Built now | Mods > Sources row line (origin, mod count, synced, error) |
| 45 | Mods | Publish your own source | Guide link | Built now | Mods > Sources > How to publish your own source |
| 46 | Mods | Auto-reapply mods | At startup | Built now | Game > Reapply mods when Sigil starts (vanilla stash back, gameinfo.gi repair) |
| 47 | Mods | Hero conflict warning | Warn | Built now | Mods > Warn about a second mod for one hero (a mod's heroes are its GameBanana hero tag plus the heroes its model paths replace) |
| 48 | Mods | Map mod launch warning | Ask | Built now | Mods > Warn about map mods at launch; a modded launch with map mods on asks Turn them off and launch, Launch anyway, Don't ask again (wave 3) |
| 49 | Mods | Fix mods before launch | Auto repair | Built now | Mods > Fix mods automatically before launch: a launch step runs the compatibility repairs (data, sounds, cameras) on every enabled mod, originals kept for Revert (wave 3); Game > Fix mods before launch still repairs gameinfo.gi |
| 50 | Mods | Several skins per hero | Locker | Elsewhere in Sigil | Locker page |
| 51 | Mods | Sort sounds by hero | Locker | Elsewhere in Sigil | Locker page |
| 52 | Mods | Default sort | Sort | Had it | Mods > Order |
| 53 | Mods | Paginated browsing | Pages | Elsewhere in Sigil | Browse loads the next page as you scroll near the end; same catalog reach, different design |
| 54 | Mods | Update badges | Badges | Built now | Mods > Browse > Show mod update marks (on; Browse cards and details, and Installed cards' Update mark, wave 4) |
| 55 | Mods | Download cache location | Info | Had it | Storage > Mod workspace, Downloads |
| 56 | Mods | Move download cache | Move | Built now | Mods > Downloads > Download cache > Move (folder moved, link left in place) |
| 57 | Mods | Move cache back | Move | Built now | Mods > Downloads > Download cache > Move back |
| 58 | Application | Vanilla mode | Launch without mods | Built now | Game > Launch options > Vanilla launches without launch options (off; Steam's options are kept and written back at the next modded launch, wave 4) |
| 59 | Application | Minimize to tray | Tray | Built, hook pending | General > Window; `tray.rs` is done, `main.rs` needs `keep_running`, `on_close` and `tray::install` (owner listed); Rune has no macOS tray backend |
| 60 | Application | Close to tray | Tray | Built, hook pending | General > Window; same hooks as Minimize to tray |
| 61 | Application | Automatic updates | Check | Had it | Network > Check for Sigil updates |
| 62 | Application | Update channel | Stable or nightly | Built now | About and updates > Release channel |
| 63 | Application | Developer mode | Dev page | Elsewhere in Sigil | Experimental > Agent shows the developer tools |
| 64 | Application | DeadlockForge 1-click | Bridge | Built now | Integrations > DeadlockForge (see Grimoire 67) |
| 65 | Application | NVIDIA Wayland compatibility | Webkit workaround | Declined (rule) | Sigil has no webview |
| 66 | Application | Theme light / dark / system | Theme | Had it | Appearance > Theme |
| 67 | Application | Audio volume | Preview volume | Built now | Status bar volume (popover, mute) and Browse > View menu move the one preview volume (wave 3) |
| 68 | Application | Show feature tips | Tips | Built now | General > Setup and tips > Show page tips |
| 69 | Application | 3D model preview | Toggle | Not built yet | Needs a 3D model renderer; Rune has no mesh pipeline yet |
| 70 | Application | Language | 18 languages | Built now | General > Language (English, Deutsch; tables for more) |
| 71 | Features | Minimal mode | All features off | Built now | General > Pages > Mods only: every optional page off, each switch comes back after (wave 3) |
| 72 | Features | Stats | Feature | Built now | General > Pages > Stats hides the page and its route; Privacy > Player stats stops its reads |
| 73 | Features | Server browser | Feature | Built now | General > Pages > Servers (moved from Experimental; the saved value is kept) |
| 74 | Features | Server picker | Block relay regions | Built now | Servers > Regions (ADR 0016, owner yes 2026-11-20): off by default, nothing changes until Apply; Windows only for blocking |
| 75 | Features | Crosshairs | Feature | Built now | General > Pages > Crosshair; community crosshairs need DMM API (backend) |
| 76 | Features | Hero skins | Feature | Built now | General > Pages > Locker; off also stops the launch shuffle |
| 77 | Features | Performance | Feature | Built now | General > Pages > Performance |
| 78 | Features | Mod Foundry | Feature | Built now | Experimental > Agent |
| 79 | Features | Autoexec editor | Feature | Built now | General > Pages > Autoexec |
| 80 | Features | Plugins | Feature | On hold (owner) | Owner decision 2026-11-18: on hold until players ask |
| 81 | Features | Experimental badges | Info | Built now | Experimental page intro |
| 82 | Features | Leftover warning | Info | Built now | Experimental: turning off hides, deletes nothing |
| 83 | Themes | Predefined / custom | Switch | Had it | Appearance > Theme |
| 84 | Themes | Seasonal themes | Auto | Built now | Appearance > Seasons and extras > Seasonal themes (off by default; DMM's dates, Sigil's own look, wave 4) |
| 85 | Themes | Seasonal dismiss | Button | Built now | Appearance > Seasons and extras > Turn off until next time / Bring it back, and Seasonal effects (off) (wave 4) |
| 86 | Themes | Remlock | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 87 | Themes | Nightshift | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 88 | Themes | Bloodmoon | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 89 | Themes | Tea | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 90 | Themes | Drututt | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 91 | Themes | Le Deadlock | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 92 | Themes | Arcane | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 93 | Themes | Deadlock API | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 94 | Themes | Lovelock | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 95 | Themes | OLED | Theme | Had it | Appearance > Black |
| 96 | Themes | Gridline | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 97 | Themes | Coding | Theme | Declined (rule) | GPL: names and palettes not copied; ten original presets in Personalize |
| 98 | Themes | Creator links | Links | Declined (rule) | No third-party themes |
| 99 | Themes | Theme accent picker | Accent | Had it | Appearance > Accent colour |
| 100 | Themes | Edit exported theme | Builder | Built now | Appearance > Personalize > Your themes > Edit |
| 101 | Themes | Delete exported theme | Builder | Built now | Appearance > Personalize > Your themes > Delete |
| 102 | Themes | Builder accent | Colour | Had it | Appearance > Accent colour |
| 103 | Themes | Builder line colour | Colour | Built now | Appearance > Personalize > Theme builder > Line |
| 104 | Themes | Builder card colour | Colour | Built now | Appearance > Personalize > Theme builder > Card |
| 105 | Themes | Builder popover colour | Colour | Built now | Appearance > Personalize > Theme builder > Card (Sigil draws popovers on the card layer) |
| 106 | Themes | Builder secondary colour | Colour | Built now | Appearance > Personalize > Theme builder > Surface |
| 107 | Themes | Builder muted colour | Colour | Built now | Appearance > Personalize > Theme builder > Field |
| 108 | Themes | Builder text colour | Colour | Built now | Appearance > Personalize > Theme builder > Text |
| 109 | Themes | Builder muted text colour | Colour | Built now | Appearance > Personalize > Theme builder > Muted text |
| 110 | Themes | Sidebar transparency | Slider | Built now | Appearance > Transparent sidebar (on or off) |
| 111 | Themes | Ambient background | Glow | Built now | Appearance > Personalize > Glow |
| 112 | Themes | Ambient colour | Glow | Built now | Appearance > Personalize > Glow colours |
| 113 | Themes | Ambient strength | Glow | Built now | Appearance > Personalize > Glow > Strength |
| 114 | Themes | Ambient spread | Glow | Built now | Appearance > Personalize > Glow > Spread |
| 115 | Themes | Corner radius | Radius | Built now | Appearance > Personalize > Shape > Corners (shared controls; screens with fixed radii keep theirs) |
| 116 | Themes | Live preview | Preview | Had it | Theme tiles preview each theme |
| 117 | Themes | Apply draft | Apply | Built now | Appearance > Personalize > Theme builder paints the app live |
| 118 | Themes | Export theme | Save | Built now | Appearance > Personalize > Your themes > Export file, Copy theme code |
| 119 | Themes | Clear builder | Reset | Built now | Appearance > Personalize > Theme builder > Cancel |
| 120 | Themes | Save / cancel edits | Save | Built now | Appearance > Personalize > Theme builder > Save theme, Cancel |
| 121 | Background | Image preview / remove | Background image | Built now | Appearance > Personalize > Background image (preview, Remove) |
| 122 | Background | Choose image | File | Built now | Appearance > Personalize > Background image > Choose image |
| 123 | Background | Image URL | Remote image | Declined (rule) | Network allowlist: loading from any pasted host is not allowed; choose a local file. A one-time download into app data would need the owner's exception |
| 124 | Background | Opacity | Slider | Built now | Appearance > Personalize > Background image > Opacity |
| 125 | Background | Blur | Slider | Built now | Appearance > Personalize > Background image > Blur |
| 126 | Background | Coverage diagram | Info | Built now | Appearance > Personalize > Background image preview shows the covered parts |
| 127 | Background | Include sidebar | Toggle | Built now | Appearance > Personalize > Background image > Behind the sidebar |
| 128 | Background | Include top bar | Toggle | Built now | Appearance > Personalize > Background image > Behind the top bar |
| 129 | Background | Include bottom bar | Toggle | Built now | Appearance > Background image > Behind the status bar (off by default): the image runs under the status bar; off, it stops at the bar's top edge |
| 130 | Network | Download routing | Mirror first | Built now | Mods > Downloads > Fastest server first |
| 131 | Network | Faster downloads | Parallel parts | Built now | Mods > Downloads > Faster downloads (4 or 8 parts, files of 8 MB or more) |
| 132 | Network | Enable proxy | Proxy | Built now | Network > HTTP proxy |
| 133 | Network | Protocol | HTTP / HTTPS / SOCKS5 | Built now | Network > Proxy server: HTTP, HTTPS, SOCKS5 and SOCKS5h (wave 4, C12) |
| 134 | Network | Host | Field | Built now | One address field |
| 135 | Network | Port | Field | Built now | Validated in the address |
| 136 | Network | Authentication | Toggle | Built now | Network > Proxy sign-in |
| 137 | Network | Username | Field | Built now | Network > Proxy sign-in user name |
| 138 | Network | Password | Field | Built now | Network > Proxy sign-in password (OS keychain) |
| 139 | Network | Bypass list | Hosts | Built now | Network > Skip the proxy for |
| 140 | Network | Test connection | Probe | Built now | Network > Test connection |
| 141 | Network | Restore defaults | Reset | Built now | Network > Clear |
| 142 | Network | Cancel | Discard | Built now | Save stays off until the text changes |
| 143 | Network | Save | Apply | Built now | Network > Save |
| 144 | Network | Proxy status | Info | Built now | Network > HTTP proxy line |
| 145 | Integrations | GameBanana pairing status | Remote install | Not built yet | Backlogged by the owner (2026-11-18): GameBanana has to list Sigil as a 1-Click manager first. The protocol is in `deadlock_core::remote_install`, not wired |
| 146 | Integrations | Pairing guide | Info | Needs a backend | With 145 |
| 147 | Integrations | Linked GameBanana member | Info | Built now | Integrations > GameBanana (public profile link) |
| 148 | Integrations | Check now | Poll queue | Needs a backend | With 145 |
| 149 | Integrations | Unlink | Unpair | Built now | Integrations > GameBanana Unlink |
| 150 | Integrations | Full guide | Link | Needs a backend | With 145 |
| 151 | Discord | Game presence | Activity | Built now | Integrations > Discord Rich Presence (needs the app id) |
| 152 | Discord | Hide party size | Detail | Built now | Integrations > Show what you are doing, Show your mod count |
| 153 | Discord | Discard | Templates | Built now | Integrations > Discord lines > Discard |
| 154 | Discord | Reset all | Templates | Built now | Integrations > Discord lines > Reset all |
| 155 | Discord | Save changes | Templates | Built now | Integrations > Discord lines > Save changes |
| 156 | Discord | Game phase selector | Templates | Built now | Integrations > Discord lines tabs per phase, from console.log (needs the Discord app id to show) |
| 157 | Discord | Customize per hero | Templates | Built now | Integrations > Discord lines > Customize for a hero |
| 158 | Discord | Reset hero override | Templates | Built now | Integrations > Discord lines > Reset to default |
| 159 | Discord | Top line | Template | Built now | Integrations > Discord lines > Top line |
| 160 | Discord | Bottom line | Template | Built now | Integrations > Discord lines > Bottom line |
| 161 | Discord | Placeholder chips | Template | Built now | Integrations > Discord lines > Insert chips |
| 162 | Discord | Reset tab | Template | Built now | Integrations > Discord lines > Reset this tab |
| 163 | Discord | Preview card | Preview | Built now | Integrations > Discord shows |
| 164 | Tools | Open game folder | Reveal | Had it | Game > folder button |
| 165 | Tools | Open mods folder | Reveal | Had it | Game > Addon folder Reveal |
| 166 | Tools | Open app data | Reveal | Had it | Storage > Data folder |
| 167 | Tools | Connection diagnostics | Probe hosts | Built now | Network > Troubleshooting > Connection checklist (DMM's checks, How to fix, Copy results; wave 4) and Diagnostics |
| 168 | Tools | Repair mod folder | Rebuild manifest | Elsewhere in Sigil | Storage > Rescan; install journal in core |
| 169 | Tools | Restore unknown-file alert | Alert | Built now | Storage > Mod data > Warn about unknown files (startup notification, Don't warn again) |
| 170 | Tools | Mod recovery report | Info | Built now | Storage > Mod data > Mod recovery report |
| 171 | Tools | Clear catalog and resync | Danger | Built now | Network > Refresh Browse |
| 172 | Tools | Wipe local catalog | Danger | Built now | Storage > Mod catalog > Saved catalog: count, last sync, size, Sync now, Wipe with a confirm (wave 4, C13) |
| 173 | Tools | Clear download cache | Danger | Had it | Storage > Downloads Clear |
| 174 | Tools | Clear all mod data | Danger | Built now | Storage > Mod data > Clear all mod data |
| 175 | Tools | Clear mods state | Danger | Built now | Storage > Mod data > Reset mod records |
| 176 | Tools | Clear all mods | Danger | Built now | Storage > Mod data > Clear all mods (to Recently deleted) |
| 177 | Backups | Backup list | Info | Had it | Backups |
| 178 | Backups | Restore (replace or merge) | Restore | Had it | Backups > Restore |
| 179 | Backups | Delete | Delete | Had it | Backups > Delete |
| 180 | Backups | Back up before updating | Policy | Had it | Replacing a mod always backs up |
| 181 | Backups | Maximum backups | Prune limit | Had it | Backups > Delete older copies |
| 182 | Backups | Create backup | Full addons backup | Built now | Backups > Full backups > Create backup; Restore replaces or merges |
| 183 | Backups | Open backups folder | Reveal | Had it | Backups > Reveal |
| 184 | Backups | Count and size | Info | Had it | Backups line |
| 185 | Logging | Open logs folder | Reveal | Built now | Logs and support > folder button |
| 186 | Logging | Open latest log | Open | Built now | Logs and support > Open |
| 187 | Logging | Log file count and size | Info | Built now | Logs and support > Log file |
| 188 | Logging | Crash dumps folder | Reveal | Built now | Logs and support > Crash dumps folder |
| 189 | Logging | Open latest dump | Open | Built now | Show newest |
| 190 | Logging | Dump count | Info | Built now | Crash dumps line |
| 191 | Logging | AI provider | AI site | Declined (rule) | Sigil has an agent; logs never go to a site automatically |
| 192 | Logging | Log source | Select | Built now | Logs and support > Log source: Combined, Sigil log, Crash dumps; Copy logs (wave 4) |
| 193 | Logging | Get AI help | Copy and open AI | Built now | Logs and support > Ask Sigil for help: the masked logs go into Sigil's own agent, nothing is sent until Send (wave 4) |
| 194 | Experimental | Custom maps | Flag | Built now | Experimental > Custom maps (off by default) adds a Maps page (GameBanana's Maps category through Browse's catalog read and install button) and How to play on the Maps page, Browse details of a Maps mod and Installed's card menu: map name from the VPK's `maps/<name>.vpk` (`deadlock_core::maps`) or the description, Launch map (managed autoexec block, gameinfo gate, launch steps), Host this map, Invite friends (`connect <code>` read from console.log) |
| 195 | Experimental | Mod Foundry | Flag | Built now | Experimental > Agent |
| 196 | Experimental | Performance configs | Flag | Elsewhere in Sigil | Performance page |
| 197 | Experimental | Cloud backup | Flag | Needs a backend | Needs a Sigil account service |
| 198 | Experimental | Plugins | Flag | On hold (owner) | Owner decision 2026-11-18: on hold until players ask; agent tools are today's extension point |
| 199 | Experimental | Blocked note | Info | Built now | Experimental intro line |
| 200 | Match data | What it powers | Info | Built now | Privacy > What is sent |
| 201 | Match data | Tracker site tiles | Links | Built now | Integrations > Stat sites |
| 202 | Match data | Share from Steam cache | Toggle | Built now | Privacy > Share match data with deadlock-api (off by default) |
| 203 | Match data | Match sync about | Info | Declined (rule) | Sigil does not sign in to Steam |
| 204 | Match data | Enable match sync | Steam login | Declined (rule) | Account risk; needs Steam session |
| 205 | Match data | Per-account sync | Toggle | Declined (rule) | With 204 |
| 206 | Match data | Quota | Info | Declined (rule) | With 204 |
| 207 | Match data | Fetch all matches | Button | Declined (rule) | With 204 |
| 208 | Match data | Full sync progress | Info | Declined (rule) | With 204 |
| 209 | Match data | Retry | Button | Built now | Share now |
| 210 | Privacy | Hide NSFW | Toggle | Had it | Mods > Mature content Hide |
| 211 | Privacy | Show likely NSFW | Toggle | Built now | Mods > Mature content > Show unclassified items (DMM's flag has no heuristic; unclassified source mods) |
| 212 | Privacy | Disable NSFW blur | Toggle | Had it | Mods > Mature content Show |
| 213 | Privacy | Blur strength | Slider | Built now | Mods > Mature content > Blur strength |
| 214 | Privacy | Lightweight blur | Linux | Declined (rule) | Native renderer, no webview blur cost |
| 215 | Privacy | Remember per-item choices | Toggle | Built now | Mods > Mature content > Remember what you reveal |
| 216 | Privacy | Clear saved choices | Button | Built now | Mods > Mature content > Clear |
| 217 | Privacy | Player stats | Toggle | Had it | Privacy > Player stats |
| 218 | Privacy | Google Analytics | Telemetry | Declined (rule) | No third-party analytics; Sigil's own opt-in telemetry (ADR 0014) covers usage, crashes and diagnostics |
| 219 | Privacy | Network activity docs | Link | Built now | Privacy > Where Sigil connects (inline) |
| 220 | Information | Third-party disclaimer | Info | Had it | About: Valve line |
| 221 | Information | About text | Info | Had it | About |
| 222 | Information | GameBanana credit | Link | Built now | Privacy hosts; About credits list notices |
| 223 | Information | Rich Presence credit | Link | Declined (rule) | Sigil's IPC is its own |
| 224 | Information | Tauri credit | Link | Declined (rule) | Not used |
| 225 | Information | shadcn credit | Link | Declined (rule) | Not used |
| 226 | Information | vpkmerge credit | Link | Had it | About > Open source |
| 227 | Information | ValveResourceFormat credit | Link | Had it | About > Open source |
| 228 | Information | Tailwind credit | Link | Declined (rule) | Not used |
| 229 | Information | Third-party notices | Link | Had it | About > Open source, from THIRD_PARTY_NOTICES.md |
| 230 | Information | Contributors | Page | Built now | About and updates > People > Contributors |
| 231 | Information | Reset onboarding | Button | Built now | General > Setup and tips > Run setup |
| 232 | Information | Reset feature tips | Button | Built now | General > Setup and tips > Reset tips |

