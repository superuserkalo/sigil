# Architecture

Status: draft. Boundaries below are proposals until recorded in `docs/decisions/`.
Updated: 2026-11-16

This document is the map. The executable rules live in code, especially
`packages/deadlock-tools/src/validate.ts` and the approval hooks in the agent host.

## Product shape

The user runs a desktop app, opens a mod project, and talks to an agent. The agent reads the game,
extracts what it needs, edits files, builds, packs, validates, and asks for approval before touching
the game install or publishing anything. A CLI exposes the same runtime without the UI.

Three promises shape every boundary:

1. The model never gets a credential. BYOK keys stay in the main process and are injected into
   provider requests there or into the isolated agent host, never into a transcript.
2. An agent tool never writes to the game install. It can write the mod workspace and produce a
   plan or a validated artifact. A main-process action, behind an approval gate, performs the
   deploy.
3. A payload that violates the community rules cannot be deployed, regardless of what the prompt
   says. Validation is code.

## Processes

```text
+---------------------------------------------------------------+
| Renderer (React, Grimoire fork)                                |
| chat, plan, approvals, build panel, mod list, 2D/3D previews   |
+------------------------------+--------------------------------+
                               | contextBridge (preload)
+------------------------------v--------------------------------+
| Electron main                                                  |
| secrets (safeStorage), settings, game discovery, VPK writes,   |
| file dialogs, approval decisions, toolchain configuration      |
+------------------------------+--------------------------------+
                               | utilityProcess IPC (view ops)
+------------------------------v--------------------------------+
| Agent host (Node utility process)                              |
| Pi Durable Harness, registry, tools, SQLite storage            |
+------------------------------+--------------------------------+
                               | ExecutionEnv
+------------------------------v--------------------------------+
| Mod workspace                    External binaries             |
| sources, staging, build output   vpkeditcli, Source2Viewer CLI, |
|                                  vpkmerge, resourcecompiler     |
+---------------------------------------------------------------+
```

Why a separate agent host:

- A crashed or wedged agent does not take the UI with it. Pi Durable reopens the same SQLite storage
  and resumes unfinished tasks.
- The renderer can attach to the durable view over IPC, so reconnects are cheap and multiple windows
  can watch one conversation.
- The main process keeps the only copy of decrypted credentials and the only code path that writes
  to the game install.

The agent host is not a security sandbox by itself. It is a crash and responsibility boundary. The
security boundary for game writes is the main process plus the approval gate.

## Agent runtime

Pi Durable is an experimental harness package, pinned to `1.5.2` (MIT). It provides:

- A durable transcript and task graph over SQLite.
- Extensions as named bundles of tools, prompt sections, hooks and tasks.
- Tool calls as checkpointed tasks. `replay: "safe"` decides whether a call may rerun after a crash.
- Documents for application state, committed with the transcript.
- Signals and parked tasks, which is how an approval can wait for a person without holding a
  process.
- Per-conversation agent configuration: model, thinking level, tools, extensions, working
  directory.
- A watchable view for UIs and a task graph for build progress.

Mapping to the product:

| Pi Durable | Product |
| --- | --- |
| Harness | The agent host session, one SQLite file in userData |
| Root conversation | The general workbench |
| Conversation | One mod project, plus optional reviewer or subagent conversations |
| Document | Project manifest, plan, approval decisions, build records |
| Task | One tool call, one build, one publish attempt |
| Signal | A human approval or a build-complete notification |
| Extension `coding` | read, write, edit, bash |
| Extension `deadlock` | game status, payload validation, domain prompt section |
| Extension `plan` | read-only planning mode (planned) |
| Extension `approval` | `beforeTool` gate for deploy, launch, publish (planned) |
| Extension `knowledge` | curated modding docs and path lookup (planned) |
| `env` | `NodeExecutionEnv` rooted at the mod workspace |

The choice of Pi Durable over fx and libfx is recorded in
`docs/decisions/0004-embedded-runtime-choice.md`. fx stays an optional external lane (ACP or MCP),
not a dependency.

One process owns a storage. The desktop app and the CLI must not open the same SQLite file at the
same time. The CLI uses its own data directory by default.

## Tool catalog

Implemented today:

