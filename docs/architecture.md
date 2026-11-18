# Architecture

Status: draft. Boundaries below are proposals until recorded in `docs/decisions/`.
Updated: 2026-11-17

This document is the map. The executable rules live in code, especially
`packages/deadlock-tools/src/validate.ts` and the approval hooks in the agent host.

## Product shape

The user runs a desktop app, opens a mod project, and talks to an agent. The agent reads the game,
extracts what it needs, edits files, builds, packs, validates, and asks for approval before touching
the game install or publishing anything. A CLI exposes the same runtime without the UI.

Three promises shape every boundary:

1. The model never gets a credential. BYOK keys stay in the Rust app and are injected into
   provider requests there or into the isolated agent host, never into a transcript.
2. An agent tool never writes to the game install. It can write the mod workspace and produce a
   plan or a validated artifact. A main-process action, behind an approval gate, performs the
   deploy.
3. A payload that violates the community rules cannot be deployed, regardless of what the prompt
   says. Validation is code.

## Mod capability ladder

The workbench grows along one ladder. Every tier uses the same loop: search the game, extract,
edit, build, validate, deploy with approval. Complexity changes the tools and the review, not the
safety gate.

| Tier | Example | What it needs | Status |
| --- | --- | --- | --- |
| 0 Inspect | Find Billy's model and the texture that paints his jacket | Game index, search, extract | Implemented in the CLI and agent tools. Verify against a real install. |
| 1 Asset swap | Recolor a texture, replace a sound or a HUD image | Decompile, edit, compile, pack, validate, deploy | Pack, validate, release and deploy work. Decompile and compile need external backends. |
| 2 Material and effect work | Change a material, particle or camera | Tier 1 plus format-aware editing | Planned |
| 3 Character work | Restyle a hero, or base a look on a reference photo | Tier 2 plus skeleton-compatible mesh, materials, likeness review | Planned. Gated by `docs/decisions/0005-reference-images-and-likeness.md`. |
| 4 Multi-asset work | A themed set across heroes, UI and audio | Build graph, batch validation, versioned workspace | Planned |
| 5 Release and survive updates | Publish to GameBanana, migrate paths after a patch | Versioned release, upload approval, path migration | Local release, deploy, migration and GameBanana handoff implemented. Direct API upload is not. |
| 6 Living workbench | Watch patches, explain breakage, propose migrations, manage a catalog of mods | Patch feed, baseline diffs, impact rules, batch checks, review queue | Watch queue, patch feed, diff, impact and migration implemented. Opt-in watch loop writes durable state; an OS service is not packaged. |

The first useful milestone is not a generated character. It is a texture recolor that travels the
whole loop and shows up in game. Each later tier reuses that loop.

## Processes

```text
+---------------------------------------------------------------+
| Rune desktop app (Rust)                                        |
| chat, projects, index, patches, impact, approvals, build panel |
| owns credentials, game writes, file dialogs, mod workspace     |
+------------------------------+--------------------------------+
                               | local IPC (JSON)
+------------------------------v--------------------------------+
| Agent host (Node utility process)                              |
| TinyAgent host, tools, JSONL sessions                            |
+------------------------------+--------------------------------+
                               | ExecutionEnv
+------------------------------v--------------------------------+
| Mod workspace                    External binaries             |
| sources, staging, build output   vpkeditcli, Source2Viewer CLI, |
|                                  vpkmerge, resourcecompiler     |
+---------------------------------------------------------------+
```

Why a separate agent host:

- A crashed or wedged agent does not take the UI with it. TinyAgent reopens the same session
  journal and resumes unfinished work.
- The app can attach to the durable view over IPC, so reconnects are cheap.
- The Rust app keeps the only copy of decrypted credentials and the only code path that writes to
  the game install.

The agent host is not a security sandbox by itself. It is a crash and responsibility boundary. The
security boundary for game writes is the Rust app plus the approval gate.

## Agent runtime

The agent host is TinyAgent TypeScript v2, vendored at a pinned revision (MIT). It provides:

- Durable sessions as an ordered journal of JSON records with fenced appends.
- Resume after a crash. A recorded tool call reruns only when it is marked idempotent; otherwise the
  model is told it may have partly run.
- Pause and resume for human approvals, questions and client-executed tools.
- `beforeTool` and `afterTool` hooks for policy, redaction and approval gates.
- OpenAI Responses, Anthropic Messages and OpenAI-compatible Chat Completions, including local
  servers and gateways.
- Streaming events and a replayable journal.
- Typed output, limits, retries and compaction.

Mapping to the product:

| TinyAgent | Product |
| --- | --- |
| Session | One mod project, plus a root workbench conversation |
| Tool | A Rust CLI operation such as index, search, extract or project check |
| `pause` | An approval gate or a question to the user |
| `beforeTool` | Tool policy and the deploy, launch and publish approval gates |
| Event stream | The workbench timeline and tool cards |
| `FileStorage` | `<data>/agent-sessions/`, one journal per session |
| Provider | BYOK provider or a local OpenAI-compatible endpoint |

The host speaks JSONL over stdio. The Rust app owns the window, credentials, approvals UI and game
writes. The choice is recorded in `docs/decisions/0007-tinyagent-host.md`; the earlier Pi Durable
choice is superseded.

