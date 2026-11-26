# Privacy and network activity

Status: active. Describes the behavior of the code in this repository at 0.1.42. The telemetry
Worker in `services/telemetry/` is deployed. The desktop app has the Settings > Privacy switches and
sends only what they allow.
Updated: 2026-11-25

This page lists every service Sigil contacts, what each one receives, and how to turn it off.
Sigil's own receiving end is a Cloudflare Worker in this repository, so the code that handles each
upload can be read in `services/telemetry/`.

Any request reveals your IP address to the server that receives it. That applies to every entry
below and is not repeated in each one. Sigil itself stores no IP address and no user agent.

## At a glance

| Recipient | What it receives | Default |
| --- | --- | --- |
| [Sigil telemetry](#sigil-telemetry) | Usage events, crash reports, diagnostics you send, mod hash lookups | Off until you agree |
| [Help build Sigil](#help-build-sigil-product-data) | Product data: GameBanana mod ids and actions, mod health, Browse use, top heroes, performance, app use | Off until you choose Share |
| [Model provider](#model-provider) | Your prompts, the files the agent reads, the model's answers | Only when you chat, in a build with the agent (not in releases) |
| [Community crosshairs](#community-crosshairs) | A crosshair's name and settings when you publish one; nothing to browse or apply | Only when you publish |
| [Catalog search index](#catalog-search-index) | Nothing from you: the app downloads a public index of GameBanana's catalog and searches it locally | On |
| [Steam](#steam) | Patch note requests | On |
| [GameBanana](#gamebanana) | Catalog and update requests, the mods you open and install | On |
| [deadlock-api](#deadlock-api) | Account ids for Stats, crosshair values, match ids and salts | Partly on, see below |
| [Stats pictures](#stats-pictures) | Nothing from you: image requests for rank badges, item icons and Steam avatars | Only with Player stats on |
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
- **Product data id**, a second random UUID, made only when Help build Sigil is on and sent only
  with product data. The Worker stores only a salted SHA-256 of it. Delete my data replaces it.
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

## Help build Sigil (product data)

Decision: `docs/decisions/0017-opt-in-product-data.md`. Off by default. The first-run setup asks
once with two equal buttons, Share and Not now; players who finished setup before this existed are
asked once in a dialog. Settings > Privacy > Help build Sigil has the switch (on or off at any
time), What Sigil shared and Delete my data. Saying no changes nothing else in the app.

Code: the app side is `rust/crates/deadlock-core/src/product_data.rs` (schema, scrubbers, consent
gate, queue) and `rust/crates/desktop/src/screens/settings/product.rs` (asking, recording, the
Privacy rows). The Worker side is `services/telemetry/src/product.ts` (validation) and
`services/telemetry/src/aggregates.ts` (totals and the publishing floor). Only the desktop app
records or reads product data; the CLI and the agent host have no access to it.

### How it travels

Each record is pushed onto a list in memory. Every 30 seconds a worker thread moves waiting
records to `<data>/telemetry/product/queue.json` (at most 1000 records, 512 KB and 14 days; older
ones are dropped unsent), and every 10 minutes it sends them in batches of 50 to
`POST /v1/product` on Sigil's own Worker, waiting longer after each failure (1 minute doubling to
6 hours). What the Worker accepted is added to `<data>/telemetry/product/shared.json`, the last 200
records, which Settings > Privacy > What Sigil shared shows. Turning the switch off clears the
list, the queue and that log at once. Nothing is recorded, queued or sent while it is off. If the
Worker is missing its secrets it answers 503 and stores nothing; the app keeps its queue within the
limits above and tries again later, without showing an error.

Every batch carries: the product data id, Sigil's version, the OS (windows, macos, linux or
other), the OS build number (digits and dots, for example `10.0.22631`) and the game build number
(`ClientVersion` from steam.inf). A record carries no time; the Worker stamps the day it arrives.

### Every record and field

Mods appear only as GameBanana ids (whole numbers up to 1,000,000,000). A mod with no GameBanana id
(a local or private file) cannot be recorded. "Code" means a short lower-case word Sigil chose,
`[a-z][a-z0-9_]`, at most 32 characters and never four digits in a row.

| Group | Record | Fields |
| --- | --- | --- |
| Mods | `mod_action` | `mod_id`; `action`: installed, removed, enabled, disabled, updated, opened, saved, unsaved, not_interested |
| Mods | `mod_kept` | `mod_id`; `kept`: under_1d, d1_7, d7_30, d30_90, over_90d (sent at removal) |
| Mods | `enabled_set` | `mods`: up to 64 pairs of `mod_id` and `slot` (load order), once per run |
| Mods | `variant_chosen` | `mod_id`, `file_id` (the GameBanana file picked) |
| Mods | `pack_install` | `mods`: up to 64 GameBanana mod ids a shared pack queued, in pack order (never the pack's name or note) |
| Mod health | `install_result` | `mod_id`, `ok`, `reason` code (for example download_failed, game_running, file_in_use) |
| Mod health | `mod_pair` | `a`, `b` (mod ids), `kind`: overlap or conflict |
| Mod health | `compat_finding` | `mod_id`, `finding` code (moved, gone, skeleton, unreadable), `game_build` |
| Mod health | `repair_result` | `mod_id`, `repair` code, `ok`, `game_build` |
| Mod health | `off_after_crash` | `mod_id` of a mod turned off while the last game session ended in a crash |
| Mod health | `crash_suspect` | `mod_id` of the crash card's suspect |
| Browse | `rec_shown` | `row` code (for_you, rising), `position` (0 to 100), `mod_id`, `outcome`: shown, opened, installed |
| Browse | `browse_query` | `sort` code, `filters` (up to 8 codes), `search` (see below), `results` (count) |
| Play | `top_heroes` | up to 10 of `hero_id` with `playtime`: under_1h, h1_10, h10_50, h50_200, over_200h |
| Play | `rank_tier` | `tier`, 0 to 11 (not the rating, not the sub-rank) |
| Performance | `perf_config` | `config` code, `version` number |
| Performance | `benchmark` | `avg_fps`, `low_fps` (1% low), `scene` code (map name, never a match id), `width`, `height`, `preset` code |
| Performance | `hardware` | `gpu`, `cpu` (model names), `ram_gb` (rounded to 4 GB), `width`, `height`, `refresh_hz` (0 until Sigil reads it); once per Sigil version |
| App use | `feature_use` | `feature` code (the page opened), `count` |
| App use | `settings` | `theme`, `density`, `language` codes, `mature` (true unless mature mods are hidden); once per version and per change |
| App use | `launch` | `modded` (true for a modded launch, false for vanilla) |
| App use | `session` | `length`: under_5m, m5_30, m30_2h, over_2h |

Search text is the only free text. On your PC and again on the Worker it is lower-cased, its
spaces collapsed and cut to 64 characters, and it is dropped whole when it has an `@`, a slash or
backslash, a drive letter, a `~`, a link (`://`, `www.`, or `name.tld`), a Steam id form, five
digits in a row, or more than eight digits. GPU and CPU names lose trademark marks, clock speeds,
core counts and anything after a `/`; a name with a backslash, a colon or six digits in a row
(a serial) is dropped. The Worker drops any record type and any field that is not in this table.

### Never sent

Your Steam id, account or persona name, friends, match ids or match history, chat, file names or
paths, mods with no GameBanana id, mod contents, API keys or tokens, hardware serials or any id
that outlives a reinstall, and anything from the AI agent's conversations. The Worker stores no IP
address and no user agent.

### What the Worker keeps, and for how long

| Table | Holds | Kept |
| --- | --- | --- |
| `product_events` | One row per record: salted SHA-256 of the product data id, record type, Sigil version, OS, OS build, game build, the record's fields | While you keep sharing, until you press Delete my data. After 24 months with nothing new from you, the id hash is replaced by a random value (see below) |
| `product_contrib` | Working rows for the current weekly period: the salted id hash, the aggregate key, the day, and a network marker | Until the period closes (about 9 days: the week and 2 days of grace), never more than 21 days, or until Delete my data |
| `product_totals` | Counts only, per period and key: installs, networks, days. No id, no network marker. Only keys that met the floor | Kept indefinitely |

Sigil keeps this data as long as possible, on purpose: a longer history makes the counts more
useful for deciding what Sigil fixes and builds. The flip side is that a breach of the Worker's database would
expose more history, which is why the network marker below lives 21 days at most and Delete my
data works at any time. When an install has sent nothing for 24 months, the daily job replaces the
id hash on all of its records with one fresh random value made for that install and stored
nowhere else: the records stay, but they can no longer be tied to your id, and nothing new is
added to them. After that, Delete my data can no longer find them.

The network marker is how Sigil counts distinct contributors without storing addresses: a keyed
hash (HMAC-SHA256 under a secret only the Worker holds) of your network prefix (the first three
parts of an IPv4 address, the first three groups of an IPv6 address) and the weekly period. It
changes every week, is kept only in `product_contrib`, about 9 days and 21 days at most. Your IP address
itself is never stored.

### What is shown to other players, and when

Nothing from a single install. A mod pair, a mod health finding,
a frame rate for a hardware class or a search term is used only when, within one weekly period, at
least 10 installs that sent data on at least 2 days, from at least 6 different networks, on at
least 3 different days, share it. A network counts at most 3 installs per period. Keys below that
floor are discarded when the period closes. The current period is never shown.

### What the app reads back

The app does not ask the Worker for related mods, mod health chips, an install-based Rising or
frame rates for a hardware class. Those were removed from the app on 2026-11-25. The Worker's
read routes (`/v1/product/related`, `/health`, `/fps`, `/rising`) still exist and answer only with
keys that passed the floor above; Sigil does not call them. Floor-passed search gaps may still
shape the public catalog tags (see Catalog tags below).

### Delete my data

Settings > Privacy > Delete my data sends your product data id to `POST /v1/product/delete`. The
Worker deletes every `product_events` row and every `product_contrib` row for it, the app clears its
queue and its What Sigil shared log, and a new id is made if you share again. Totals from periods
that already closed are anonymous counts with nothing pointing to you, so they stay: you may still
be one of the installs in a count from an earlier period. If the request fails, nothing changes on
your PC and you can press again.

## Community crosshairs

Publishing a crosshair to the community library is your own action, one crosshair at a time, and
only then does Sigil send its name and its settings to Sigil's Worker. Both become public: anyone
using Sigil can see the name, the settings, how often it was applied and the day it was published.
Nothing else is attached: no account, no install id, no player or Steam id, and the Worker stores
no address. Browsing, applying and reporting send no data about you. To count fairly, the Worker
keeps two keyed hashes of your network (not your address): a daily one, deleted after 2 days, that
counts an apply once per day and caps publishes, reports and applies per day; and, when you report
a crosshair, a hash of your network and that crosshair, kept up to 90 days so one network counts
once per crosshair, which cannot be matched to your reports of other crosshairs (see
`docs/decisions/0019-community-crosshairs.md`). The list is fetched only when you open Community
on the Crosshair page, kept 5 minutes, and pressing a crosshair there only loads it into the
editor. When you then Apply it unchanged, Sigil counts one use for it; nothing else is sent. To
remove a crosshair you published, load that saved preset and press Remove from library on the same
PC: the app keeps the delete token the Worker gave it in `crosshair-library.json` in its data
folder (readable only by you on macOS and Linux, never logged and never part of telemetry or
product data), and the row is gone at once. If you lose that token (a new PC, cleared app data),
ask the owner to remove it. The same file lists the crosshairs you reported, so they stay hidden
from your list. Crosshairs that five networks report are hidden until the owner reviews them.
Offline mode and the proxy in Settings apply to every library request.

## Catalog search index

Sigil's Worker builds a public search index of the GameBanana catalog so the app can search
offline and tolerate typos. See `docs/decisions/0018-typesafe-catalog-tags.md`.

Sigil downloads a public search index of GameBanana mods from its own server; the request
carries nothing about the player.

- What it holds: GameBanana's own public text per mod (title, description words, category, hero
  subcategory, author) and, when the owner turns term expansion on, verified search terms
  proposed from that text. Nothing from you or about you.
- The download: `GET /v1/catalog/tags` on Sigil's Worker, at start when the saved copy is older
  than three hours and at most every three hours after, with the saved ETag so an unchanged
  index is not sent again. Only the usual `x-sigil-client` and version headers and the
  `Sigil` user agent go with it. It goes through the proxy you set and never runs in offline
  mode. It is saved under `catalog-index/` in the app data directory.
- Search stays on your PC. Browse matches what you type against the saved index as you type
  (`rust/crates/deadlock-core/src/catalog_index/matcher.rs`, ported from
  `services/telemetry/src/catalog/match.ts`). What you type is never sent for this. To show the
  matches as cards, the app reads the cards it does not have yet from GameBanana by mod id
  (`Mod/Multi`, one request for up to 50 ids, after typing pauses): the request carries ids,
  never the text. With no index yet, outside the Mods section, or with the Featured filter on,
  Browse uses GameBanana's title search as before (see GameBanana).
- Two optional steps are built but off (`CATALOG_EXPANSION`, `CATALOG_FILE_LABELS`). If the owner
  turns one on, the Worker sends public mod page content only (titles, categories, descriptions,
  file names, sizes and notes, and search terms proposed from those) to Cloudflare Workers AI (the
  company that already runs the Worker) and to TypeSafe (`api.typesafe.ai`, the Jev model).
  TypeSafe states it does not train on customer requests and lists its data processing
  agreement, privacy policy and zero data retention option on its
  [Models page](https://docs.typesafe.ai/models.md#data-handling) and
  [Legal page](https://docs.typesafe.ai/legal.md). Nothing from the product, crash, diagnostics
  or hash tables is ever sent: an allowlist in `services/telemetry/src/catalog/typesafe.ts` refuses
  any other field, and tests check that a player's search text stored on the Worker never appears.
- What the Worker stores: per mod the public text it read, a hash of it and the time
  (`catalog_mods`), the optional judgments with their probabilities (`mod_tags`), and its run
  state and spend tally (`enrich_state`). No player data.
- Status: the app downloads `GET /v1/catalog/tags` by default (owner approved 2026-11-25). It
  needs no consent because the request carries no id and no product data, only the usual
  `x-sigil-client` and version headers, and the answer is the same for everyone. The app does
  not call `/v1/catalog/mod/<id>`.

## Model provider

Release builds leave the mod agent out (`ROADMAP.md`), so they never contact a model provider. This
section applies to a developer build with the agent (`--features agent`).

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

- Browse and search use anonymous GETs. When the catalog search index is on your PC, search in
  the Mods section is matched locally and only the result ids are sent (`Mod/Multi`) to read
  their cards. Otherwise (no index yet, Sounds, WiPs, the mixed feed, the Featured filter) the
  search text you type is sent as GameBanana's name filter.
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

## Stats pictures

With Player stats on, the Stats page shows pictures for what it lists: rank badges from
`api.deadlock-api.com`, item icons from `assets-bucket.deadlock-api.com` (run by deadlock-api) and
Steam avatars from `avatars.steamstatic.com` (Valve). Each request is a plain GET of the image URL
deadlock-api named; it carries no account id, cookie or key beyond what the URL itself is. Sigil
refuses any other host for these pictures. Each picture is scaled down and kept in `cache/stats/art`
for 30 days, so it is asked for at most once a month. With Player stats off nothing is fetched. For
accounts signed in on your PC, the start page shows the avatar Steam already keeps on disk
(`config/avatarcache`), without a request.

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
| Help build Sigil (product data) | Settings > Privacy | Off |
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

- Help build Sigil: the monitor refresh rate is not read (sent as 0).
- Offline mode does not cover the agent host's provider calls (`rust/crates/agent-host`),
  subscription sign-in (`desktop/src/signin.rs`) or publish
  (`desktop/src/publish.rs`).
- A proxy is not used by every download path yet (mod variants and Flatpak updates do not use it).
- Mod descriptions can load images from the host the author chose.
