# Roadmap

Sigil is a free, open-source mod manager for Deadlock on Windows 10 and 11. It browses and installs
GameBanana mods, keeps load order and overlaps in view, and launches the game modded or vanilla.

## Next

- Verification on a real Windows install of the rows still marked unverified in
  [docs/runbooks/windows.md](docs/runbooks/windows.md).
- Code signing for the installer and `Sigil.exe`, so Windows stops warning on first run.
- Sounds and work-in-progress mods in the local search index, so Browse search finds them too.

## Later: the mod agent

The mod agent is an assistant that helps a player build, pack and check a Deadlock mod. It runs
with the player's own model key (or a local model) and works in a staging folder inside Sigil's data
folder. It never writes to the game by itself: installing, editing `gameinfo.gi` and launching stay
buttons the player presses.

What exists in the code today: the agent host (`Sigil agent-host`, a separate process built on
TinyAgent), its tools for finding exact game paths, packing and validating a mod, the Agent page
with approvals for each step, and a Models page for keys and sign-in.

Why it is off in releases: it has not been verified end to end on a real install, and building
most mods for real needs Valve's Windows compile toolchain, which Sigil cannot ship. Release builds
leave it out entirely; nothing of it shows in the app and its process never starts.

How a developer can try it: build Sigil with the feature on and switch it on in
Settings > Experimental > Agent.

```bash
cd rust && cargo run -p dah-desktop --features agent
```

## Not planned

- Third-party analytics. Sigil's own opt-in reports are described in [docs/privacy.md](docs/privacy.md).
- Accounts. Sigil has no sign-in for players; favourites, profiles and settings stay on the PC.
