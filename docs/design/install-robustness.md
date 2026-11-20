# Install robustness: every mod shape, three apps

Status: current, 2026-11-20. The shapes and failures below are handled in
`rust/crates/deadlock-core/src/install.rs` and `install_shapes.rs`, covered by generated fixtures
(`install_shape_tests.rs`) and by a corpus of 64 real GameBanana uploads
(`rust/crates/deadlock-core/tests/install_corpus.rs`, run by hand). Not verified on Windows with
a real game install: read-only attributes and long paths are tested on macOS only.

## Why

The owner reported install failures "left and right" (2026-11-20). The last one, GameBanana mod
679331, shipped one `Josuke Remaster.vpk` in a RAR5 and was refused with "No compiled _c files".
Commit `1e1b842` fixed that file. This document compares how Grimoire (MIT, code read and partly
ported) and Deadlock Mod Manager (GPL-3.0, behaviour only) accept mods, and records what Sigil
does for each shape.

## How each app installs

- **Grimoire** extracts only `*.vpk` files (any name, case-insensitive) from `.zip` (adm-zip),
  `.7z` (bundled 7za) and `.rar` (node-unrar-js, then 7za, then unrar), flattens them into its
  disabled folder under free-form names and asks the player to pick when there are several.
  Everything else in the archive is dropped without a word. Cursor packs are the one loose-file
  mod it installs.
- **Deadlock Mod Manager** extracts `.zip`, `.7z` and `.rar` in process (the same `zip`,
  `sevenz-rust2` and `unrar` crates Sigil uses), collects every `.vpk` (case-sensitive), offers a
  checkbox list when there are several, and renames each to the next free `pakNN_dir.vpk`. Loose
  fonts under `panorama/fonts/` are offered for a font install; `gameinfo.gi`, `.cfg` and
  `video.txt` files are staged for review, never installed. Downloads are retried and checked
  against GameBanana's size and MD5.
- **Sigil** does the union, and packs loose compiled files into a VPK, which neither reference
  app does.

## Archive and file shapes

