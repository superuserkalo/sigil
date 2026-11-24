# Packs

Status: active (0.1.38). Core `rust/crates/deadlock-core/src/packs.rs`, UI
`rust/crates/desktop/src/screens/packs.rs`.

A pack is a named set of GameBanana mods a player shares so friends run the same mods.

## What the reference apps do

- Grimoire (MIT): a profile shares as an `mp1:` code (base64url of gzip `mod-profile` JSON) or a
  `.modprofile.json` file. Import looks each mod up on GameBanana (exact, upgraded or unavailable),
  can skip NSFW, downloads, then saves a profile.
- Deadlock Mod Manager (behaviour only): a profile is uploaded to its server and shared by id.
  Import installs in the shared load order and lands mods on.

## Sigil

- Format: the same `mod-profile` document and `mp1:` code, so Sigil's and Grimoire's profile
  import read a pack too. A pack holds only the format and version, a name (64 characters), an
  optional note (300), and per mod the GameBanana mod id, file id (0 is "newest"), place in the
  load order (`priority`) and on flag. It carries `extensions.sigil.pack = 1` so Sigil tells a pack
  from a profile. No paths, URLs, crosshair or autoexec lines.
- Share: a code (Copy code), a link `sigil://pack/<code>` (Copy link), a `.sigilpack.json` file.
- Make: Installed > More > Make a pack (every enabled mod, in load order) or the selection bar's
  More > Make a pack from N mods. Local files have no GameBanana id: they are listed as not
  shareable and left out.
- Packs list: Installed > More > Packs, or the Packs button on the Profiles page (same dialog). Saved packs (yours and received) with Install, Share,
  Update from current mods, Rename, Delete; Paste a code and Open a file. Stored in
  `<workspace>/.dah/packs/`.
- Receive: Browse > Paste mods, Packs > Paste a code or Open a file, or a `sigil://pack/` link.
  Each opens the preview: every mod with its picture (blurred or hidden per the NSFW setting),
  size, file, NSFW tag and state (installed already, will install, needs review, removed from
  GameBanana, no file, could not check). Install pack and Cancel.
- Install: each mod goes to the download queue (Downloads page) through the normal one-press
  install: install checks, backups, safety gate. Mods install one at a time in pack order so each
  takes the next free slot and the load order holds. With Help build Sigil on, a `pack_install`
  record lists the GameBanana ids queued (never the name or note); the shared file is used, or the newest when
  it is gone. They land off unless "Turn on after installing" is ticked (a mod off in the pack
  stays off; the safety gate still holds a mod it has a question about).

## Safety

A pack is untrusted input. `packs::read_pack_text` caps the code at 64 KiB, the inflated JSON at
256 KiB, the mods at 300, the name and note; refuses control and invisible characters, wrong
types and ids outside 1 to 4294967295; ignores unknown fields; never reads a path from pasted text.
The preview's only network host is GameBanana; download links and images from GameBanana's answer
are used only on GameBanana hosts. Nothing installs before the player presses Install pack.

## Not done

- A mod with several VPKs in one archive opens the normal VPK picker during the install.
- The "needs review" guess uses the GameBanana category; the real check runs on the files.
