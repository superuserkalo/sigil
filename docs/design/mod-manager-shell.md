# Mod-manager shell

Status: historical. The 2026-11-16 sidebar draft; the app's order is `PLACES` in `rust/crates/desktop/src/app.rs` and the Agent page sits behind Settings > Experimental. Not maintained; the code wins where they differ. Original status: draft. Product direction for the native Rune app, Sigil.
Updated: 2026-11-16

The app is Sigil: a Deadlock mod manager with an agent inside it. The mod-manager surfaces are
primary; the agent is one place in the same window, shaped like a coding-agent desktop app. The
visual north star is Grimoire's layout with more restraint: no cards around groups, hairline rows,
one ember accent for the primary action, an on switch or focus.

## Window

- A custom titlebar, 40 points, drawn by the app (`rune::Titlebar`). macOS keeps its traffic
  lights over the sidebar band; Windows gets Rune-drawn caption buttons at the trailing edge.
  On Windows the custom frame leaves no top non-client row: on Windows 10 even a 1 px row makes
  DWM paint its own title bar above Sigil's (Rune patch `0012-win10-titlebar-top-inset.patch`).
- The titlebar band over the content holds the search for screens that search and a single line
  when something needs attention. No status bar, no stack or safety chips.

## Sidebar

Flat, no section headings, in this order:

```text
Installed      addon slots, on and off switch per row, overlaps button
Browse         GameBanana catalog, install pipeline
Locker         hero art grid, hero detail, recolor briefs
Crosshair      reticle editor with a live preview
Autoexec       the game config file
Stats          rank, heroes, matches
Overlap        game paths more than one enabled slot replaces
Profiles       saved sets of enabled slots

Agent          conversation, projects, approvals

(footer)
Launch modded  primary action
Launch vanilla
Settings       game, models and sign-in, advanced; app version
```

Projects, Game Index, Patches, Impact, Release, Toolchain and Character live inside the Agent place
(its left column links to them) and keep their keyboard shortcuts. Updates surfaces as a banner, not
a nav item. Selection is a neutral fill; there is no accent bar and no layout motion.

A sidebar item appears only when it has a real data source and a real action. There is no
"coming soon" state. When a backend is unavailable, the screen says which backend, why, and what
would make it available, and keeps every action that does not need it.

## Real data sources

| Screen | Source |
| --- | --- |
| Installed | `addons::read_conflict_report`: slot files, chunk files, entry counts, conflicts |
| Browse | GameBanana API v11 `Game/20948/Subfeed`, item preview images and profile URLs |
| Conflicts | the same conflict report, grouped by game path |
| Updates | `dah watch`, patch cache, project baselines |
| Profiles | `profiles::list_profiles`, `capture_profile`, approved `apply_profile` |
| Autoexec | `autoexec::read_autoexec`, approved `write_autoexec` with a backup |
| Stats | `stats::StatsClient`: player, rank, career, heroes, matches, heatmap |
| Locker | `locker::discover_heroes_with_assets`, local art root, never copied |
| Launch | `launch::launch_modded`, `launch_vanilla`, `restore_vanilla`, `stash_status`, `read_game_process`, approved `stop_game`, the launch marker (`record_launch`, `record_exit`, `acknowledge_crash`), `impact` crash suspects and ignores, `download_queue` |
| Game Index | citadel, core and map VPK catalog |
| Patches | Steam news cache |
| Impact | project baseline + catalog + patch correlation |
| Release | release manifest, deploy plan, publish state |
| Toolchain | `toolchain::toolchain_status`, plan and approved run |
| Character | `character::character_status`, create, refresh, import |
| Agent | TinyAgent JSONL host |

## Servers

Deadlock is matchmade and Valve ships no server browser. Deadworks community servers are the one
joinable directory, so Servers is a sidebar place after Profiles. What held it back (a third-party
host, content from hosts a server picks, and `gameinfo.gi` and content writes) is settled in
`docs/decisions/0009-deadworks-servers.md`:

- The list comes from `api.deadworks.net` (Steam's list for Deadlock filtered to Deadworks, plus the
  registry), now on the network allowlist. Pings are A2S over UDP to each listed server.
- The table follows Grimoire's page: status dot, name with country and Deadworks version, map,
  players, ping and a Join button per row; search in the title bar, a map filter and a "Has space"
  switch. A row opens the server's detail with its address and who is playing.
- Join is a read-only check, then a confirm that lists every write (the `gameinfo.gi` lines, the
  Deadworks UI pack, each download with its size and host), then the writes on a worker with a
  progress bar, then `steam://connect`. Nothing touches the game folder before the confirm.
- Server content is checked before it lands: hash and size against what the server advertises,
  VPK checks, caps, and a resolver that refuses local and private addresses.

## Layout rules

- One page head per screen: eyebrow, title, one sentence of provenance, trailing actions.
- A strip under the head carries the path or scope the screen reads, so a screenshot always says
  where the data came from.
- Cards state a real count, a real path or a real error. A number without a sample size is a bug.
- Results of an app action land in a result strip on the same screen, not only in the status bar.

## Design rules

- Mod manager first. The default view is the library, not a chat transcript.
- The agent is always reachable, but it does not replace navigation. Use a persistent assistant pane
  or a dedicated Agent screen, not a chat-only shell.
- Dense tables and real thumbnails where they help, not a dashboard of equal cards.
- No demo fixtures in normal launch. Demo scenes stay in headless tooling only.
- Every empty state names the missing input: no game install, no project, no release, no API data.
- One primary action per screen. Approval dialogs are explicit and readable.
- Native Rune: springs, hairlines, OLED canvas, generous whitespace, monospace for paths and hashes.

## Implementation order

1. Installed, Launch, Updates. These use data and actions the core already owns.
2. Browse read-only from GameBanana. Install follows once download, ZIP extraction and slot rules
   are in core.
3. Profiles and Autoexec. Both are local and approval-gated.
4. Locker previews, recolor and sounds. These need Source 2 asset export and preview work.
5. Merge VPK. Needs a merge backend and conflict report.

## What not to do

- Do not add sidebar items for features with no data or action.
- Do not ship fixture data as if it were the user's library.
- Do not hide the agent, but do not make it the whole app.
