# Privacy and network activity

Status: draft. Describes the intended behavior of the code in this repository. The telemetry Worker
exists in `services/telemetry/` with tests; it is not deployed. The desktop app does not send
telemetry yet, and the Settings > Privacy switches for it do not exist yet.
Updated: 2026-11-18

This page lists every service Sigil contacts, what each one receives, and how to turn it off.
Sigil's own receiving end is a Cloudflare Worker in this repository, so the code that handles each
upload can be read in `services/telemetry/`.

Any request reveals your IP address to the server that receives it. That applies to every entry
below and is not repeated in each one. Sigil itself stores no IP address and no user agent.

## At a glance

| Recipient | What it receives | Default |
| --- | --- | --- |
| [Sigil telemetry](#sigil-telemetry) | Usage events, crash reports, diagnostics you send, mod hash lookups | Off until you agree |
| [Model provider](#model-provider) | Your prompts, the files the agent reads, the model's answers | Only when you chat |
| [Steam](#steam) | Patch note requests | On |
| [GameBanana](#gamebanana) | Catalog and update requests, the mods you open and install | On |
| [deadlock-api](#deadlock-api) | Account ids for Stats, crosshair values, match ids and salts | Partly on, see below |
| [GitHub](#github) | Update checks, performance config files, contributor list | On |
| [Deadworks](#deadworks) | Server list and registry requests, server pings | Only when you use Servers |
| [Divine Skins](#divine-skins) | Public one-click manifests | Not called today |
| [Mod sources you add](#mod-sources-you-add) | The catalog request for that source | Only sources you add |
| [Discord](#discord) | Your current screen and activity, through the Discord app on your PC | On |
| [DeadlockForge](#deadlockforge) | Nothing outbound; a listener on your own PC | Off |
| [Stat sites](#stat-sites) | Nothing from Sigil; they open in your browser | On press |

## What can identify you

Sigil sends four kinds of identifier. Nothing else it sends is tied to you.

- **Install id**, a random UUID Sigil makes on first start. It is not a machine id and is not
  derived from hardware. It is sent with telemetry only, and the Worker stores only its SHA-256.
- **Steam account id**, the numeric id of the account you enter in Stats. It is sent to
  deadlock-api for the player and match views you open.
- **GameBanana member id**, only if you link a GameBanana account for Remote Install. The pair
  token stays in the OS keychain and travels in the Remote Install URL and nowhere else.
- **Proxy user name**, only if you configure a proxy. The password stays in the OS keychain.

Sigil never reads your Steam login for telemetry, never uses a hardware or operating system machine
id, and never puts an install id into a crash report or a diagnostics bundle.

## Sigil telemetry

Off until you agree, in a first-start dialog or in Settings > Privacy. There is one switch per kind:
usage events, crash reports and unknown-mod lookup. Diagnostics are never automatic: they are sent
only when you press Send. After Sigil crashes, the next start asks about that one crash report even
when crash reports are off (see [Crash reports](#crash-reports)).

The receiving end is the Cloudflare Worker `sigil-telemetry` in `services/telemetry/`, deployed by
the owner. The desktop app sends to `https://sigil-telemetry.gantchevkaloyan.workers.dev`. Analytics Engine holds the
usage events, D1 holds crashes, diagnostics and the hash index. Telemetry goes through the same
network gate as every other core request, so Offline mode blocks it too.

### What each kind sends

Usage events (`POST /v1/events`):

| Field | Example | Stored |
| --- | --- | --- |
| install id | a random UUID | Only its SHA-256, as the analytics sampling index |
| app version | `0.4.1` | Yes |
| operating system | `windows`, `macos`, `linux` or `other` | Yes |
| event name | `page:browse`, `install:done`, one of 42 names | Yes, allowlisted names only |
| page | `browse` | Yes when it is a known screen |
| time | milliseconds since the epoch | Validated, not stored |

There is no free text field. An event name outside `src/events.ts` is dropped instead of stored, so
no file name, mod name, hero name or search text can ride along. At most 50 events per request.
Events wait in a local queue of at most 500 (oldest dropped) when Sigil is offline or between
sends.

<a id="crash-reports"></a>Crash reports (`POST /v1/crash`, at most 96 KiB; the app keeps a report
under 60 KiB):

| Field | What it holds | Stored |
| --- | --- | --- |
| install id | the random UUID | Checked for shape, never stored |
| app version | the version that crashed | Yes |
| operating system | `windows`, `macos`, `linux` or `other` | Yes |
| kind | `panic`, `unclean_exit` or `native` | Yes |
| message | the panic message and its source location (at most 8 KiB) | Yes |
| backtrace | the Rust backtrace (at most 48 KiB); for an unclean exit, the last log line; for `native`, the Windows exception code, the address as an offset in `Sigil.exe` and the thread id | Yes |
| context | the panicking thread's name, when the session started, and the last 150 lines of Sigil's own log (at most 16 KiB, newest lines kept) | Yes, the newest one per crash |

Everything is scrubbed on your PC and again on the server. Home paths lose the user name
(`/Users/<user>`), and Steam ids, emails, provider keys, GitHub tokens, bearer tokens and JWTs become
placeholders. Identical crashes are one row with a repeat count, matched on the message and the
first frames. Kept 90 days.

How a crash becomes a report: Sigil writes a small session marker in its data folder when it starts
and removes it whenever it exits cleanly (window close, tray Quit, the quit key, Restart to update).
Versions before 0.1.29 did not remove it on a normal quit, so a marker they left is ignored unless a
crash was also recorded. A marker
found at the next start means the last run did not exit cleanly. A panic is written to a local
crash file the moment it happens; a run that died without one (an abort, a stack overflow, a driver
crash, a forced quit) becomes an `unclean_exit` report, and on Windows an unhandled exception leaves
a one-line `native` record. When nothing was recorded and the computer started after the run did,
it was a shutdown, not a crash, and nothing is offered.

- Crash reports on: the report is sent at start and a toast says so.
- Crash reports off, or never answered: a dialog says what happened, shows exactly what would be
  sent (Show what is sent), and offers Send report, Don't send and "Always send crash reports" (the
  same switch as in Settings > Privacy). Pressing Send report is your consent for that one report;
  nothing goes without the press. Don't send deletes the local file, and the same crash is not
  asked about again for a day.
- Limits on your PC: the same crash is sent at most once a day, and at most 3 reports an hour and
  10 a day leave this install, whatever their kind.

Diagnostics (`POST /v1/diagnostics`): app version, operating system, a summary you see, and a log
file (maximum 1 MiB), scrubbed the same way. The Worker returns a support code such as
`SGL-7K4M-9QZT` so you can refer to the bundle without pasting it. The install id is not stored.
Kept 30 days.

Unknown mod lookup (`GET /v1/identify`): the SHA-256 and size of a VPK file. The Worker looks it up
in the hash index and answers with the GameBanana mods other players reported for it. It stores
nothing about the caller and keeps no search history.

Hash contribution (`POST /v1/hashes`): the SHA-256, size, GameBanana mod id and file id of a file you
just installed from GameBanana. It is sent only for installs you asked for, only while the
unknown-mod lookup switch is on, and it carries no install id.

### What is never sent

- Log files, except inside a diagnostics bundle you send by pressing a button, and the last 150
  scrubbed lines inside a crash report (sent automatically only with crash reports on).
- Mod files, VPK contents, or any file from the game install.
- File paths, except paths inside a diagnostics bundle, and those are scrubbed of user names.
- Steam ids or Steam login details.
- API keys, provider tokens, proxy passwords or the GameBanana pair token.
- IP addresses and user agents, which the Worker reads only to derive a rate limit key; the key is a
  SHA-256 with a salt held in memory that is replaced at UTC midnight, and it is never written to
  storage or to a log line.

The Worker writes one structured log line per request with the method, path, status and duration.
It never logs a request body, a header value or a query string. Cloudflare, as the operator of the
network, processes request metadata to deliver the request under its own policy; Sigil does not
store it.

Events are aggregate points. Cloudflare keeps them for the retention period set on the dataset in
the owner's account, and Sigil keeps no separate copy. The daily cron job deletes diagnostics older
than 30 days and crashes older than 90.

## Model provider

Sigil is bring your own key. Provider requests go from the app or its agent host straight to the
provider you configured: OpenAI, Anthropic, or an OpenAI-compatible endpoint such as a local model
server or gateway. No Sigil server is in the path.

- Your prompt, the files the agent reads while working on your mod, tool output and the model's
  answers go to that provider. Their privacy terms apply to that content.
- Keys live in the Rust app: the OS keychain on the desktop, a `chmod 600` file for the CLI. They
  never enter the chat transcript, tool output, logs or a commit.
- A local OpenAI-compatible endpoint never leaves your machine unless you point it at a remote host.
- Subscription sign-in for a provider happens in your browser. The token stays on your PC; nothing
  about it is sent to Sigil.

## Steam

`api.steampowered.com` serves the official Deadlock patch feed (news items and their dates) when
Patches opens or refreshes. Anonymous GETs, no account, no login.

The relay region picker (Servers > Regions) reads Valve's relay list for Deadlock from
`api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=1422450` when the tab opens and the saved
copy is older than a day, or when you press Refresh. Anonymous GET, no account, nothing about you
in it; the answer is kept in Sigil's data folder. Measure ping, on your press only, sends ICMP
echo (the system `ping`) to the relay addresses in that list. With "Relay health in the status bar"
turned on (Regions tab, off by default) the same sweep also runs at startup and every 30 minutes,
and the list is read from the saved copy first; without it nothing is pinged unless you press.
Blocking regions adds Windows Firewall rules on your PC behind Windows' admin prompt; nothing
about them is sent anywhere.

Match data sharing is opt-in, off by default. While it is on, Sigil reads the first 512 bytes of
each file in Steam's local HTTP cache (`<Steam>/appcache/httpcache`), keeps URLs shaped
`replayN.valve.net/1422450/<match>_<salt>.(meta|dem).bz2`, and posts the match id, cluster id, salt
and the fixed user name `Sigil` to deadlock-api. No account id, Steam login, file or other data is
sent, and Steam itself is not contacted for this. Details are under [deadlock-api](#deadlock-api).

## GameBanana

GameBanana hosts most mods. Sigil talks to `gamebanana.com/apiv11` and the image, file and cache
hosts those answers name (`images.gamebanana.com`, `files.gamebanana.com`,
`filecacheN.gamebanana.com`, and `gamebanana.com/dl/<id>` redirects).

- Browse and search use anonymous GETs. Search text you type is sent as GameBanana's name filter.
  Categories, hero filters, sort order, collections, dashboard rails and a submission's posts,
  updates and requirements are read when you open or scroll them.
- A bounded local copy of the catalog (`browse/mirror/`) is refreshed while Browse is open. It
  sends only public page and filter parameters.
- Installed GameBanana mods are checked for updates with one anonymous GET per mod when Installed
  opens, at most every six hours, or on Check for updates. The request carries only the public mod
  id.
- An install downloads from the file server GameBanana or you picked. A server speed test reads a
  256 KiB range from at most three servers, only when you press Test now. Favorites, hidden items,
  ignored updates and filters stay local.
- Linking a GameBanana account reads the public member name once.
- A creator's page (Browse scoped to one creator, or More from a creator on an Installed card)
  reads that member's public `Member/<id>/ProfilePage` and their Deadlock `Mod/Index` and
  `Sound/Index` lists filtered to the member (at most six and two pages of 50). More from a
  creator first reads the mod's own page to find who posted it. Only public ids are sent.
- Remote Install, if you pair it, reads your queued mods from GameBanana every five minutes while
  Sigil is open. The member id and pair token travel in that URL; the token is in the OS keychain.
  Each read marks returned entries retrieved on GameBanana, which is the only acknowledgement the
  protocol has.
- Mod preview images, hero portraits and avatars are cached under the app data directory.

Nothing about you is sent beyond what answering those requests requires.

## deadlock-api

`api.deadlock-api.com` is a community project with game data and statistics.

- Hero, item and rank data for Stats, Locker and hero pickers. Nothing about you is sent.
- Stats > Fun run reads the hero, item and ability lists (`/v1/assets/heroes?only_active=true`,
  `/v1/assets/items/by-type/upgrade` and `/by-type/ability`) on the first Roll, cached for a week.
  Rolling, codes and Copy build stay on this computer. Nothing about you is sent.
- Stats views send the Steam account ids you enter or pick, with the filters you chose, and read
  hero stats, item stats, hero combination stats and one match's item timeline. Responses are
  cached locally (`cache/stats`).
- Crosshair: Copy code sends only the crosshair convar values, and only when you press it. Import
  sends a pasted `DL.` code only when the text looks like one and you press Import. Both follow the
  share codes switch in Settings > Privacy.
- Match data sharing (opt-in, off by default) posts `match_id`, `cluster_id`,
  `metadata_salt` or `replay_salt`, and `username: "Sigil"` to `/v1/matches/salts`. No account id,
  login or file is sent. Each batch is recorded in `<data>/match-sharing.json` and shown in
  Settings; Forget deletes that log.
- Live match player lookup (opt-in, off by default, and only with Player and match stats on): once
  per match, the account ids the game's versus screen showed are sent to `/v1/players/steam` and
  `/v1/players/{id}/rank` for match counts and ranks. The live match view itself reads only the
  game's `console.log` on this computer; the live bridge HUD mod draws and sends nothing.
- Regions "Where I play" (only with Player and match stats on and a Stats account set): when
  Servers > Regions opens, the primary Stats account id is sent to `/v1/players/{id}/region` for
  its matches by month and datacenter, cached six hours. The counts stay on this computer and
  build the preset and the match count beside each location.

## GitHub

- `api.github.com` and `raw.githubusercontent.com` serve Performance configs: the newest release or
  commit of the community configs Sigil offers, their files, and Valve's convar list. Anonymous
  GETs of public files, cached under `performance/`.
  Performance > Add from GitHub reads a public repository you type, only when you press Look up
  (`api.github.com`: its newest commit and file tree), and downloads the files you pick when you
  press Add (`raw.githubusercontent.com`). Anonymous GETs; a copy is kept under `performance/`.
  Autoexec > Console variables reads that cached convar list. Sigil does not ship a copy. With no
  cached list, its Download the list button fetches the same file from the same hosts (an
  anonymous GET, only when you press it).
- The update check reads `api.github.com/repos/superuserkalo/sigil/releases`. With Stable it asks
  for the latest release, with Preview for the last ten. It runs at startup only if you turned
  Check for Sigil updates on (off by default), from Settings > About, or on the interval you chose.
  No identifier, cookie or install id is sent; the user agent is `sigil-updater`.
- `github.com/superuserkalo/sigil/releases/download/...` serves a newer Sigil. Sigil fetches the
  signed manifest and its signature (a few KB) and, if the Ed25519 signature verifies, this
  platform's zip into `updates/` in the app data directory. GitHub redirects to
  `release-assets.githubusercontent.com` to serve the file. Size and SHA-256 are checked against the
  signed manifest before anything is staged; the swap happens on Restart to update or the next
  launch, and never touches the game install.
- The contributor list is one anonymous GET when About > Contributors opens.

Offline mode blocks all of it.

## Deadworks

`api.deadworks.net` serves the Servers page: the Steam server list filtered to Deadworks servers,
the Deadworks registry (country, version, roster), and the bootstrap plus the UI pack a join needs.
Anonymous GETs, no user data.

Listed game servers receive A2S_INFO pings while the Servers page is open, and the one server you
open receives an A2S_RULES query. A join that needs server content downloads from the host the
server advertises (`dw_fastdl`) after you confirm, with the host shown in the dialog; that guard
refuses loopback, private, link-local and reserved addresses, limits redirects and size, and checks
any VPK before installing.

Host a server (Windows) downloads Valve's SteamCMD from `steamcdn-a.akamaihd.net` when you press
Install SteamCMD: one anonymous GET, checked against a pinned SHA-256.

Host a server downloads the pinned Deadworks release from `github.com/Deadworks-net/deadworks`
when you press Install Deadworks: one anonymous GET, checked against a pinned SHA-256.

Host a server with custom maps or addons runs a small read-only file server on your PC beside the
game server (TCP, the game port plus one, 27068 by default) so friends who use Sigil can download
that content from you. It starts when you press Start and stops with the server. It hands out only
the files in the server's `sigil-fastdl` folder, keeps no log, and sends nothing anywhere itself.

Steam relay mode (the default for a hosted server) changes only the server's own gameinfo.gi so
the server listens through Steam networking; friends connect through Valve's relays with a code
that changes every restart, and neither side's IP address is shown to the other by Steam. The
server is kept off the server lists. This is Valve's network, used by the game itself; Sigil sends
nothing extra.

The UPnP switch (off by default) lets Sigil ask your home router to open the server's ports while
it runs and close them when it stops. Sigil talks only to the router on your local network
(a multicast search on your LAN, then the router's own address); nothing goes to the internet and
nothing is logged. The router's answer may include your public IP address, which Sigil shows on the
Host page and puts in a Direct-mode invite; it is not stored or sent anywhere.

A Sigil invite you copy holds the join code or address, whether a password is needed (never the
password), and the GameBanana mod and file ids of the server's content. Where you paste it is up to
you. A friend's Sigil uses those ids to download from GameBanana, as any install does.

Play with friends (the in-game host) adds `-condebug` to Deadlock's launch options in Steam so Sigil
can read the join code from the game's `console.log` on your PC. It stays on until you turn it off
in Settings > Game.

What the two sides learn about each other: anyone who connects to your server or its download
port sees your public IP address (or your LAN address on a local network), as with any game
server you host. You, as the host, see the IP address of every player who joins (Deadworks prints
it in the server console) and of anyone who connects to the download port; Sigil's file server
does not record it. A joiner's Sigil downloads from the same address it connects to, so joining a
Sigil-hosted server reveals nothing to a third party.

## Divine Skins

`api.divineskins.gg`, `lol-mods.divine-cdn.com`, `lol-assets.divine-cdn.com` and
`images.divine-cdn.com` are for Divine Skins installs. The core module exists but nothing in the app
calls it yet, so Sigil makes none of these requests today. When it does, a pasted Divine Skins or
Deadlock Mod Manager link reads the public one-click manifest (and the release list when the link
names no release), then downloads the archive the manifest names. Anonymous, no account, no user
data.

## Mod sources you add

If you add a mod source in Settings > Mods, Sigil fetches the descriptor URL you pasted and only the
`api`, `downloads` and `media` origins it declares, over https, never a private or local address.
Sigil shows every origin and asks before saving. It syncs on add or Sync again, and downloads only
when you press Install. Nothing about you is sent.

## Discord

Discord Rich Presence is opt-in through the Discord app on your PC and uses no network call from
Sigil: it writes to the local Discord IPC socket. It shares the screen you are on, optionally the
enabled mod count and whether the game runs modded or vanilla, and a session start time. With Show
what you are doing off, it sends only Using Sigil. Discord shows the activity to your friends.

## DeadlockForge

1-click installs from `deadlockforge.net` are opt-in and off by default. Sigil listens on
`127.0.0.1` ports 43110 to 43114 and answers only that site's origins with a loopback host. It takes
one VPK (1 KiB to 512 MiB, VPK magic checked) and installs nothing until you press Install. There is
no outbound call.

## Stat sites

Statlocker, DeadChaps, Buildlock, DLTV and EDL open in your system browser when you press their
link. Sigil makes no request to them.

## Connection diagnostics

Settings > Network has Test connection and connection diagnostics. On a press only, they send HEAD
requests to GameBanana, `files.gamebanana.com`, deadlock-api, `api.github.com`,
`raw.githubusercontent.com` and Deadworks, through your proxy when one is set, so you can see which
host fails.

## What stays on your PC

- Your mods, projects, profiles, crosshair settings, filters and view options.
- Logs and crash dumps, which are analysed locally and only leave inside a diagnostics bundle you
  send or, as a scrubbed tail, a crash report. Deadlock's own crash dumps and the end of its
  `console.log` are read on this computer to name the mod file a crash was loading; saved game
  presets (`game-presets.json`) stay in the app data directory. A panic writes a scrubbed crash file locally whether
  or not crash reports are on; Don't send deletes it.
- The session marker (`telemetry/session.json`) and the crash ledger (`telemetry/crash-ledger.json`:
  fingerprints and times of sent and declined crashes, for the limits), in the app data directory.
- The queued usage events, until they are sent or dropped.
- Your Steam login: match data sharing reads local files and never asks Steam for it.
- Your provider keys and proxy password, in the OS keychain or the `chmod 600` CLI store.
- The install id, in the app data directory.
- Images, stats responses, patch notes, performance configs and the Browse catalog, in caches under
  the app data directory.

## Turning things off

| Setting | Location | Default |
| --- | --- | --- |
| Usage events | Settings > Privacy | Off |
| Crash reports | Settings > Privacy | Off |
| Unknown mod lookup and hash contribution | Settings > Privacy | Off |
| Player and match stats | Settings > Privacy | On |
| Crosshair share codes | Settings > Privacy | On |
| Share match data with deadlock-api | Settings > Privacy | Off |
| Live match view (reads console.log, local) | Settings > Game | On |
| Live bridge HUD mod | Settings > Game | Off |
| Look up players in the live match | Settings > Game | Off |
| Discord Rich Presence | Settings > Integrations | Off |
| 1-click installs from DeadlockForge | Settings > Privacy | Off |
| Check for Sigil updates | Settings > About and updates | Off |
| Offline mode | Settings > Network | Off |

Offline mode blocks every request the core makes through `deadlock-core`. It does not yet cover the
agent host's model provider calls, subscription sign-in or publish; see Known gaps.

## Known gaps

- The telemetry Worker is not deployed and the desktop client that sends to it does not exist yet.
  The switches above describe the agreed behavior, not shipped UI.
- Some desktop strings still say Sigil has no telemetry. They are being replaced with this page's
  wording.
- Offline mode does not cover the agent host's provider calls (`rust/crates/agent-host`,
  `apps/agent-host`), subscription sign-in (`desktop/src/signin.rs`) or publish
  (`desktop/src/publish.rs`).
- A proxy is not used by every download path yet (mod variants and Flatpak updates do not use it).
- Mod descriptions can load images from the host the author chose.