TinyAgent's Rust implementation is protocol v2 in `rust/crates/tinyagent`. `rust/crates/agent-host`
is the host the Rune app spawns, as its own executable with the `agent-host` subcommand (a separate
child process; see `decisions/0010-one-executable.md`). The TypeScript host is a reference implementation
only. fx remains an optional external lane, not a dependency.

One app instance owns the session directory. Do not point two hosts at the same journal directory.

## Tool catalog

Implemented today:

| Tool | Effects | Idempotent | Approval |
| --- | --- | --- | --- |
| `game_status` | Reads Steam libraries, `gameinfo.gi`, addon folder | yes | none |
| `game_index` | Builds the catalog for citadel, core and maps VPKs | yes | none |
| `search_paths` | Filters the catalog by text, hero, type, folder or VPK | yes | none |
| `extract_paths` | Copies exact game paths into the workspace | no | none (workspace only) |
| `project_check` | Compares project targets with the current catalog and baseline | yes | none |
| `patch_notes` | Fetches and reads the official Steam patch feed | no | none |
| `validate_payload` | Pure path rules over a proposed entry list | yes | none |
| `project_validate` | Checks the staging tree against payload rules | yes | none |
| `project_pack` | Writes a VPK and chunks in the workspace | no | none |
| `project_release` | Writes a versioned release manifest | no | none |
| `deploy_plan` | Computes slot, conflicts, replacement and gameinfo repair | yes | none |
| `project_migrate` | Proposes moved target paths | yes | none |
| `watch_projects` | Reviews every project after a patch | yes | none |
| `publish_prepare` | Builds a release folder and metadata | no | none |

Planned, in rough dependency order:

| Tool | Effects | Idempotent | Approval |
| --- | --- | --- | --- |
| `game_decompile` | Runs an external decompiler into the workspace | no | none (workspace only) |
| `game_index_watch` | Detects a game update and invalidates stale paths | yes | none |
| `vpk_inspect` | Reads a VPK manifest and verifies CRCs | yes | none |
| `vpk_merge` | Writes a merged VPK in the workspace | no | none |
| `addon_deploy` | Copies a VPK into the game install | no | required |
| `game_launch` | Starts the game | no | required |
| `gameinfo_repair` | Restores the `addons` SearchPaths line | no | required |
| `publish_upload` | Sends files to GameBanana | no | required |
| `knowledge_search` | Searches curated modding notes | yes | none |

Rules for adding a tool:

- Mark a tool `idempotent: true` only when repeating the same call with the same execution id is
  safe. Reads and pure computation qualify. Writes, process launches and external calls do not.
- Model-facing output stays small and text. Large results go to workspace files and are referenced
  by path.
- Anything that leaves the mod workspace goes through the Rust app. The tool returns a plan, not a
  side effect.
- Approval-gated tools pause through `beforeTool`. The run resumes with the user's decision.
- Every new contract gets a test in the owning Rust crate or TypeScript package.

## Game index

`game_index` discovers `game/citadel/pak01_dir.vpk`, `game/core/pak01_dir.vpk` and
`game/citadel/maps/*_dir.vpk`, then writes one metadata cache per source under
`<workspace>/.dah/index/`. It never copies a whole VPK into memory.

- The cache records the source path, size and modification time. A mismatch rebuilds it and reports
  a diff against the previous cache: added, removed, changed and likely moves.
- Search is a pure function over the catalog. Filters compose: text, hero codename, compiled file
  type, folder prefix and VPK.
- Hero codenames come from paths such as `models/heroes_wip/billy/`, not from a bundled table that
  goes stale after a patch.
- Extraction re-reads the VPK tree, verifies the entry CRC32 and writes under the workspace. It
  refuses absolute paths, parent segments and destinations outside the workspace.
- `DAH_GAME_DIR` points at a game folder, its `game` folder or `game/citadel` for non-Steam installs
  and fixtures.
- VPK format facts and sources are in `docs/research/vpk-format.md`.

Extraction returns compiled `_c` files. Decompiling them is a separate backend that is not bundled
yet.

## Update survival

A mod that worked yesterday is a compatibility problem, not an agent bug. The workbench keeps three
records:

1. A catalog of game paths from citadel, core and maps VPKs.
2. A project baseline: target paths, CRCs, owning VPK, slot and `gameinfo.gi` state.
3. The official patch feed, with the newest post date.

The loop after an update:

1. Fetch the official patch notes. Mark new posts since the last check.
2. Rebuild the catalog. Diff it against the previous cache: added, removed, changed and likely moves
   by unique basename.
3. Check the project against the baseline and the new catalog. Report unchanged, changed, missing,
   moved and unbaselined targets, plus a missing `addons` SearchPath and a slot collision.
4. Propose a migration: exact new path, recompile, re-extract, or a human decision.
5. Never deploy automatically. A migration is a plan until the user approves it.

Correlating a patch note with a path change is a hint, not proof. Valve's notes omit file moves and
compiled changes. The catalog diff and the baseline are the evidence.

## Build pipeline

```text
source file -> build backend -> compiled _c file -> staging -> VPK -> validation -> deploy
```

Stages:

1. **Locate.** Find the game install and the exact game path. Hero folders move between updates, so
   path lookup reads the installed VPK index, not a hard-coded table.
