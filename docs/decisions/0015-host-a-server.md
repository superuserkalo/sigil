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
