# Tier 5 acceptance

Status: active. This is the evidence record, not a promise.
Updated: 2026-11-16

`scripts/acceptance.sh` runs the release loop against a disposable fake install under
`.scratch/acceptance/<stamp>/`. It never reads or writes a real game directory.

```bash
scripts/acceptance.sh              # 42 checks, offline
scripts/acceptance.sh --network    # 44 checks, also the public stats API
```

## Result on this machine

```text
macOS aarch64, Rust 1.99.1
pass 44  fail 0
Tier 5 acceptance passed.
```

Both the debug CLI and the packaged CLI were run. The packaged run passed 42 of 42 offline checks.

## What it covers

1. Discovery and project
   - `doctor` finds the install and prints the capability report.
   - A project is created, given exact targets, validated and packed.
2. Catalog and slots
   - The game catalog indexes the packed VPK and resolves Billy's material.
   - Installed slots list, disable and enable with approval.
3. Manager writes
   - Profile capture, list and approved apply.
   - Autoexec write with backup.
   - Crosshair block apply and clear.
   - Character task from a generated PNG, plus the stage graph.
4. Release, deploy and publish
   - Baseline, release, deploy plan and approved deploy.
   - `gameinfo.gi` still lists the addon path.
   - Publish folder prepared; status refuses before an upload record exists.
5. Locker, demo, watch and install
   - Locker lists Billy, and a recolor brief is written with a real target.
   - Demo backends report the container extractor and its honest limits.
   - The watch queue and the durable watch state are written.
   - A compiled tree is planned and installed into slot 09, with a journal entry.
6. Launch and stats
   - Launch status reports the stash state.
   - With `--network`, the player snapshot and the real heatmap are fetched.

## What it does not prove

| Area | Why |
| --- | --- |
| Windows install, launch and deploy | No Windows machine or Deadlock install here |
| Real `resourcecompiler.exe` compile | CSDK 12 is Windows-only and is not bundled |
| Real `Source2Viewer-CLI` decompile | Not installed on this machine |
| Real Meshy or local model generation | No key and no local model were used |
| Positions and events from a real `.dem` | Needs a serializer-aware backend |
| 7z and rar archives | `7z` and `unrar` are absent here |
| Deadworks server browser | Third-party relay plus content sync; not shipped |
| Signed bundles | No signing identity |

A row that is not verified stays listed. It does not become a claim.

## Related

- `docs/runbooks/packaging.md` for the artifact and its unverified rows.
- `docs/research/demo-import.md` for the demo limits.
- `docs/research/locker-and-recolor.md` for the brief rules.
