# Browse install pipeline

Status: active, with one unverified area: 7z and rar extraction is wired but has not been run
against a real archive, because neither tool is installed on the machine this was built on.
Updated: 2026-11-16

How `dah install` takes a GameBanana mod or an archive and ends with a file in
`game/citadel/addons`. The executable rules live in `rust/crates/deadlock-core/src/install.rs`;
this document records the boundaries and what has actually been proven.

## Shape

```text
URL or mod id
  -> download        <workspace>/downloads/<source-key>/<file>
  -> extract         <workspace>/downloads/<source-key>/extracted/
  -> classify        VPK candidate or compiled tree, game path prefix detected
  -> validate        payload rules from `payload.rs` (hard gate)
  -> slot            lowest free 01..99, or --slot, never silent overwrite
  -> plan            .dah/install/plans/<id>.json, nothing written to the game
  -> apply           approval gate, backups, installs.jsonl
```

`plan_install` writes only inside the workspace. `apply_install` is the only function here that
writes into the game install, and it refuses without an explicit approval flag. That matches the
safety model in `docs/architecture.md`: a tool produces a plan; the app performs the deploy.

## Network

The download allowlist is `https://gamebanana.com`, `https://www.gamebanana.com`,
`https://files.gamebanana.com`, plus every HTTPS subdomain of `gamebanana.com`.

The apex-domain rule is not cosmetic. On 2026-11-16 a live `https://gamebanana.com/dl/<id>`
redirected to `https://filecache52.gamebanana.com/mods/<file>`, not to `files.gamebanana.com`. A
fixed host list would have refused real downloads. The rule still excludes `http://`,
non-443 ports, `notgamebanana.com` and `gamebanana.com.evil.test`; all four are covered by tests.

- Redirects are followed by hand, five hops at most, and every hop is checked again. A redirect
  target may be absolute or root relative.
- HTTPS only, except a mirror origin that names `http://` itself. `DAH_INSTALL_MIRROR` (or
  `--mirror`) is the only way to add an origin; the tests use it for a 127.0.0.1 server.
- Whole-request budget: 300 s. Download cap: 2 GiB. The body streams to `<file>.part` in 64 KiB
  chunks, and any failure removes the partial file. There is no resume.
- The plan records the sha256 and the byte count of the archive. GameBanana's md5 is kept in the
  plan metadata but is not verified: the pipeline hashes sha256 only, and the plan says so.
- A GameBanana file id is immutable, so a cached archive whose size matches the `_nFilesize` from
  the file record is reused with a warning instead of downloaded again. `dah install cache prune`
  removes the cache.
- GameBanana metadata comes from `GET https://gamebanana.com/apiv11/Mod/<id>/ProfilePage`. When a
  mod lists several files, the newest `_tsDateAdded` wins and the plan warns that a choice was
  made; there is no "install every file". A mod page URL such as
  `https://gamebanana.com/mods/715100` is read as that mod id.

## Archive formats

| Format | Backend | Status |
| --- | --- | --- |
| ZIP | In process through the `zip` crate, stored and deflate | Verified with fixtures and a real 3.7 MiB GameBanana archive |
| 7z | External `7z` / `7zz` / `7za` from `DAH_7Z` or `PATH` | Wired. Not run against a real archive here: no 7z on this machine |
| rar | External `unrar` / `rar` from `DAH_UNRAR` or `PATH` | Wired. Not run against a real archive here: no unrar on this machine |
| anything else | None | Refused with the detected or unknown format named |

Facts behind the table:

- `zip` and `sha2` were already in `rust/Cargo.lock` through `dah-rs`, so the core reuses them
  rather than adding a new HTTP or archive stack. The lock diff for this work adds two dependency
  edges to `deadlock-core`, no new packages.
- The core build enables only `deflate`. A ZIP entry that uses deflate64, LZMA, bzip2 or zstd fails
  with the decoder error from the `zip` crate. That is a real limit, not a silent skip.
- 7z and rar are resolved from settings or `PATH` and run with an argument array, never through a
  shell: `7z x -y -bd -o<dest> <archive>` and `unrar x -o+ -p- <archive> <dest>`. Nothing is
  downloaded on demand.
- When the tool is absent the error names the format, the tool and the environment variable:
  `Archive backend unavailable: 7Z archives need 7z, which was not found. Set DAH_7Z ...`.
  This is the common outcome on macOS today: Deadlock uploads on GameBanana are often `.7z`.
- The external tool does its own path handling. The tree it produced is scanned afterwards: no
  symbolic links or special files, entry and byte caps enforced, every path relative. The scan and
  the exact argument array are tested with a stand-in script, not with 7z itself.

