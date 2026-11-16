# Mod-manager shell

Status: draft. Product direction for the native Rune app.
Updated: 2026-11-16

The app is a Deadlock mod manager with an agent inside it. It is not an agent demo and it is not a
reskin of the old Grimoire window. The mod-manager surfaces are primary; the agent is a persistent
tool that works on the selected mod or project.

## Sidebar

```text
LIBRARY
  Installed        addon slots, enable/disable, conflicts, open folder
  Browse           GameBanana catalog, categories, search, install
  Updates          patch feed, stale baselines, broken projects

PLAY
  Launch Modded    start through Steam, approval
  Launch Vanilla   stash addons, launch, restore

LOCKER
  Heroes           hero list from the catalog, 2D previews
  Recolor          ability VFX recolor
  Sounds           per-ability sound picker

TOOLS
  Profiles         save and apply enabled-mod sets
  Autoexec         edit the game autoexec with approval
  Merge VPK        merge or split addon VPKs

WORKBENCH
  Agent            conversation, tools, approvals
  Projects         mod workspaces and manifests
  Game Index       exact VPK paths, search and extract
  Patches          official posts and changes
  Impact           what a patch breaks
  Release          pack, deploy plan, publish

SETTINGS
```

A sidebar item appears only when it has a real data source and a real action. A screen with a
"coming soon" badge is not shipped.

## Real data sources

| Screen | Source |
| --- | --- |
| Installed | `read_game_status`: addon slots, chunk files, ignored files, `gameinfo.gi` |
| Browse | GameBanana API v11: `Game/20948/Subfeed`, item preview images and profile URLs |
| Updates | `dah watch`, patch cache, project baselines |
| Launch | Steam `steam://rungameid/1422450`, approval gate |
| Game Index | citadel, core and map VPK catalog |
| Patches | Steam news cache |
| Impact | project baseline + catalog + patch correlation |
| Release | release manifest, deploy plan, publish state |
| Agent | TinyAgent JSONL host |

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