| Tool | Effects | Replay | Approval |
| --- | --- | --- | --- |
| `deadlock_game_status` | Reads Steam libraries, `gameinfo.gi`, addon folder | safe | none |
| `deadlock_validate_payload` | Pure path rules over a proposed entry list | safe | none |

Planned, in rough dependency order:

| Tool | Effects | Replay | Approval |
| --- | --- | --- | --- |
| `game_index` | Reads and caches the game VPK entry list | safe | none |
| `game_extract` | Writes extracted files to the workspace | unsafe | none (workspace only) |
| `project_init` | Writes a project manifest and staging layout | unsafe | none |
| `project_build` | Runs the build backend, writes staging | unsafe | none |
| `vpk_inspect` | Reads a VPK manifest | safe | none |
| `vpk_pack` | Writes a VPK in the workspace | unsafe | none |
| `vpk_merge` | Writes a merged VPK in the workspace | unsafe | none |
| `addon_plan_deploy` | Computes conflicts and target slot | safe | none |
| `addon_deploy` | Copies a VPK into the game install | unsafe | required |
| `game_launch` | Starts the game | unsafe | required |
| `gameinfo_repair` | Restores the `addons` SearchPaths line | unsafe | required |
| `publish_prepare` | Builds a release folder and metadata | unsafe | none |
| `publish_upload` | Sends files to GameBanana | unsafe | required |
| `knowledge_search` | Searches curated modding notes | safe | none |

Rules for adding a tool:

- A tool that only reads or computes is `replay: "safe"`. A tool that writes, spawns a process that
  writes, or calls an external service is `replay: "unsafe"`.
- Model-facing output stays small and text. Large results go to workspace files and are referenced
  by path.
- Anything that leaves the mod workspace goes through a main-process action. The tool returns a
  plan, not a side effect.
- Every new contract gets a test in `packages/*/test`.

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
   file, records the exact edit, and can restore Valve's version. Anything else is refused.
5. **Secrets.** Decrypted keys exist only in main. The agent host receives provider auth through a
   narrow handshake, not the transcript. Tool output and logs are scrubbed for known key shapes.
6. **Network allowlist.** Provider endpoints, GameBanana, and the GitHub update check. No other
   outbound calls from tools without a documented allowlist entry.
7. **Exec allowlist.** The desktop wraps `bash` with a workspace boundary and a binary allowlist.
   The CLI keeps the plain tool for local development until the wrapper lands.
8. **Untrusted input.** Mod descriptions, wiki pages and GameBanana metadata are data, not
   instructions. Tools that fetch them must not forward them into a privileged path. The approval
   gate is the backstop.

Cosmetic-only policy is both an ethical rule and a compatibility rule: gameplay data goes stale
after updates and can break other players' mods.

## Data layout

Desktop userData:

```text
agent.sqlite          Pi Durable transcripts, tasks, documents
credentials.json      safeStorage-wrapped provider credentials
settings.json         model choice, toolchain paths, workspace root, UI settings
mods/<slug>/          mod workspace, git-friendly
  project.json        manifest: name, target paths, slot preference
  sources/            decompiled sources the agent edits
  staging/            compiled _c files at game paths
  builds/             packed VPKs and validation reports
  notes/              agent notes and screenshots
logs/                 redacted app and tool logs
```

CLI data directory follows the OS convention (`~/Library/Application Support` on macOS,
`%APPDATA%` on Windows, `~/.local/share` on Linux), overridable with `DAH_DATA_DIR`.

A mod workspace is deliberately a normal directory. Users can open it in an editor, version it, or
hand it to another agent.

## Desktop integration

The desktop app is Grimoire with an agent surface. Keep the fork narrow:

- Keep: Steam discovery, addon slots, `gameinfo.gi` handling, VPK merge, mod catalog, Locker
  previews, design tokens, i18n pipeline, no-telemetry policy.
- Add: agent chat and plan views, tool call timeline, approval panel, project picker, build panel,
  payload validation report, settings for providers and toolchains.
- Cut or hide initially: social profiles, Steam sign-in, deadlock-api stats, crosshair designer.
  They widen the security surface and are not needed to prove the mod loop.
