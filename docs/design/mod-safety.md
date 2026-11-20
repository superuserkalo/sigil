# Mod safety scan: what the game loads, what the reference apps flag, what Sigil flags

Status: current, 2026-11-20. Implemented in `rust/crates/deadlock-core/src/mod_safety.rs` (scan),
`rust/crates/deadlock-core/src/payload.rs` (`script_mod_kind`, shared with the install check) and
`rust/crates/desktop/src/screens/installed/safety_gate.rs` (the gate). Verified against GameBanana
mod 679331 (Shiv "Josuke Remaster", file 1871507): clear. Not verified on Windows.

## Why

The owner installed mod 679331, a Shiv skin, on 0.1.18. The gate said "Deadlock does not run these
files, but they are unusual in a mod" and listed `panorama/images/heroes/hero_names/shiv.vsvg_c`
and `shiv_localized.vsvg_c` as "Reaches out". Both are compiled Panorama SVG icons, which the game
ships 655 of. Two bugs made that happen:

1. The scan read `vsvg_c`, `svg` and `vsvg` as code ("Compiled script", "Page").
2. It matched any `http://` or `https://`, and every SVG names `xmlns="http://www.w3.org/2000/svg"`.

## 1. What Deadlock loads and runs

Sources: `docs/research/deadlock-modding.md`, and `SteamTracking/GameTracking-Deadlock` (master,
2026-11-20): `game/citadel/pak01_dir.txt` (the game's own file list), the decompiled
`pak01_dir/panorama/` tree, and the string dumps of `panorama.dll`, `panoramauiclient.dll` and
`client.dll`.

What the game itself ships under `panorama/` in `pak01`:

| Type | Count | What it is |
| --- | --- | --- |
| `vtex_c` | 2142 | Images (textures) |
| `vsvg_c` | 655 | Images (compiled SVG: hero icons, hero names, ability icons) |
| `vcss_c` | 454 | Styles |
| `vxml_c` | 445 | Layouts, most with inline script and `onactivate=` style event handlers |
| `vts_c` | 6 | Compiled TypeScript (`panorama/scripts/async.vts_c`, post-game pages) |
| `css`, `cfg` | 1 each | `panorama/browser/webkit.css`, `panorama/citadel_keybinds.cfg` |

`vjs_c` does not appear in the current build, but Panorama in Source 2 compiles `.js` to `vjs_c`
and HUD mods ship it. Outside `panorama/` the game also ships `scripts/vscripts/weapons/weapons.lua`
and 93 `scripts/**/*.vdata_c` (gameplay data). Anything the game ships in the same form is a normal
asset: a type is never a reason on its own, except for programs and loose source scripts.

Panorama's web and browser surface in this build, from the string dumps:

- `$.AsyncWebRequest` exists only as a stub: `panorama.dll` carries
  `ERROR: AsyncWebRequest has been removed.` A call does nothing today, but it is the classic
  Panorama network call and a mod that uses it is trying to reach a server.
- `CitadelHTMLPanel` and `HTML` panels with `SetURL` embed a browser (CEF). The game uses one in
  `popups/popup_news_post.xml`. Grimoire's safety feature was built for a since-fixed bug where this
  panel could open `file://` pages and send them out.
- `ExternalBrowserGoToURL` and `BrowserGoToURL` are Panorama events that open a page in the Steam
  overlay or the browser (`popups/citadel_popup_join_forum.xml` uses it).
- `ExecuteSteamURL` runs a `steam://` link.
- Images and styles can load from a URL: `panorama.dll` has `CLoadFileURLTask` with
  `HTTPRequestCompleted_t`, so `<Image src="https://...">` or `url("https://...")` is a fetch.
  `file://{images}/...` and `s2r://` resolve inside the game.
- `SteamOverlayURL` and `OpenExternalBrowserURL` do not appear in this build. Sigil still matches
  `OpenExternalBrowserURL` and `OpenURL`, as Grimoire does, since a later build could add them.

## 2. What Grimoire flags (MIT, `electron/main/services/modSafetyScan.ts`, `modSafetyPolicy.ts`)

Mod safety is an experimental setting, off by default (`experimentalModSafety`). Grimoire's own
explainer says Valve fixed the bug it was built for. When on, it scans each VPK (and nested VPKs):

- Skipped unread: assets (`vtex_c`, `vmat_c`, `vmdl_c`, `vnmskel_c`, sounds, fonts, `png`, `jpg`, ...)
  and any type that is not text, compiled text, a layout, an archive or a program.
- Read: `js vjs ts vts css vcss xml vxml html htm svg vsvg cfg lua nut`, compiled `vjs_c vts_c
  vcss_c vsvg_c` (DATA block) and `vxml_c` (DATA source, or the binary LaCo layout through its
  bundled decoder).
- `native-code` "Executable program": `exe dll com bat cmd ps1 vbs lnk msi so dylib wasm`.
- `executable` "Runs scripts": every JavaScript file (`js vjs vjs_c vts_c`), every `ts vts lua nut
  cfg`, and any layout with `<script>` or an `on...=` event handler. In practice every HUD mod.
- `browser` "Web content": `javascript:`, `CitadelHTMLPanel`, `HTMLPanel`, `HTMLTitle`,
  `HTMLFinishRequest`, `SetURL`, `SetURLWithParams`, `OpenURL`, `OpenExternalBrowserURL`.
- `remote-code` "Network requests or external code": `fetch`, `XMLHttpRequest`, `WebSocket`,
  `importScripts`, `RunScriptInPanelContext`, `import`, and `<script|include|iframe|object|embed
  src|href|url="https:|//|data:|blob:">`.
- `dynamic-code` "Code created at runtime": `eval`, `Function`, string decoders (`atob`,
  `fromCharCode`, `unescape`, `decodeURI...`), computed method calls, panel factories with a
  non-constant argument.
- `local-file`: `file:` other than `file://{root}/`, kept only when another active finding exists.
- `uninspectable`, `unreadable-archive`, `inspection-failed` when the scan cannot finish.

JavaScript is parsed with acorn, so names and constant strings are checked, not raw bytes. Layouts
drop XML and block comments first. A bare URL is never a finding, so SVG namespaces never trip it.
A flagged mod stays off until the player trusts that exact version (hash of the files).

## 3. What Deadlock Mod Manager flags (GPL-3.0, behaviour only)

DMM does not look inside a VPK for code. Its protections are elsewhere:

- A server-published policy manifest per GameBanana submission: `hidden`, `blacklisted`,
  `takedown` (hidden from the catalog, download refused) and `emergency_disable` (download refused).
- Install extracts only `.vpk` files from an archive; `gameinfo.gi`, `.cfg` and `video.txt` are
  staged for review, never installed (see `design/install-robustness.md`).
- Remote install sources must be an archive name over HTTPS from the source's host with a matching
  SHA-256; a `.exe` name is refused.

So DMM never warns about icons, layouts or scripts inside a VPK.

## 4. Sigil's rule per type

The scan lists a file only for a concrete reason, and `Finding::reason` says which. Every finding
opens the gate when the mod is turned on (unless Settings > Allow script mods is on, or the player
trusted that version).

| Files | Read? | Listed when | Label and reason shown | Why |
| --- | --- | --- | --- | --- |
| Images: `vtex_c vsvg_c svg vsvg png jpg tga dds` | no | never | | The game ships 2800 of them; an image runs nothing. Grimoire skips `vtex_c`/`png` but read `vsvg_c`; it never flagged bare URLs. |
| Other assets: models, materials, sounds, particles | no | never | | Normal assets. |
| Panorama layouts and styles: `vxml_c vcss_c xml vxml css vcss html htm` | yes | text calls a web or browser API, loads a URL, or builds code | "Reaches out" or "Hidden code", with the API or URL | The game ships 900 of them. Event handlers and inline script are normal HUD mod content; Grimoire flags them as "Runs scripts", Sigil does not. |
| Panorama scripts: `vjs_c vts_c` | yes | same as layouts | same | The game runs them and ships `vts_c` itself; inside Panorama they have no network API but the ones listed. Grimoire flags every script; Sigil flags what it does. |
| API calls in that text: `$.AsyncWebRequest`, `SetURL`, `CitadelHTMLPanel` or `<HTML`, `ExternalBrowserGoToURL`, `BrowserGoToURL`, `OpenURL`, `OpenExternalBrowserURL`, `ExecuteSteamURL`, `fetch(`, `XMLHttpRequest`, `WebSocket`, `importScripts(`, `javascript:` | | match on a whole word | "Reaches out: path (API)" with a sentence per API | Section 1. Searched in the full text, so a comment cannot hide a call. |
| URL loads: `src=`, `href=`, `url=` attribute, CSS `url(...)`, `SetImage(...)` with `http:`, `https:` or `//` | | match outside comments | "Loads <url> from the internet when the game shows it." | Panorama fetches these. `xmlns=`, doctype URLs, `xlink:href` and credits in comments do not match. |
| Code built at runtime: `eval(`, `new Function(`, `Function("`, `atob(`, `fromCharCode(`, `RunScriptInPanelContext` | | match on a whole word | "Hidden code" | Grimoire's `dynamic-code`; the usual way to hide a URL. |
| Programs: `exe dll com bat cmd ps1 vbs lnk msi so dylib wasm` | no | always | "Program" | Never needed by a mod. Same list as Grimoire. The install check refuses them outside a VPK. |
| Source scripts: `js vjs ts vts lua nut` | no | always | "Source script": the game only runs compiled scripts | Unusual in a mod; Grimoire lists them too. |
| Console configs: `cfg` | no | always | "Source script": "Console config: the game runs its commands when a bind or another config executes it." | Can run console commands. DMM stages loose `.cfg` for review. |
| Script mods: `*.vnmskel_c`, `scripts/**/*.vdata_c` | no | always | "Script mod" with the install check's message | One definition, `payload::script_mod_kind`, for both the install check and the scan, so "Allow script mods" covers the same files in both places. |

Limits: the scan matches text, not a parsed AST, so a script that builds an API name from pieces
(`$["Async" + "WebRequest"]`) gets through unless it also uses `atob`, `eval` or the like. A binary
LaCo `vxml_c` is searched as bytes; its identifiers and strings are stored as text, so names match,
but this is not a full decode as Grimoire's bundled decoder does.
