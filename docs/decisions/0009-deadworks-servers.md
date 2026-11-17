# 0009 Deadworks community servers are a joinable Servers screen

Status: accepted
Date: 2026-11-17

## Context

Deadlock is matchmade and Valve ships no server browser. Deadworks is a community server framework:
hosts run a Deadlock dedicated server with Deadworks, and players join them by address. Grimoire
(MIT) and Deadlock Mod Manager (GPL-3.0) both ship a Servers tab for it. The user asked for one
that takes the best of both.

`docs/design/mod-manager-shell.md` held the screen back for three reasons, each a boundary:

1. **A third-party host.** The list comes from `api.deadworks.net`, which is not on the network
   allowlist.
2. **Content from untrusted hosts.** A server can require addons and maps. They are downloaded
   from a host the server advertises, which can be anyone.
3. **Game writes.** Joining needs two `gameinfo.gi` lines, a UI pack under `citadel/deadworks_mods`
   that the game mounts on every launch, and the content under `citadel/deadworks_addons` and
   `citadel/maps`.

The two references differ. Grimoire reads a relay's `/api/servers` list, downloads a relay
manifest of `.vpk.bz2` files with 7-Zip, mounts them with a `Game citadel/deadworks_addons/vpks`
search path and opens `steam://connect`. Deadlock Mod Manager follows the newer public spec
(<https://docs.deadworks.net/guides/content-discovery/>): Steam's master list proxied by Deadworks,
content read from the game server's A2S_RULES reply and downloaded from its `dw_fastdl` host,
the UI bootstrap, `Game citadel/deadworks_mods` plus `addonroot citadel/deadworks_addons`, then
`steam://connect` with a console-log watchdog.

## Decision

Sigil ships a Servers screen that follows the public Deadworks spec, implemented from the spec in
`deadlock_core::servers`. Grimoire's page layout and ping readout are ported; nothing is taken from
Deadlock Mod Manager's code or strings.

The list:

- `GET https://api.deadworks.net/api/steam/servers?deadworks=1` (Steam's list, the source of truth
  for what is up), filtered to the `dw1` tag and without `dwu`, merged by address with
  `GET https://api.deadworks.net/api/servers` (the registry: country, version, roster). When Steam's
  list fails, the registry stands in and the page says so.
- Pings are one A2S_INFO round trip per listed server over UDP, from the user's machine.
- Sorted servers with room first, then busiest, then lowest ping, then name. Search, a map filter
  and a "Has space" switch filter the table.

Joining is a read-only check, then an explicit confirm, then the writes:

1. **Check (no writes).** A2S_RULES handshake with the game server (1.5 s budget, challenge echoed,
   `dw_ver` 1). Every entry is validated (`a-z0-9_` names up to 64, no Windows device names,
   16 hex digit hashes, decimal sizes) and so is `dw_fastdl` (absolute http(s), no credentials,
   query or fragment). Each file is planned as current, download, keep local (a map Sigil did not
   install, never replaced) or unavailable (no download host, which blocks the join). The
   `gameinfo.gi` lines are read and the bootstrap manifest is fetched.
2. **Confirm.** The dialog lists every write in plain words, the download host and size, and asks
   the user to quit the game first. Nothing is written until the user presses "Install and join".
3. **Writes**, in `prepare_join` on a worker:
   - `ensure_deadworks_gameinfo` first, so a locked file fails before a long download. It adds
     only the missing lines (`Game citadel/deadworks_mods`, `addonroot citadel/deadworks_addons`,
     and `Mod citadel` / `Write citadel` when `SearchPaths` has neither) above the first `Game`
     entry, inside marked comments. Lines another tool already wrote are left alone. The file is
     copied to `gameinfo.gi.dah-backup-<stamp>` and written atomically. This stays inside the
     `SearchPaths` scope of the safety model.
   - `install_bootstrap` when the installed UI pack does not match the manifest: https from
     `api.deadworks.net` only, full SHA-256 against the manifest, and a gate that refuses any entry
     that is not a compiled Panorama file (`panorama/**/*_c`). Other `.vpk` files in
     `deadworks_mods` are removed, as the spec requires.
   - `install_entry` per download: `.vpk.zst`, falling back to `.vpk.bz2` on 403, 404 or 410; format
     detected by magic bytes; every zstd frame and bzip2 stream decoded; zstd windows above log 27
     refused; 4 GiB caps on received and decoded bytes and 16 GiB per join; a minimum transfer rate;
     the VPK signature; size and hash against the advertisement; and a VPK gate that refuses
     unreadable directories, unsafe paths, executables and archives. The file is written as
     `.part` and renamed into place. A size or hash mismatch installs nothing and asks the user;
     "Install anyway" keeps every other check and is not recorded as the advertised build, so the
     next join checks again.
4. **Connect.** `steam://connect/<ip:port>` through the platform opener (`open`, `xdg-open`, or
   `rundll32 url.dll,FileProtocolHandler` on Windows, never `cmd`). Only a literal address is
   accepted; a password is limited to ASCII letters, digits and `-_.~`.

Untrusted download hosts:

- Every connection resolves through a guarded resolver that refuses loopback, private, link-local,
  shared and reserved addresses. ureq connects to exactly the addresses it returns, so a host cannot
  pass the check and rebind to a local address. Each redirect hop (at most five) resolves through
  it again. A local or LAN game server may use a local download host.
- The user's HTTP proxy is not used for these downloads, so the guard sees the real host.

Cosmetic-only and matchmaking:

- Server addons sit under `addonroot`, which the game mounts per server, only while the player is
  on a server that asks for them. They may carry game data because the host's mode needs it; they
  never load in matchmaking, so they are outside the cosmetic payload rule for mods Sigil builds.
- The UI bootstrap is the one file mounted on every launch. It is limited to compiled Panorama
  files and must match Deadworks' published hash, so it can change menus, not gameplay.
- Maps in `citadel/maps` load only when a server runs that map.

Network allowlist additions (documented in `docs/architecture.md`): `api.deadworks.net`; UDP A2S
queries to the listed game servers; and the download host a server advertises, contacted only
after the user confirms a join and shown in the confirm dialog. No user data is sent beyond the
requests themselves. No install pings or heartbeats, unlike the upstream Deadworks launcher.

## Consequences

- Players can browse and join Deadworks servers from Sigil on every platform the game runs on.
  Hosting stays out of scope; the page links to the Deadworks hosting guide.
- The allowlist now contains a class of hosts chosen by third parties. The guarded resolver, caps,
  VPK gates and the confirm dialog are what make that acceptable; weakening any of them reopens
  this decision.
- `gameinfo.gi` gains up to five marked lines that survive Sigil's own `Game citadel/addons`
  repair. A game update or Steam verification removes them; the next join adds them again.
- The ledger `citadel/deadworks_cache/sigil-content.json` records what Sigil installed and cached
  hashes. A map not in it is never replaced.
- Deadlock Mod Manager's console-log watchdog that re-sends `steam://connect` when the client lands
  in the hideout is not implemented. If `steam://connect` alone proves unreliable, add it later.
- Grimoire's relay manifest path (`/api/servers/:id/content`) is not used; servers that only
  publish content there and not in A2S_RULES join without it.
- Verified: unit tests on temp dirs for parsing, planning, decoding, gates, `gameinfo.gi` edits,
  local HTTP downloads with redirects and mismatches, and the resolver guard; a live run listed
  42 servers, read a real server's rules, installed the real UI pack and one real addon into a
  scratch root. Not verified: a join on a Windows machine with the game running a Deadworks server.

## Alternatives

- **No Servers screen** (the previous position): the user asked for it and both references ship
  one.
- **Grimoire's relay manifest and `Game` search path**: older than the public spec, needs 7-Zip,
  and mounts server addons on every launch, including matchmaking. Rejected for the spec's
  `addonroot`.
- **Deadlock Mod Manager's aggregation API** (`api.deadlockmods.app`): adds another third-party
  host between the player and Deadworks for no gain on the client. Rejected.
- **Launch with `+connect`**: Deadlock drops a boot-time `+connect` often; `steam://connect` is the
  path the Deadworks launcher uses.