- Do not grow the existing god files (`Installed.tsx`, `Browse.tsx`). New UI goes in
  `src/components/<feature>/`.
- Keep the no-em-dash rule and the MIT attribution in the fork.

Grimoire depends on the sibling `grimoire-social` workspace for wire types. `pnpm vendor` clones
both into `vendor/` so the relative workspace path resolves. Once the fork is official, the fork
becomes its own repository and the desktop app can live in `apps/desktop` or stay as a pinned
submodule; that decision is recorded in `docs/decisions/0001-grimoire-fork-and-monorepo.md`.

## CLI

`dah` is the headless surface over the same packages:

```text
dah doctor                 game install, gameinfo, slots, providers
dah models                 providers, auth status, available models
dah auth set <provider>    store a key, chmod 600
dah auth list              provider ids and credential types, no secrets
dah chat "<prompt>"        one agent run
```

Later: `dah mod new`, `dah mod build`, `dah mod validate`, `dah mod deploy`. The CLI is useful for
CI, for scripted batch work, and for testing the runtime without Electron.

## Failure modes

- **Process death during a run.** Reopen the same SQLite storage, call `resume()`, unfinished tool
  calls rerun only if replay-safe. A model request that was cut off is retried.
- **Agent host crash loop.** Pi Durable's restart limit ends the task instead of looping. The UI
  reports it and keeps the transcript.
- **Game update.** Hero paths move and `gameinfo.gi` resets. Detection: compare the project's target
  paths with the current VPK index. Repair: add the SearchPaths line back, then migrate paths.
- **Missing toolchain.** `doctor` reports the missing backend and the compile step is disabled with a
  reason. Never silently substitute a different compiler.
- **Slot collision.** Show the winning mod, the conflicting files, and ask before replacing a slot.
- **Corrupt or huge VPK.** Validate before deploy. Refuse > 2 GB without a chunk plan.

## Roadmap

Phase 0, scaffold (current):

- Monorepo, docs, agent manuals, domain glossary.
- Working harness bootstrap and BYOK credential store.
- CLI `doctor`, `auth`, `models`, `chat`.
- Game discovery, `gameinfo.gi` status, addon slot status.
- Payload validator with tests.

Phase 1, read-only game introspection:

- Index `pak01_dir.vpk` and cache entry paths.
- Search paths by hero codename and file type.
- Extract and decompile into the workspace.
- Preview extracted textures and models in the existing Grimoire viewers.

Phase 2, build loop without compile:

- Project manifest and staging layout.
- Pack with `vpkeditcli`, inspect, merge.
- Validate, plan deploy, deploy to a test slot with approval, launch.
- Acceptance: recolor a texture whose source asset can be produced without a compiler, pack it,
  deploy it, and see it in game.

Phase 3, compile backends:

- Local Windows backend for textures, sounds, materials and UI.
- Remote backend over a Pi execution environment.
- Acceptance: edit a source asset, compile, pack, deploy, verify in game on Windows.

Phase 4, publish and survive updates:

- Release folder and metadata for GameBanana, upload with approval.
- Update detection using `GameTracking-Deadlock`, path migration, `gameinfo.gi` repair.
- Acceptance: update a published mod after a game patch with a recorded diff.

Phase 5, advanced:

- Model swaps and AnimGraph 2 via `DeadlockTools`.
- Multi-chunk packs, batch operations, VPK previews.
- Reviewer subagent and multi-client harness.

## Open questions

- Is CSDK 12 viable under Wine or Proton on macOS and Linux? Until verified, do not promise it.
- Can any part of the toolchain be redistributed? CSDK 12 has no license. Bundle only tools with a
  clear license and a pinned hash.
- Which GameBanana upload flow is acceptable: browser handoff, API key, or a prepared zip only?
- Do we keep Grimoire's social features, and does the fork inherit its companion server?
- How much of the prompt and knowledge pack ships in the app versus being read from `docs/`?
- What is the minimum viable visual verification after deploy?

## Non-goals

- Gameplay cheats, aim assistance, wallhacks, or anything that gives an advantage in matchmaking.
- Redistributing Valve assets.
- A hosted model proxy or a paid inference tier.
- Telemetry, analytics, or a heartbeat.
- Automatic public releases without a human decision.
