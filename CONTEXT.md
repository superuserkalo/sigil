# Sigil (Deadlock Agent Harness)

A Deadlock mod manager, with an experimental workbench where an agent turns a mod idea into a
validated, installable mod. The terms below are the domain and workbench vocabulary.

## Language

**Mod**:
A VPK placed in the game's addon folder that replaces game files by exact path.
_Avoid_: addon, package, asset pack

**Addon**:
The game-side container concept and folder (`citadel/addons`). An addon slot is one `pakNN_dir.vpk`.
_Avoid_: mod, when the file or slot is what is meant

**Addon slot**:
One numbered file `pak01_dir.vpk` through `pak99_dir.vpk` in a folder. Sigil adds
`citadel/sigil-addons2` to `sigil-addons10` for slots 100 to 990. Lower numbers win when mods
collide.
_Avoid_: load order entry, index

**Payload**:
The exact set of paths and bytes that go inside a mod VPK. Normally only compiled `_c` files; the
validator flags anything else, and the app shows that check rather than enforcing a rule (ADR 0011).
_Avoid_: contents, package

**Staging folder**:
A workspace folder laid out like the game, holding only the compiled files a mod changes.
_Avoid_: build folder, output

**Compiled file**:
A game-readable binary whose name ends in `_c` (`vmdl_c`, `vtex_c`, `vsnd_c`). The game ignores
source files.
_Avoid_: asset, cooked file

**Build backend**:
The thing that turns source assets into compiled files. A Windows CSDK 12 toolchain today; possibly
a remote Windows host later.
_Avoid_: compiler (too narrow), pipeline

**Mod workspace**:
The per-project directory the agent can read and write. Sources, staging, build scripts and notes.
_Avoid_: sandbox, repo

**Game install**:
The Steam copy of Deadlock the app inspects and, with approval, installs into.
_Avoid_: game folder, install root

**Game catalog**:
The cached paths from the citadel, core and map VPKs, with the metadata needed to extract them.
_Avoid_: file list, database

**Extracted file**:
A raw game file copied out of the VPK into the mod workspace. It is still compiled until a
decompile backend runs.
_Avoid_: source asset, decompiled file

**Reference image**:
A user-supplied or licensed image used as visual direction for a mod. It stays local and is never
shipped in the payload.
_Avoid_: asset, texture source

**Likeness**:
A real person's face or identity in a mod. It needs a recorded rights basis and a separate approval
before publish.
_Avoid_: face swap, celeb mod

**Character mod**:
A mod that changes a hero's appearance while staying on the game skeleton and animations.
_Avoid_: model swap, hero replacement

**Agent host**:
The TinyAgent process that owns durable sessions, tools, providers and approvals. It runs beside the
app and never draws the UI.
_Avoid_: server, backend, harness

**Session**:
One durable TinyAgent journal. The product uses one per mod project plus a root workbench.
_Avoid_: chat, conversation

**Run**:
One prompt through the model and its tool calls. A run is journaled and can resume after a crash.
_Avoid_: job, task

**Tool**:
A function the model can call. Declares whether it is idempotent when a run resumes after a crash.
_Avoid_: action, skill, command

**Approval gate**:
A durable pause that requires a human decision before a tool with external effects runs.
_Avoid_: confirmation, prompt

**Release**:
A versioned VPK plus its manifest, checksums and release notes, stored in the project workspace.
_Avoid_: build, export

**Deploy plan**:
The slot, conflicts, replacement decision and `gameinfo.gi` repair computed before any game write.
_Avoid_: install

**Migration**:
A proposed mapping from a broken target path to its new path after a game update.
_Avoid_: fix, patch

**Baseline**:
The recorded CRCs, sizes and owning VPK for a project target set, captured after a check.
_Avoid_: snapshot, lockfile

**Change impact**:
The comparison of a project baseline with the current game catalog: unchanged, changed, missing,
moved or unbaselined.
_Avoid_: diff, breakage report

**Patch note**:
An official Deadlock update post from the Steam news feed. A hint, not proof of a file change.
_Avoid_: changelog, update log

**Deploy**:
Copying a validated VPK into a chosen addon slot, with approval.
_Avoid_: install, publish

**Publish**:
Preparing a release and handing it to GameBanana's submission page in the browser, with approval.
Sigil records and verifies the submission URL; it never uploads or touches the account (ADR 0008).
_Avoid_: deploy, share