## Extraction safety

Every ZIP entry name goes through `safe_entry_path` before anything is written:

- rejected: empty names, backslashes, absolute paths, drive letters, `.` and `..` segments, empty
  segments, colons (NTFS alternate streams), control characters, and anything that resolves outside
  the extract folder;
- rejected: symbolic links and any unix mode that is not a regular file or folder;
- rejected: more than 10,000 entries, more than 4 GiB uncompressed in total, or one entry above
  8 MiB that expands more than 500:1 (compression bomb);
- a failed extract removes the whole destination tree, so a half-extracted payload is never left
  behind to be installed later.

The caps are `ExtractLimits` with defaults from constants, which is why the tests can lower them
instead of building a 4 GiB fixture.

## Classification

`classify_extracted` walks the tree once and picks exactly one candidate:

1. A `pakNN_dir.vpk` anywhere in the tree wins, but only if there is exactly one. More than one is
   refused: the installer will not guess which file is the mod. The VPK's entries are read and its
   internal chunk references are checked against the chunk files next to it, so a directory file
   whose chunks are missing is refused instead of deployed broken. Other files in the archive are
   reported as "outside the VPK, not installed".
2. Otherwise the tree is the candidate. The game path prefix is detected from
   `game/citadel/`, `citadel/`, `game/` or nothing, deepest first, and must cover at least one
   compiled file. A shallower prefix is not preferred just because it covers more files: mapping a
   second tree such as `game/core/...` into the citadel root would ship wrong game paths. Files
   outside the chosen prefix are listed as unmapped and are never installed.
3. Mapped `_c` files become the payload. Mapped files that are not compiled are listed and skipped;
   unmapped files are listed as well. Both lists appear in the plan warnings.

## Payload validation

The candidate's mapped paths are checked with the existing `payload.rs` rules, not with a copy. Any
error rejects the candidate with the offending paths in the message: `scripts/*.vdata_c`,
`*.vnmskel_c`, source assets, executables, archives, absolute or parent paths, duplicates. A VPK
candidate's entries are checked the same way, so a packaged mod that ships gameplay data is refused
just like a loose tree.

Validator warnings are grouped by rule (`not_compiled`, `not_compiled: 328 entries: ...`) because a
map mod can ship hundreds of config files and one line per file makes the plan unreadable.

## Slot rules

- `--slot N` wins, otherwise the lowest free 01..99.
- A slot is taken when `pakNN_dir.vpk`, `pakNN_dir.vpk.disabled` or any `pakNN_MMM.vpk` chunk
  exists.
- A taken slot refuses without `--force`, and the error names the exact file or files that would
  be replaced. `--force` plans the replacement, lists every affected file, and the apply step backs
  each one up before writing.
- Installing over a `.disabled` file backs it up and removes it, so the slot never ends with both
  `pakNN_dir.vpk` and `pakNN_dir.vpk.disabled`. `dah addons enable` would refuse that state.
- Lower slots win a shared path, so the plan reports overlaps with lower slots as conflicts rather
  than pretending the new mod will win.
- Chunk files of the slot that the new payload does not use are backed up and removed at apply
  time. A leftover `pakNN_001.vpk` from a previous, larger mod would otherwise still be loaded.
- `gameinfo.gi` is read, `gameinfo_repair` is planned when the `Game citadel/addons` line is
  missing, and apply repairs only that line through the existing `deploy::repair_gameinfo`.

## Apply, journal and status

`apply_install(plan, approve)` refuses when approval is missing, when the plan target does not match
the slot, when `game/citadel` disappeared, when the artifact no longer matches the plan's file list
(re-read from the VPK or the staging tree), or when the addons folder changed since planning. That
last check is what stops a plan from overwriting a file that appeared after it was made.

Writes:

- backups of every affected file under `.dah/install/backups/<stamp>/`, before any replace;
- a tree candidate is packed in the workspace with the existing `vpk_write` (directory file plus
  chunks) and copied into addons; a VPK candidate is copied as-is, with its chunk names renamed to
  the chosen slot;
- the directory file and chunks land through a sibling temporary file and a rename;
- one JSON line appended to `<workspace>/installs.jsonl` with plan id, source, archive sha256, slot,
  target, entry count, payload bytes, VPK size and sha256, backups, removed files, chunk names and
  the gameinfo result.

`dah install list` re-reads the journal and stats each target, so "present" means a file is on disk
now, not that it was written once. An entry superseded by a later install of the same slot is
marked. A journal line that does not parse is an error naming the line, not a silent skip.