| Shape | Grimoire | DMM | Sigil |
| --- | --- | --- | --- |
| `.zip`, deflate | yes | yes | yes |
| `.zip` with deflate64, LZMA, bzip2 or zstd entries | via adm-zip / 7za | yes | yes (zip features, crates already in the lock) |
| `.zip` names not UTF-8 (GBK, Shift-JIS) | latin1 fallback | raw | ZIP code page fallback, no refusal |
| `.zip` names with `\` separators | yes | yes | yes (were refused before) |
| `.zip` listing one name twice | 7z fallback | error | last copy wins |
| `.rar` RAR4 and RAR5 | yes | yes | yes (UnRAR in process) |
| split RAR (`.part1.rar` ...) | partly | no | part 1 reads the rest; a later part alone says which file to import |
| `.7z` (LZMA, LZMA2, BCJ, PPMd, bzip2, zstd, deflate) | 7za | yes | yes |
| format by magic bytes, not extension | 1-click only | no (extension, case-sensitive) | yes |
| bare `.vpk` download or file | yes | update path only | yes, with its `_NNN` parts from the same folder |
| archive inside the archive | no | no | yes, 3 levels deep, at most 64 inner archives, one size and file budget shared with the outer archive |
| `.tar`, `.gz`, `.bz2` | no | no | no; the message says to unpack with 7-Zip |
| password-protected | no detection (may hang on 7za) | raw library error | clear message for ZIP, 7z and RAR |
| corrupt or cut-off archive | raw error, archive deleted | raw error | "damaged or only partly downloaded", cached copy deleted so a retry downloads again |
| HTML error page saved as the archive | install may pass | raw error | says it is a web page |
| empty archive | silent success | "no VPK" | "the archive is empty" |

## Finding the mod inside

| Shape | Grimoire | DMM | Sigil |
| --- | --- | --- | --- |
| `pakNN_dir.vpk` at any depth | yes | yes | yes |
| any name: `Josuke Remaster.vpk`, `skin_dir.vpk` | yes | yes (lower-case `.vpk` only) | yes, any case; a non-`_dir` file must carry the VPK header |
| browser copy names: `pak98_dir (1).vpk`, `- Copy` | as any name | as any name | normalised, so its parts still match |
| VPK sets: `<name>_dir.vpk` plus `<name>_NNN.vpk` | broken (each part is a mod) | broken (each part is a mod) | grouped, parts renamed into the slot; a missing part is named |
| two variant folders with the same VPK names | kept (suffix) | kept (qualified name) | kept, parts matched per folder |
| several VPKs | picker | checkbox list | picker, labels are the archive path |
| `__MACOSX/._pak01_dir.vpk` AppleDouble | taken for a VPK | taken for a VPK | dropped with `.DS_Store`, `Thumbs.db`, `desktop.ini` |
| a map: addon VPK holding `maps/<name>.vpk` | as a VPK | as a VPK | as a VPK (it failed before: no `_c` inside) |
| a loose `maps/<name>.vpk` | as an addon (wrong) | as an addon (wrong) | packed as `maps/<name>.vpk` into an addon VPK |
| loose compiled files at `game/citadel/`, `citadel/`, `addons/`, a mod-named folder, deeper | dropped | dropped | packed; the game path starts at the first game folder (`materials`, `models`, `panorama`, `sounds`, ...) |
| loose variant folders (`Red/materials`, `Blue/materials`) | dropped | dropped | picker of folders, each packed |
| source files (`.vmat`, `.png`, `.wav`) beside compiled ones | dropped | dropped | left out with a warning (were refused before) |
| source files inside a creator's VPK, tool caches (`cache_*.soc`) | installed | installed | warning (were refused before) |
| programs (`setup.exe`, `Install Fonts.ps1`, `.bat`) beside the mod | 1-click prompt | ignored | left out with a warning, never run (were refused before) |
| readme, previews, licences | dropped | dropped | left out with a warning |
| fonts (`panorama/fonts/*.ttf`, or a font pack's `fonts/*.otf`) | no | "Install Fonts" offer | packed at `panorama/fonts/`; Installed > tools installs them into `fonts.conf` |
| localization text (`resource/localization/**/*.txt`), Panorama movies (`.webm`) | in a VPK only | in a VPK only | also loose, packed |
| cursor pack (`cursor*.bmp`, `cursor.res`) | installed loose | no | message: import from Locker > Cursors (Sigil's port of Grimoire's packs) |
| `gameinfo.gi` replacement | no | staged for review | refused with why; points to the Performance page, which applies the listed GameBanana configs as a reversible merge |
| `.cfg` / autoexec mod | no | staged for review | refused with what to do (Autoexec page, or copy to `game/citadel/cfg`) |
| gameplay data (`scripts/*.vdata_c`) or skeletons | installed | installed | refused unless Settings > Preferences > Allow script mods is on; the message names the setting |

## Downloads

| Behaviour | Grimoire | DMM | Sigil |
| --- | --- | --- | --- |
| HTTPS allowlist, redirects checked | yes | yes | yes |
| retry a dropped connection or a 5xx | mirror failover, resume | 3 retries with backoff, resume | 2 retries (1 s, 3 s); 4xx, caps and the allowlist are not retried |
| short body against `Content-Length` | rejected | rejected | rejected (was accepted) |
| GameBanana size check | yes | yes | yes |
| GameBanana MD5 check | no | yes | yes, one fresh download, then a clear error |
| cached archive reused | no | partial resume | only when size and MD5 match the file record |
| archive that will not open | deleted | kept | deleted when downloaded (never a player's own file), so Install again re-downloads |

## Files on disk

| Behaviour | Grimoire | DMM | Sigil |
| --- | --- | --- | --- |
| read-only files from an archive | rename retries | "make writable" prompt | flags cleared at extract, staging and deploy; stale trees removed even when read-only |
| long paths | flattened names | no handling | Rust's verbatim paths on Windows; tested with a 230-character archive path |
| Unicode and spaces in names | yes | extraction fallback | yes |
| size limits | none | 100x ratio, 16 GB | 2 GiB download, 8 GiB unpacked, 50,000 files, 500:1 ratio above 8 MiB |

## Corpus

`cargo test -p deadlock-core --test install_corpus -- --ignored --nocapture` downloads 64 mods
across every GameBanana Deadlock category into `.scratch/install-corpus/` and plans each against
a fake game. Uploads that neither reference app installs (a `gameinfo.gi`, a `.cfg` folder, a
`.bat` installer, a `.js` template) are expected to be refused with the specific message.

Before this change, 35 of the first 49 passed. The 14 failures: 2 script mods with no hint, 3 maps
refused for a tool cache inside the VPK, 2 maps refused for having no `_c` files (their payload is
`maps/<name>.vpk`), 1 font pack refused for its `.ps1`, 1 installer and 1 config set refused for a
program or a `.png` beside them, and 4 config mods refused with the generic "No compiled _c files".
After it, 64 of 64 pass.

## Not done

- DMM's staged review of `gameinfo.gi`, `.cfg` and `video.txt` files is not built: Sigil explains
  and points to Performance and Autoexec instead.
- Resumable downloads across a restart (both references keep a partial file with its ETag).
- Splitting one GameBanana mod with several files: the newest file is taken unless the player
  picks another in Browse.