2. **Extract.** Pull the original file out of the game VPK. Decompile it when needed.
3. **Edit.** The agent works inside the mod workspace with normal coding tools.
4. **Compile.** Run the build backend. CSDK 12 and `resourcecompiler.exe` are Windows-only. On
   macOS and Linux this stage is unavailable unless a verified remote or Wine backend exists.
5. **Pack.** `vpkeditcli` or the Source2Viewer CLI creates `pakNN_dir.vpk`. Chunk sets are used above
   the 2 GB single-file limit. Packing is cross-platform.
6. **Validate.** Check every entry path: compiled only, exact game path, no banned content, no
   duplicates, expected slot name. Validation is a hard gate.
7. **Deploy.** Choose a slot, check conflicts with existing mods, back up nothing that is already
   safe, and copy the VPK. Main process only, approval required.
8. **Verify.** Launch the game and report what to look at. Automated visual verification is out of
   scope for the first milestones.

### Build backend interface

```ts
export type BuildBackendInfo = {
  id: string;
  label: string;
  available: boolean;
  reason?: string;
  platform: "win32" | "darwin" | "linux" | "remote";
};

export type CompileRequest = {
  workspace: string;
  sourcePaths: readonly string[];
};

export type CompileResult = {
  compiledPaths: readonly string[];
  log: string;
};

export interface BuildBackend {
  info(): Promise<BuildBackendInfo>;
  compile(request: CompileRequest): Promise<CompileResult>;
}
```

First implementation: `LocalWindowsBackend` using `resourcecompiler.exe -i <file>` with
`-game <path to gameinfo.gi>` for files outside the content folder. Second: `RemoteBackend` over a
Pi execution environment. `UnavailableBackend` returns a clear reason instead of pretending.

Do not add a downloader that fetches a compiler from an arbitrary URL. Tool paths come from
settings. Versions and hashes are pinned in the notices and settings schema when a binary is
bundled or downloaded from a documented source.

## Safety model

Enforced in code:

1. **No game writes from tools.** Deploy, `gameinfo.gi` changes, launch and publish are
   main-process IPC handlers with an approval gate.
2. **Payload rules.** `validatePayload` rejects empty payloads, absolute or parent paths, staging
   prefixes, source assets, tool caches, `scripts/*.vdata_c`, `*.vnmskel_c`, executables and
   duplicate paths.
3. **Slot rules.** Only `pak01_dir.vpk` through `pak99_dir.vpk` in the addons folder. A target slot
   that already exists requires an explicit replace decision.
4. **`gameinfo.gi` scope.** Only the `SearchPaths` block is allowed to change. The app backs up the
   file, records the exact edit, and can restore Valve's version. Anything else is refused. One
   exception: a Performance config edits ConVars and engine sections on the user's Apply, with every
   line tagged `sigil-perf` so Turn off restores the file byte for byte
   (`deadlock_core::performance`). It never touches `FileSystem`/`SearchPaths`.
   Joining a Deadworks server adds up to five marked `SearchPaths` lines (`Game
   citadel/deadworks_mods`, `addonroot citadel/deadworks_addons`, and `Mod`/`Write citadel` when
   absent) after the user confirms, with a backup (`deadlock_core::servers`).
5. **Secrets.** Decrypted keys exist only in main. The agent host receives provider auth through a
   narrow handshake, not the transcript. Tool output and logs are scrubbed for known key shapes.