## Workspace layout

```text
<workspace>/downloads/<source-key>/<archive>       source-key: gamebanana-<id>, url-<hash>, local-<hash>
<workspace>/downloads/<source-key>/extracted/
<workspace>/downloads/<source-key>/staging/        compiled tree candidates only
<workspace>/.dah/install/plans/<plan-id>.json
<workspace>/.dah/install/backups/<stamp>/
<workspace>/.dah/install/build/<plan-id>/           packed VPK before it is copied
<workspace>/installs.jsonl
```

`dah install cache prune` deletes `downloads/*` and nothing else. An unapplied plan whose staging
tree was pruned refuses to apply; plan again.

## Verified

Unit tests in `install.rs` (run through a throwaway crate that includes the real modules, so the
real `error.rs`, `util.rs`, `vpk.rs`, `vpk_write.rs`, `deploy.rs`, `discovery.rs`, `payload.rs` and
`addons.rs` are compiled):

```text
running 23 tests ... test result: ok. 23 passed; 0 failed
```

The install tests cover:

- every merged-source case: id, mod URL, direct URL, local file, and the forms that must be refused;
- the allowlist, including `filecache52.gamebanana.com`, `notgamebanana.com` and https-on-a-port;
- redirect resolution, and refusal of relative redirect targets;
- zip-slip (`../`, absolute, drive letter, backslash), symlinks, device file types, entry caps, byte
  caps, ratio bombs;
- prefix detection for `game/citadel/`, `citadel/`, bare game paths, a second tree and an empty tree;
- the GameBanana profile shape, using a trimmed copy of the live `Mod/715100/ProfilePage` response;
- end-to-end over a 127.0.0.1 server: download, extract, validate, plan, refuse without approval,
  apply, VPK contents, journal, status, second-plan conflicts, cache prune;
- rejected payloads with the exact offending paths (gameplay data, skeleton, source asset,
  executable);
- an occupied slot needing `--force`, with backup and replacement;
- a disabled slot file being replaced and removed;
- an archive that already ships its own `pakNN_dir.vpk` with a chunk, renamed to the chosen slot;
- pruned artifacts, a stale plan and a vanished game folder all refusing at apply time;
- the external backend argument array and post-extraction scan, with a stand-in script.

Live checks against GameBanana (2026-11-16), into a fixture game folder under `.scratch`, never into
a real install:

- Mod `733166` (`boxrun_25.zip`, 3,760,861 bytes): resolved through API v11, downloaded from
  `filecache52.gamebanana.com`, extracted in process, found one VPK at
  `game/citadel/addons/pak77_dir.vpk` with 331 entries and a 45.4 MiB payload, planned slot 01,
  refused without approval, then applied 47,626,685 bytes as `pak01_dir.vpk` and repaired the
  addons SearchPath. A second run reused the cached archive and planned slot 02 with 331 conflicts
  against slot 01.
- Mod `715100` (`drifter_taokaka_3d31a.7z`, 14,453,644 bytes): downloaded to the cache and refused
  with the 7z backend error, because 7z is not installed here.

## Not verified, and gaps

- Real 7z and rar archives. The invocation and the post-scan are tested, the extraction itself is
  not. This is the main gap, and Deadlock mods on GameBanana are often `.7z`.
- md5. GameBanana publishes it; this pipeline records sha256 only.
- The 2 GiB download cap and the 4 GiB extraction cap at their real sizes; the guards are tested
  with lowered limits and with a single oversized entry.
- A real deflate compression bomb. The ratio guard is unit tested on its own; the fixture ZIPs are
  stored entries.
- Windows-specific path behaviour (reserved device names, long paths, case-insensitive collisions).
  Names with colons, backslashes and control characters are rejected on every platform.
- Chunked VPK candidates. The rename path (dir plus `pakNN_MMM.vpk`) is tested with the crate's own
  writer; no third-party multi-chunk addon has been read.
- No progress reporting. A 2 GiB download blocks for up to 300 s with no output.
- `dah install` is not reachable from `rust/crates/dah-rs/src/main.rs` yet. The command functions
  are in `commands/install.rs`; the module declaration and subcommand block are handed to the
  integrator because `main.rs`, `commands/mod.rs` and `lib.rs` are owned elsewhere in this change.
- `pack_vpk` always writes chunk files, so a tree candidate always deploys `pakNN_dir.vpk` plus at
  least `pakNN_000.vpk`. Real mods often ship a single directory file with inline data; that case is
  not produced by this writer.
