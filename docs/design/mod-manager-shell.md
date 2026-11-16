# Mod-manager shell

Status: draft. Product direction for the native Rune app.
Updated: 2026-11-16

The app is a Deadlock mod manager with an agent inside it. It is not an agent demo and it is not a
reskin of the old Grimoire window. The mod-manager surfaces are primary; the agent is a persistent
tool that works on the selected mod or project.

## Sidebar

The shipped order. Every item below has a real core read or a gated app action.

```text
LIBRARY
  Installed        addon slots, enable/disable, conflicts, open folder
  Browse           GameBanana catalog, filter, install pipeline
  Conflicts        game paths more than one enabled slot replaces
  Updates          patch feed, stale baselines, broken projects

MODDING
  Profiles         capture and apply named sets of enabled slots
  Autoexec         read the game config, write behind approval
  Stats            rank, career, heroes, matches, heatmap
  Locker           hero renders and asset paths from the local game
  Launch           modded launch, vanilla stash, restore

WORKBENCH
  Agent            conversation, tools, approvals
  Projects         mod workspaces and manifests
  Game Index       exact VPK paths, search and extract
  Patches          official posts and changes
  Impact           what a patch breaks
  Release          pack, deploy plan, publish

FOUNDRY
  Toolchain        decompile and compile tools with real availability
  Character        reference or prompt to a verified GLB, compile plan

SETTINGS
  Settings         providers, paths, safety, nothing secret on screen
```

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
| Launch | `launch::launch_modded`, `launch_vanilla`, `restore_vanilla`, `stash_status` |
| Game Index | citadel, core and map VPK catalog |
| Patches | Steam news cache |
| Impact | project baseline + catalog + patch correlation |
| Release | release manifest, deploy plan, publish state |
| Toolchain | `toolchain::toolchain_status`, plan and approved run |
| Character | `character::character_status`, create, refresh, import |
| Agent | TinyAgent JSONL host |

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
