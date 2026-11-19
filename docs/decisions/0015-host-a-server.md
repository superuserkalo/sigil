# 0015 Host a Deadworks server from the player's PC

Status: accepted (owner, 2026-11-19)
Date: 2026-11-19

## Context

Players want custom servers and, next, game modes the agent writes. Deadworks (MIT) is the
plugin framework that runs inside Deadlock's dedicated server; Sigil already lists and joins
Deadworks servers (ADR 0009). Research: `.scratch/server-hosting/research.md`. Hosting for other
people for money waits on Valve's terms and the Steam login question; hosting on the player's own
PC does not.

## Decision

- Sigil gets a "Host a server" flow on Windows. The server lives in its own folder the player
  picks (default under the app data directory or a drive with room), never inside the player's
  Deadlock install.
- New downloads, each pinned and checked: the Deadworks release zip from GitHub (a recorded
  version and sha256, updated deliberately), and Valve's SteamCMD installer zip from
  `steamcdn-a.akamaihd.net`. Both are added to the network allowlist. SteamCMD then talks to
  Steam itself; that traffic is Valve's tool, started by the player.
- The Steam login happens in SteamCMD's own console window, which Sigil opens on purpose. Sigil
  never asks for, sees or stores the Steam password or Steam Guard code.
- New processes, all app actions the player triggers: SteamCMD (install and update the server
  files), `deadworks.exe -dedicated` (start and stop, its console output shown in the app), and a
  Windows Firewall rule for the server port through `netsh`, which raises the Windows admin
  prompt. No UPnP and no public-IP lookup service in v1: the app explains router forwarding.
- The .NET 10 runtime is a documented prerequisite Sigil checks for and links to; it is not
  downloaded by Sigil.
- Agent tools never start or stop a server, change the firewall or install server files. Game
  modes the agent writes (phase 2) reach the server only through an approved app action.

## Consequences

- One more page that only works on Windows; macOS and Linux show why.
- About 35 GB of disk for the server files; the setup checks free space first.
- Deadworks updates follow Deadlock patches; Sigil offers to update both the server files and the
  pinned Deadworks release.

## Amendment (owner, 2026-11-19): serve custom content to joiners

Joiners only get a custom map offered when the server advertises a download address
(Deadworks `fastdl_url`). Asking the host to upload files elsewhere is not zero setup, so:

- While the hosted server runs, Sigil runs a read-only HTTP file server on the next port (game port
  plus one, 27068 by default, configurable) that serves only `<server>/sigil-fastdl/`: GET and HEAD,
  no directory listing, no paths outside that folder, no symlinks, a cap on connections and request
  size. It stops with the server. Files are content-addressed (`<name>_<hash16>.vpk.zst`) and the
  joiner checks the hash, so a tampered download is refused.
- The advertised address uses the placeholder host `0.0.0.0` (`http://0.0.0.0:27068`). A joining
  Sigil replaces it with the address it is connecting to, so no public-IP lookup is needed and it
  works on a LAN and over the internet. Other managers that do not know the rule see an address
  they cannot reach and fall back to their own behaviour.
- The firewall rule covers this port (TCP) together with the game port, behind the same Windows
  admin prompt; the router steps name both ports.

## Amendment (owner, 2026-11-19): hosting without port forwarding

Friends should be able to join without the host touching the router, as in Lunar Client:

- First choice, no setup: the in-game host (a listen server started with Launch map) registers
  with Steam, and friends join with `connect <ServerSteamID>` through Steam's own networking and
  Valve's relays when a direct path fails. Sigil turns on `-condebug` for that launch so it can read
  the code from `console.log`. Plugins do not run there.
- For the dedicated Deadworks server: if it can register through Steam networking (research in
  `.scratch/server-hosting/steam-networking.md`), Sigil offers the same code-based invite.
- Otherwise an opt-in switch asks the home router to open the game and download ports through
  UPnP (IGD): SSDP discovery and SOAP requests on the local network only, mappings removed when the
  server stops, nothing sent outside the LAN. Off by default; the router steps remain the fallback.
  No third-party relay or tunnel service.
- Research result (`.scratch/server-hosting/steam-networking.md`): the dedicated server can join
  through Steam networking. Sigil's default "Relay" mode writes Deadworks' two edits into the
  SERVER's own `game/citadel/gameinfo.gi` (`CreateListenSocketP2P 2` in NetworkSystem,
  `"net_p2p_listen_dedicated" "1"` in ConVars; re-applied after every server file update), starts
  with `-nomaster` (unlisted), waits for the relay to be ready and reads `ServerSteamID` from the
  console. The code changes on every restart.
- Relay joins cannot reach the download port. Sigil's invite carries the join code plus the
  GameBanana ids of the server's custom content, so a joining Sigil installs those from GameBanana
  and then connects. Content that is not on GameBanana needs the download port (UPnP or
  forwarding); the Host page says which items those are.
