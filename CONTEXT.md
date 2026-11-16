# Deadlock Agent Harness

A workbench where an agent turns a mod idea into a validated, installable Deadlock mod.

## Language

**Mod**:
A VPK placed in the game's addon folder that replaces game files by exact path. Cosmetic only.
_Avoid_: addon, package, asset pack

**Addon**:
The game-side container concept and folder (`citadel/addons`). An addon slot is one `pakNN_dir.vpk`.
_Avoid_: mod, when the file or slot is what is meant

**Addon slot**:
One numbered file `pak01_dir.vpk` through `pak99_dir.vpk`. Lower numbers win when mods collide.
_Avoid_: load order entry, index

**Payload**:
The exact set of paths and bytes that go inside a mod VPK. Only compiled `_c` files belong here.
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

**Game index**:
The cached list of paths inside `game/citadel/pak01_dir.vpk`, with metadata needed to extract them.
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

**Harness**:
The durable agent runtime: storage, conversations, tasks, tools and the model.
_Avoid_: server, backend

**Conversation**:
One durable transcript. The product uses one per mod project plus a root workbench.
_Avoid_: chat, session

**Task**:
A durable unit of work with checkpoints, such as one tool call or one build. Survives process death.
_Avoid_: job, step

**Tool**:
A function the model can call. Declares whether it is safe to replay after a crash.
_Avoid_: action, skill, command

**Approval gate**:
A durable pause that requires a human decision before a tool with external effects runs.
_Avoid_: confirmation, prompt

**Deploy**:
Copying a validated VPK into a chosen addon slot, with approval.
_Avoid_: install, publish

**Publish**:
Preparing and uploading a release to a mod host such as GameBanana, with approval.
_Avoid_: deploy, share
