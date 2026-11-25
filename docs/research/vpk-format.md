# VPK container format

Status: active. Verified against reference readers, not yet against a local Deadlock install.
Updated: 2026-11-16

This note records the facts the game index depends on. The parser lives in
`rust/crates/deadlock-core/src/vpk.rs` (the writer is `vpk_write.rs`). When this note disagrees
with a real Deadlock file, the file wins; update both.

## Why this matters

The game index needs to answer three questions without the game running:

1. What exact paths exist inside `game/citadel/pak01_dir.vpk`?
2. Which chunk file holds the bytes for a path?
3. Can those bytes be copied into the mod workspace safely?

The index caches only metadata. Every extraction re-reads the directory tree, checks the CRC32 and
writes inside the workspace.

## Container layout

A VPK is a directory file plus zero or more numbered chunks:

```text
pak01_dir.vpk      header, directory tree, optional embedded data, checksum sections
pak01_000.vpk      raw entry data
pak01_001.vpk      raw entry data
```

The directory file name ends in `_dir.vpk`. A chunk index `N` maps to `<stem>_NNN.vpk`, where
`<stem>` is the directory name without `_dir.vpk`, and `NNN` is zero padded to three digits.

## Header

All integers are little endian.

Version 1, 12 bytes:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | Magic `0x55AA1234` |
| 4 | 4 | Version `1` |
| 8 | 4 | Directory tree size |

Version 2, 28 bytes:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | Magic `0x55AA1234` |
| 4 | 4 | Version `2` |
| 8 | 4 | Directory tree size |
| 12 | 4 | File data section size |
| 16 | 4 | Archive MD5 section size |
| 20 | 4 | Other MD5 section size |
| 24 | 4 | Signature section size |

Source 2 games, including Deadlock, use version 2. Respawn's modified format (`0x00030002`) is
rejected. The parser also rejects unknown versions instead of guessing.

## Directory tree

The tree is a sequence of null terminated UTF-8 strings, nested three levels deep:

```text
for each extension:
  for each directory:
    for each file name:
      18 bytes of metadata
      preload bytes
```

An empty string ends the current list. A final empty extension ends the tree.

Special values:

- Extension `" "` means the file has no extension. The full path has no trailing dot.
- Directory `" "` means the root directory.
- File name `" "` is a directory-only placeholder and is skipped. It is not a real file.

Preload bytes are part of the tree, not the data section. They are concatenated before the archive
bytes when a file is read and are included in the CRC32.

## Entry metadata

Each entry is 18 bytes:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | CRC32 of preload bytes plus data bytes |
| 4 | 2 | Preload byte count |
| 6 | 2 | Archive index |
| 8 | 4 | Offset |
| 12 | 4 | Data length, excluding preload |
| 16 | 2 | Terminator `0xFFFF` |

Archive index `0x7FFF` means the data is in the directory file. Any other index selects a chunk.

Offset rules:

- Directory file: offset is relative to the start of the file data section, which is
  `headerSize + treeSize`.
- Chunk file: offset is relative to the start of the chunk file.

This is the detail most easy to get wrong. Both SteamDatabase/ValvePak and SourcePP/vpkpp use the
same bases.

## Checksum sections

After the file data section, version 2 stores an archive MD5 section, an other MD5 section and an
optional signature section. Their sizes come from the header. The index does not validate these
sections yet. It validates each extracted file against the entry CRC32 instead.

## Safety rules in the parser

- Reject the wrong magic, an unknown version, and a tree that runs past the file.
- Reject an entry that ends in the middle of metadata or preload bytes.
- Reject a terminator other than `0xFFFF`.
- Skip paths that are absolute, use backslashes, contain a null byte, or contain `.` or `..`
  segments. Count them so the index summary is honest.
- Check every data range against the actual file size before reading.
- Never write to the game folder. Extraction targets the mod workspace only.

## What is still unverified

- No Deadlock install was available on this machine. The fixture tests synthesize version 1 and 2
  files, including embedded data, chunk data and preload bytes.
- On the first real install, run `dah index` and compare the entry count and hero folder names with
  the in-game asset browser. If the game uses a format extension we did not model, the parser should
  fail loudly rather than return a partial index.
- Compressed VPK variants (the Respawn/`vpkpp` compression path) are out of scope. Deadlock's main
  VPK is expected to be the standard version 2 container.

## Sources

- [Valve Developer Community: VPK (file format)](https://developer.valvesoftware.com/wiki/VPK_(file_format))
- [Source2Wiki: VPK](https://www.source2.wiki/FileFormats/vpk)
- [SteamDatabase/ValvePak: Package.Read.cs](https://github.com/SteamDatabase/ValvePak/blob/master/ValvePak/ValvePak/Package.Read.cs)
- [craftablescience/SourcePP: vpkpp VPK.cpp](https://github.com/craftablescience/SourcePP/blob/master/src/vpkpp/format/VPK.cpp)
- [craftablescience/VPKEdit](https://github.com/craftablescience/VPKEdit)
- [ValveResourceFormat](https://github.com/ValveResourceFormat/ValveResourceFormat)
- [Deadlock pak01 corruption report, official forums](https://forums.playdeadlock.com/threads/pak01-vpk-corruption-workaround.67321/)
- [Deadlock mod installation guide, Steam Community](https://steamcommunity.com/sharedfiles/filedetails/?id=2810096728)