6. **Network allowlist.** Provider endpoints, `api.steampowered.com`, `api.deadlock-api.com`, GameBanana, the GitHub update check and signed update download, Deadworks for Servers, the local Discord IPC socket, and Sigil's own telemetry Worker (opt-in, off by default). No other
   outbound calls from tools without a documented allowlist entry.
   - `gamebanana.com/apiv11`, `images.gamebanana.com` and `files.gamebanana.com`: Browse.
     Anonymous GETs, no key, no user data: `Game/20948/Subfeed?_nPage=N&_csvModelInclusions=Mod|Sound|Wip`
     for the mixed feed, `<Model>/Index` (`_aFilters[Generic_Game|Generic_Category|Generic_Name|Generic_WasFeatured|Generic_Submitter]`,
     `_sSort=Generic_*`) once a filter, a search or another order is picked (the next page as the
     grid scrolls near its end), `<Model>/Categories` and `<Model>Category/<id>/SubCategories`
     for the category and hero pickers (once per session), `Game/20948/TopSubs` for the
     dashboard rails, `Collection/Index?_aFilters[Generic_Game]=20948` for the Collections tab,
     `Collection/<id>/Items` when a collection opens, `<Model>/<id>/ProfilePage`, `Posts` and
     `Updates` when a submission is opened (plus later `Posts` and `Updates` pages on Load more),
     `Mod/<id>?_csvProperties=_aRequirements` for the requirements panel, and preview images,
     hero portraits and avatars. `<Model>` is `Mod`, `Sound` or `Wip`. The search text the player
     types goes to GameBanana as the name filter. Images are cached under the app data directory
     (`cache/images`). A sound's sample downloads only when the player presses Play, into memory
     for the preview player; nothing is written to disk. Favorites (`browse/favorites.json`) and
     hidden items, ignored updates, filters and view options (`browse/state.json`) stay local.
   - `gamebanana.com/apiv11` background catalog sync: Browse keeps a bounded local index under
     `browse/mirror/` (`entries.jsonl`, `meta.json`, 40,000 rows and 24 MB caps). Every five
     minutes while Browse is open it reads two `<Model>/Index?_sSort=Generic_Newest` pages per section,
     walks back through the catalog incrementally and prunes the oldest rows. The index answers
     offline browse, the creator rail and custom date ranges. It runs only while Browse is open,
     stops in offline mode, and sends nothing but the public page and filter parameters.
   - `gamebanana.com/apiv11/Mod/<id>?_csvProperties=_idRow,_aFiles`: Installed update checks. One
     anonymous GET per installed GameBanana mod when Installed opens, at most every six hours
     (cached in `mod-updates.json` in the app data directory), or on "Check for updates". The
     request carries only the public mod id. An update downloads through the install pipeline.
     The same request lists a candidate's files when Identify runs a CRC check on an unknown mod
     (`mod_lookup::crc_detect`, wave 4), which then reads only the end of each candidate ZIP with
     at most two `Range` GETs on the GameBanana file hosts (the install allowlist applies on every
     redirect; at most 40 archives per run). Nothing is downloaded in full; the request carries
     only public mod and file ids. The name search for an unknown mod is the Browse `Mod/Index`
     name filter above.
   - `api.github.com` and `raw.githubusercontent.com`: Performance configs. Anonymous GETs of public
     files, no user data. `api.github.com` resolves the newest release of
     `Sqooky/OptimizationLock` (newest commit) and `dacooderr/OptiLock` (latest release tag) and
     lists the files at that commit; `raw.githubusercontent.com` serves the config files at that
     commit, each checked against its git blob hash. Valve's convar list
     (`SteamTracking/GameTracking-Deadlock`, `DumpSource2/convars.txt`) comes from the same hosts.
     Runs when the Performance page opens and the cache is older than 12 hours, or on the refresh
     button. Results are cached under the app data directory (`performance/`), so the page works
     offline after the first fetch.
     `api.github.com` also serves the update check: one anonymous GET of
     `/repos/superuserkalo/sigil/releases/latest` (Stable) or `releases?per_page=10` (Preview),
     from Settings > About and updates, at startup when the user turned "Check for Sigil updates"
     on (off by default), or on the interval chosen beside it. No identifier, cookie or install id
     is sent; the User-Agent is `sigil-updater`.
   - `github.com/superuserkalo/sigil/releases/download/...`: Sigil updates. When the check finds a
     newer release, Sigil fetches `sigil-update.json` and its `.sig` (a few KB) and, if the
     Ed25519 signature verifies against the key compiled into Sigil, downloads this platform's zip
     into the app data directory (`updates/`). GitHub answers these with a redirect to its release
     asset host (`release-assets.githubusercontent.com`, formerly `objects.githubusercontent.com`),
     which serves the file. Only URLs under the repository's release download path are fetched.
     Size and SHA-256 are checked against the signed manifest before anything is staged; the swap
     happens only on "Restart to update" or the next launch, and never touches the game install.
     Offline mode blocks all of it. See `docs/runbooks/releasing.md`.
   - `api.deadlock-api.com`: Stats (public match and player data for the account the user enters)
     and Crosshair share codes. "Copy code" sends only the crosshair convar values, as a GET to
     `/v1/crosshair/settings/code`, and only when the user presses it. Import sends a pasted
     `DL.` code to `/v1/crosshair/code/settings` only when the text has the shape of a code and
     the user presses Import; pasted console commands are parsed locally. Both follow the
     share codes switch in Settings > Privacy.
     Match data sharing (opt-in, off by default, owner-approved 2026-11-17): Settings > Privacy >
     "Share match data with deadlock-api". While on, Sigil reads the first 512 bytes of each file
     in Steam's HTTP cache (`<Steam>/appcache/httpcache`), keeps URLs shaped
     `replayN.valve.net/1422450/<match>_<salt>.(meta|dem).bz2`, and POSTs the documented
     `/v1/matches/salts` body: `match_id`, `cluster_id`, `metadata_salt` or `replay_salt`, and
     `username: "Sigil"`. No account id, Steam login, file or other data is sent. It runs every
     15, 30 or 60 minutes while Sigil is open, or on "Share now"; each batch is logged in
     `<data>/match-sharing.json` and shown in Settings; "Forget" deletes that log.
     `deadlock_core::salts` holds the scan and the request.
     Stats views also read `/v1/analytics/item-stats` (the player's item win rates and
     average buy time, plus the unfiltered baseline the delta is measured against),
     `/v1/analytics/hero-comb-stats` (pair and trio records for the Party view),
     `/v1/players/hero-stats` with the same ranked, unranked and start-date filters, and
     `/v1/matches/{match_id}/metadata?disable_steam=true` for one match's item purchase
     timeline. The only identifiers sent are the account ids the user entered or picked.
     Responses are cached under the app data directory (`cache/stats`).
   - `gamebanana.com/apiv11/Member/<id>/ProfilePage`: one anonymous GET when the user links a
     GameBanana account in Settings > Integrations, to read the public member name. The member
     id is stored in `settings.json`. The same path is read once when a mod's details open the
     author's contacts (`library_facts::fetch_author_profile`): the author's member id from the
     mod page, nothing about the user. Only https links on each platform's own host are shown.
   - `gamebanana.com/apiv11/RemoteInstall/<member id>/<token>/<manager alias>`: Remote Install
     (`deadlock_core::remote_install`). While a pairing exists and Sigil is open, one GET every
     five minutes, or on Check now, reads the member's queued mods; each read marks the returned
     entries retrieved on GameBanana, which is the only acknowledgement the protocol has. The
     token comes from GameBanana's Pair button (the `registerKey,<member id>,<token>` link) and
     stays in the OS keychain; it travels in this URL and nowhere else. The manager alias is
     `Sigil`, a placeholder until GameBanana enables Remote Install for it: until then the read
     answers an `_sErrorCode`, which the app shows. `Member/<id>/ProfilePage` is also read once
     when a pair link arrives, to show the account name before the user links it. The queue
     itself names only GameBanana mod and sound ids and a one-click link on a `gamebanana.com`
     host; the download goes through the existing install pipeline.
   - The release check also reads `/repos/<owner>/<repo>/releases?per_page=1` when the user picks
     the Preview channel in Settings > About and updates.
   - Discord Rich Presence (opt-in, off by default, owner-approved 2026-11-17): no network. Sigil
     writes to the local Discord client's IPC socket (`discord-ipc-N` in `$XDG_RUNTIME_DIR`,
     `$TMPDIR`, `$TMP`, `$TEMP` or `/tmp`; the named pipe `\\?\pipe\discord-ipc-N` on Windows):
     a HANDSHAKE with Sigil's application id, then SET_ACTIVITY with the page the user is on,
     optionally the enabled mod count and whether the game runs modded or vanilla, and a session
     start time. "Show what you are doing" off sends only "Using Sigil". Discord itself shows the
     activity to the user's friends. See `screens/settings/discord.rs`.
   - Sigil's telemetry Worker (opt-in, off by default, owner-approved 2026-11-18): the one host in
     `deadlock_core::telemetry` (`ENDPOINT`, `https://sigil-telemetry.gantchevkaloyan.workers.dev`). Nothing is sent until the player agrees in the first-start
     dialog, and each kind has its own switch in Settings > Privacy:
     - `POST /v1/events`, with usage consent: `installId` (a random UUID kept in
       `<data>/telemetry/install-id`, not a machine id), app version, OS and up to 50 allowlisted
       events (`name`, optional `page`, `ts`). Queued locally, 500 events, oldest dropped.
     - `POST /v1/crash`, with crash consent: `installId`, version, OS, the panic message and a
       backtrace. The panic hook writes a scrubbed file locally first, with or without consent.
     - `POST /v1/diagnostics`, only when the player presses Send: `installId`, version, OS, a
       summary and a scrubbed log tail under the Worker's 1 MB cap. The Worker answers with a
       reference code.
     - `GET /v1/identify?sha256=<64 hex>&size=<n>`, with hash lookup consent: the hash and size
       only; the Worker returns GameBanana mod and file ids. No install id is sent and nothing is
       stored about the caller.
     - `POST /v1/hashes`, with hash lookup consent: the sha256, size and GameBanana mod and file
       ids of a file just installed, so a later lookup can identify it.
     Every request carries `x-sigil-client: 1` and `x-sigil-version`, goes through `net::gate`
     (so Offline mode blocks it) and has a short timeout. Failures are one log line. Home paths,
     user names, keys, tokens, emails and Steam ids are scrubbed on the device and again by the
     Worker. See `docs/decisions/0014-opt-in-telemetry.md`.
   - Offline mode (Settings > Network) blocks every request the core makes: each `ureq::Agent` in
     `deadlock-core` carries `net::gate` as middleware, and `DAH_OFFLINE=1` carries the switch
     to the helper processes Sigil starts. Not covered yet: the agent host's model provider
     calls (`rust/crates/agent-host`, `apps/agent-host`), subscription sign-in
     (`desktop/src/signin.rs`) and publish (`desktop/src/publish.rs`), which do not go through
     the core gate.
   - An HTTP, HTTPS or SOCKS5 (`socks5://`, or `socks5h://` to resolve names on the proxy) proxy
     from Settings > Network applies to GameBanana browsing and mod downloads
     (`net::with_proxy`). Without one, ureq honours `HTTPS_PROXY` from the environment.
   - `api.deadworks.net`: Servers. Anonymous GETs, no user data: `/api/steam/servers?deadworks=1`
     (Steam's server list for Deadlock, filtered to Deadworks servers) and `/api/servers` (the
     Deadworks registry: country, version, roster) when the Servers page opens or on Refresh;
     `/api/bootstrap` and the UI pack it names (`/api/bootstrap/v/<n>.vpk.bz2`) when the user
     opens a join. See `docs/decisions/0009-deadworks-servers.md`.
   - Listed Deadworks game servers, UDP: one A2S_INFO ping per listed server on the Servers page,
     and an A2S_RULES query to the one server the user opens a join for.
   - The download host a Deadworks server advertises (`dw_fastdl`, any host): only after the user
     confirms a join that needs content, and the host is shown in the confirm dialog. Every
     connection resolves through a guard that refuses loopback, private, link-local and reserved
     addresses (unless the game server itself is local), at most five redirects, 4 GiB per file and
     16 GiB per join, VPK checks before anything is installed.
   - GameBanana file servers (parity round, 2026-11-17): `gamebanana.com/apiv11/Util/Fileservers`
     (the server directory, when Settings > Mods opens or on Refresh status), and a speed test on
     "Test now" only: the newest file of GameBanana's top Deadlock mod (`Mod/Index`, its
     `ProfilePage`, a HEAD of `gamebanana.com/dl/<id>`) and a 256 KiB range read from at most
     three `filecacheN.gamebanana.com` servers. A chosen or fastest server rewrites download
     redirects to that `*.gamebanana.com` host (`net::route_download`), falling back to the one
     GameBanana picked. "Faster downloads" sends parallel Range requests to the same host.
   - `gamebanana.com/apiv11/Mod/<id>?_csvProperties=_aRequirements`: one anonymous GET when the
     install panel opens for a mod and "Show what a mod needs" is on.
   - A mod source the user adds (Settings > Mods > Sources): the descriptor URL the user pastes
     and only the `api`, `downloads` and `media` origins it declares, https only, never a private
     or local address. Sigil shows every origin and asks before the source is saved; it syncs
     only on add or Sync again, and downloads only when the user presses Install. Nothing about
     the user is sent.
   - `api.divineskins.gg`, `lol-mods.divine-cdn.com`, `lol-assets.divine-cdn.com` and
     `images.divine-cdn.com`: Divine Skins installs. Backlogged (2026-11-18): the core module
     exists but nothing in the app calls it yet, so Sigil makes none of these requests today. A pasted Divine Skins page or Deadlock Mod
     Manager link reads the public one-click manifest from `api.divineskins.gg`, plus the public
     release list when the link names no release; the manifest may name an archive on
     `lol-mods.divine-cdn.com` and a cover image on `lol-assets.divine-cdn.com` or
     `images.divine-cdn.com`. Anonymous GETs, no account and no user data. See
     `deadlock_core::divine_skins`.
   - Connection diagnostics and Test connection (Settings > Network, on a press only): HEAD
     requests to GameBanana, `files.gamebanana.com`, `api.deadlock-api.com`, `api.github.com`,
     `raw.githubusercontent.com` and `api.deadworks.net`, through the proxy when one is set.
     The diagnostics list (`diagnostics::run`, wave 4, DMM's checks) sends one small GET each to
     the GameBanana API, `files.gamebanana.com`, the Steam news feed on `api.steampowered.com`,
     `api.deadlock-api.com` and `api.github.com`, and resolves those host names. It does not use
     DMM's Cloudflare trace probe, which is not on this list.
   - `api.github.com/repos/superuserkalo/sigil/contributors`: one anonymous GET when About >
     Contributors opens.
   - The proxy sign-in (Settings > Network) is kept in the OS keychain (macOS `security`,
     Windows Credential Manager, Secret Service through `secret-tool`); only the user name is in
     `options.json`. A bypass list skips the proxy for the hosts it names.
   - DeadlockForge 1-click installs (opt-in, off by default): an inbound HTTP listener on
     `127.0.0.1` ports 43110 to 43114, no outbound call. It answers only the
     `https://deadlockforge.net` and `https://www.deadlockforge.net` origins with a loopback
     `Host`, takes one VPK (1 KiB to 512 MiB, VPK magic checked), and installs nothing until the
     user presses Install in Sigil.
   - Stat sites (Settings > Integrations: Statlocker, DeadChaps, Buildlock, DLTV, EDL) open in
     the system browser on a press; Sigil makes no request to them.
7. **Exec allowlist.** The desktop wraps `bash` with a workspace boundary and a binary allowlist.
   The CLI keeps the plain tool for local development until the wrapper lands.
8. **Untrusted input.** Mod descriptions, wiki pages and GameBanana metadata are data, not
   instructions. Tools that fetch them must not forward them into a privileged path. The approval
   gate is the backstop.
9. **Reference images and likeness.** Reference images stay local and are never shipped or used to
   train a shared model. A likeness mod records the rights basis and needs a named approval before
   publish. See `docs/decisions/0005-reference-images-and-likeness.md`.

Cosmetic-only policy is both an ethical rule and a compatibility rule: gameplay data goes stale
after updates and can break other players' mods.

## Data layout

Desktop userData:

```text
agent-sessions/       TinyAgent session journals, one per conversation
credentials.json      safeStorage-wrapped provider credentials
settings.json         model choice, toolchain paths, workspace root, UI settings
mods/<slug>/          mod workspace, git-friendly
  project.json        manifest: name, target paths, slot preference
  sources/            decompiled sources the agent edits
  staging/            compiled _c files at game paths
  builds/             packed VPKs and validation reports
  notes/              agent notes and screenshots
  extracted/          raw game files pulled out of the VPK
  .dah/index/         cached game path index, safe to rebuild
logs/                 redacted app and tool logs
```

CLI data directory follows the OS convention (`~/Library/Application Support` on macOS,
`%APPDATA%` on Windows, `~/.local/share` on Linux), overridable with `DAH_DATA_DIR`.

A mod workspace is deliberately a normal directory. Users can open it in an editor, version it, or
hand it to another agent.

## Desktop app

The desktop app is native Rust with Rune. It carries its own design language on every platform,
with GPU rendering, springs and headless snapshots. The shell is not Grimoire and has no browser
renderer.

Screens and responsibilities. Sidebar order follows `docs/design/mod-manager-shell.md`, and a screen
ships only when core can back it with real data or a gated action:

- Library: Installed (addon slots on disk), Browse (GameBanana feed), Conflicts (shared paths),
  Updates (project baselines against the patch feed).
- Modding: Profiles (captured slot sets), Autoexec (the config file behind approval), Stats (the
  public API plus local demos), Locker (hero renders and asset paths), Launch (modded, vanilla,
  restore).
- Workbench: Agent, Projects, Game Index, Patches, Impact, Release.
- Foundry: Toolchain (external decompile and compile tools), Character (reference to mesh, stopped
  at a verified artifact).
- Settings: BYOK provider status, local model endpoint, toolchain paths, workspace location.

The default surface is Installed, because this is a mod manager first. The agent is one screen inside
the same shell, not the product shell.

The Rust app owns credentials, settings, file dialogs, mod workspace access, game install writes and
approval decisions. It talks to the TinyAgent agent host over local IPC. It never loads the
transcript or credentials into a web context because there is no web context.

Grimoire remains a reference for Steam discovery, addon slot handling, VPK merge and preview ideas,
and its MIT attribution stays in `THIRD_PARTY_NOTICES.md`. See
`docs/decisions/0006-native-rust-rune-desktop.md`.

Rune is pinned to a dev revision and vendored by `scripts/vendor-rune.sh`. The app repeats Rune's
`[patch.crates-io]` entries in `rust/Cargo.toml`; a plain git dependency does not resolve its
patched text stack.

## CLI

`dah` is the headless surface. The Rust CLI is `rust/crates/dah-rs`; the TypeScript CLI remains
during the port.

```text
dah doctor                 game install, gameinfo, slots, providers
dah models                 providers, auth status, available models
dah auth set <provider>    store a key, chmod 600
dah auth list              provider ids and credential types, no secrets
dah chat "<prompt>"        one agent run
dah index                  build or load the game catalog for citadel, core and maps
dah find "billy"           search game paths by text, hero, type, folder or VPK
dah extract <path...>      copy exact game paths into the workspace
dah patches                official Deadlock patch notes and what is new
dah mod new <slug>         create a project manifest and workspace
dah mod validate           check staging against the payload rules
dah mod pack               pack staging as pakNN_dir.vpk plus chunks
dah mod release            write a versioned release manifest and history
dah mod baseline           record target paths, CRCs and index sources
dah mod check              show what changed, moved or broke after a patch
dah mod deploy             plan or approve an addon install
dah mod migrate            plan or apply moved target paths
dah watch [--watch]        review every project and correlate the newest patch
dah publish prepare        build a local release folder, never upload
dah publish upload         approval-gated GameBanana handoff with upload record
dah publish confirm        record and verify the submitted GameBanana URL
dah publish status         show the recorded upload state
dah addons list            installed slots and shared-path conflicts
dah addons set <n> on|off  rename one slot file, behind approval
dah profiles list|save|apply   named sets of enabled slots
dah autoexec show|set      the game config file, writes behind approval
dah launch modded|vanilla|restore|status   Steam launch and vanilla stash
dah toolchain status       external tool availability with reasons
dah mod decompile|compile  Source2Viewer-CLI and resourcecompiler.exe, plan first
dah stats player|matches|heroes|heatmap|leaderboard   public API reads
dah character create|status|stages|import   reference or prompt to a stage graph, then a GLB
dah vpk inspect            list a VPK tree and verify CRCs
```

The CLI is useful for CI, scripted batch work, and testing the runtime without the desktop app.

## Failure modes

- **Process death during a run.** Reopen the session journal and resume. A call marked idempotent
  reruns with the same execution id. Any other interrupted call is reported to the model as
  uncertain. A model request that was cut off is retried.
- **Agent host crash loop.** The sidecar supervisor stops restarting after repeated crashes. The UI
  reports it and keeps the journal.
- **Game update.** Hero paths move and `gameinfo.gi` resets. Detection: compare the project's target
  paths with the current VPK index. Repair: add the SearchPaths line back, then migrate paths.
- **Missing toolchain.** `doctor` reports the missing backend and the compile step is disabled with a
  reason. Never silently substitute a different compiler.
- **Slot collision.** Show the winning mod, the conflicting files, and ask before replacing a slot.
- **Corrupt or huge VPK.** Validate before deploy. Refuse > 2 GB without a chunk plan.

## Roadmap

Phase 0, scaffold (complete):

- Monorepo, docs, agent manuals, domain glossary.
- Working harness bootstrap and BYOK credential store.
- CLI `doctor`, `auth`, `models`, `chat`.
- Game discovery, `gameinfo.gi` status, addon slot status.
- Payload validator with tests.

Phase 0.5, native port (complete):

- Rust workspace, pinned Rune vendor and headless Rune app.
- Rust domain core: VPK, catalog, index, extraction, discovery, projects, patches and impact.
- Rust CLI with index, find, extract, patches and mod checks.
- Rune workbench screens: workbench, projects, game index, patches, impact and settings, with headless scenes.
- Rust TinyAgent protocol v2 crate: durable sessions, pause/resume, hooks, providers and events.
- Done: the Rune app spawns `dah-agent` and streams turns, tool events and approvals.
- Done: validate, pack, versioned release, deploy plan/apply, migration, watch and publish prepare.
- Done (2026-11-17): Settings parity with Grimoire and Deadlock Mod Manager
  (`docs/design/settings-parity.md`): search across categories, German UI strings, rotating
  redacted log, offline mode and proxy in the core HTTP paths, start with the computer, Discord
  Rich Presence (waits for the application id), opt-in match data sharing, Steam and GameBanana
  links, experimental flags. Not verified on Windows or Linux yet: the login entry, the named
  pipe and `tasklist`.

Phase 1, read-only game introspection (in progress):

- Done: index `pak01_dir.vpk` and cache entry paths.
- Done: search paths by hero codename, file type and folder.
- Done: extract exact game paths into the workspace with CRC verification.
- Done: CLI `dah index`, `dah find`, `dah extract` and agent tools for the same.
- Next: decompile extracted files through a configured Source 2 Viewer backend.
- Next: preview extracted textures and models in the existing Grimoire viewers.
- Next: verify entry counts and hero folders against a real install.

Phase 2, build loop without compile (in progress):

- Done: project manifest and staging layout.
- Done: native VPK writer, pack, inspect with CRC verification.
- Done: validate, versioned release, deploy plan, approved deploy to a test slot.
- Next: launch and a real in-game recolor acceptance run.
- Acceptance: recolor a texture whose source asset can be produced without a compiler, pack it,
  deploy it, and see it in game.

Phase 3, compile backends:

- Local Windows backend for textures, sounds, materials and UI.
- Remote backend over a Pi execution environment.
- Acceptance: edit a source asset, compile, pack, deploy, verify in game on Windows.

Phase 4, publish and survive updates (in progress):

- Done: release folder and metadata, publish prepare with install notes.
- Done: update detection, path migration including staging, `gameinfo.gi` repair, cross-project watch.
- Next: GameBanana upload with approval; the current handoff prepares the folder only.
- Acceptance: update a published mod after a game patch with a recorded diff.

Phase 5, stats and the living catalog (in progress):

- Done: public API player, rank, career, hero, match, leaderboard and heatmap reads.
- Done: API host added to the network allowlist; no key is required.
- Done: local demo container parser, merged API plus demo summary, and a linted review contract.
- Done: Locker hero and asset resolution from a local art root, with art-only heroes labelled.
- Done: Browse install pipeline: HTTPS allowlist, ZIP in process, 7z/rar through explicit tools,
  payload validation, free-slot plan and approval-gated apply. Verified with a local server and a
  live GameBanana ZIP; 7z and rar are unverified because those tools are absent here.
- Done: Installed slots toggle on and off behind a confirm dialog.
- Done: Crosshair editor with the HUD geometry, in-game import and a managed
  autoexec block; applied through the same approval and backup path.
- Done: Locker hero roster from the catalog merged with Grimoire art, a pinned
  project hero, and recolor briefs with exact targets and the remaining steps.
- Done: Servers: Deadworks list with pings, filters and a confirmed join (gameinfo.gi lines, UI
  pack, server content, `steam://connect`). Verified with unit tests and a live list, rules query,
  UI pack and addon install into a scratch root. Not verified: a full join on Windows.
- Next: real `.dem` files from a live match; positions need a serializer-aware extractor.
- Next: verify 7z and rar extraction on a machine with those tools.

Phase 6, character generation (in progress):

- Done: reference and prompt to GLB task pipeline with a recorded stage graph.
- Done: deterministic reference and GLB checks, and a Source 2 compile plan.
- Next: a local generative backend, retarget to a Source 2 skeleton, vmdl and AnimGraph binding.
- Blocked: no in-game character can be claimed until a real compile and an in-game check happen.

Phase 7, packaging:

- Done: `scripts/package-desktop.sh` stages the one executable (the CLI and agent host are folded
  in as subcommands, decision 0010) and a manifest with
  byte counts, SHA-256 and the excluded material. A macOS bundle is assembled and the release
  build was produced on this machine.
- Next: sign the macOS bundle, produce a Windows build on Windows, run `Sigil.exe cli doctor` and
  the `agent-host` child on Windows.
- Acceptance: a clean machine runs the app, finds the game, deploys a mod and restores vanilla.

## Open questions

- Which reference image source is acceptable: user upload only, a licensed stock provider, or a
  search API with attribution? Any choice updates the network allowlist and notices.
- Which image-to-3D backend, if any, produces a mesh that fits a Deadlock skeleton, materials and
  AnimGraph 2, and on which platform? Do not promise one before it compiles in game.
- Does the app support local OpenAI-compatible models, and how are they configured without leaking
  keys or adding a hosted proxy?
- Is CSDK 12 viable under Wine or Proton on macOS and Linux? Until verified, do not promise it.
- Can any part of the toolchain be redistributed? CSDK 12 has no license. Bundle only tools with a
  clear license and a pinned hash.
- Which GameBanana upload flow is acceptable: browser handoff, API key, or a prepared zip only?
- Do we keep Grimoire's social features, and does the fork inherit its companion server?
- Answered (decision 0010): the agent host is Rust and ships inside the app executable as the
  `agent-host` subcommand, so there is no Node install and no sidecar file to locate.
- How much of the prompt and knowledge pack ships in the app versus being read from `docs/`?
- What is the minimum viable visual verification after deploy?

## Non-goals

- Gameplay cheats, aim assistance, wallhacks, or anything that gives an advantage in matchmaking.
- Redistributing Valve assets.
- A hosted model proxy or a paid inference tier.
- Telemetry, analytics, or a heartbeat.
- Automatic public releases without a human decision.
