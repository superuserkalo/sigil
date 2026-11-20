# 0016 Relay region picker

Status: accepted (owner, 2026-11-20: build it, off by default)
Date: 2026-11-20

## Context

Deadlock routes every match through Valve's relay network (Steam Datagram Relay). The client
pings the relays and matchmaking places the player where those pings say. Deadlock Mod Manager's
Server Picker lets a player block relay locations with firewall rules so matchmaking avoids them:
far regions, language barriers, regions a group of friends agrees to skip. The parity audit kept
it open for a decision because it changes the player's firewall, which no other Sigil feature
does outside hosting (ADR 0015). The owner decided to build it, off by default.

## Decision

- Servers gets a Regions tab (`deadlock_core::relays`, `screens/servers_regions`). It lists
  Valve's relay locations by region with ping and health, lets the player tick locations, keeps
  presets in `options.json`, and applies the choice on Apply. Nothing changes until Apply; Reset
  removes every Sigil relay rule.
- The relay list comes from `api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=1422450`,
  a host already on the allowlist, cached for a day, 15 s timeout, 2 MB cap. Pings use the
  system `ping` on a press only, bounded (eight at a time, two addresses and three echoes per
  location, one second each) and cancellable.
- Rules are Windows Firewall outbound blocks for UDP from `deadlock.exe` to the relay addresses
  and port ranges of one location, named `Sigil-Relay-<location>-<first>-<last>`. Scoping to the
  program leaves other Steam games, which use the same relays, untouched. Location ids,
  addresses and the program path are checked before they reach a command.
- Apply and Reset write one `netsh` script (delete every Sigil relay rule, add the planned
  ones) and run `netsh -f <script>` behind Windows' admin prompt, once. The rules are then read
  back without elevation; the read back, not the exit code, decides what the page shows.
  Partial coverage (Valve added relays, a rule removed by hand), rules for locations Valve no
  longer lists, disabled rules, a firewall turned off for the active network, and a change that
  never reported back are each shown with Apply again or Reset.
- These are app actions behind a press and the admin prompt. The core functions take an
  `approve` flag, and no agent tool calls them.
- While any region is blocked the status bar shows it on every page (also when the bar is
  hidden), and every launch shows a notice, so a forgotten block is not taken for broken
  matchmaking. The saved record of applied locations survives Settings' reset, because the rules
  outlive it.
- Blocking is Windows only. macOS and Linux show the list and pings and say why blocking needs
  Windows. Deadlock Mod Manager's Linux nftables mode is not built.

## Consequences

- A forgotten block makes queues long or matchmaking fail; the status bar mark and the launch
  notice are the guard against that, and Reset needs neither the relay list nor the saved
  choices.
- `netsh` labels are translated, so the read back parses by the shape of each value. A Windows
  language whose "No" word is not in the list still reads a disabled rule correctly through the
  Edge traversal comparison; a value Sigil does not recognise reads as enabled.
- The script file sits in Sigil's data folder for the moment it runs. Anything that can rewrite
  it there already runs as the player and could raise its own admin prompt.
- Not yet run on a Windows PC: admin prompt, `netsh -f` with a non-ASCII program path (sent as
  UTF-16), and the read back on a localized Windows need a real check.

## Alternatives

- The Windows Firewall COM API (what Deadlock Mod Manager uses) gives exact field reads, but
  needs an elevated helper process with its own IPC. One elevated `netsh` batch reuses the
  hosting path (ADR 0015) and keeps a single admin prompt.
- One rule per relay address: about 150 rules today, slower to add and harder to read in
  Windows' own firewall window. Grouping by location and port range gives about 35.
- System-wide rules without a program: would also block other Steam games on the same relays.
