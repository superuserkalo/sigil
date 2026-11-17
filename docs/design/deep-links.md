# Install with Sigil links

Status: current. Link formats, registration, single instance and the GameBanana 1-Click listing.
Updated: 2026-11-17

An "Install with Sigil" link opens a GameBanana mod in Sigil with one of its files chosen and the
normal Install dialog open. Sigil never installs from a link alone: the player presses Install in
the dialog (and the dialog's own plan and confirm steps run as they do for any Browse install).

Code: `rust/crates/desktop/src/deeplink.rs` (parser, handler, registration, Settings state),
`rust/crates/desktop/src/single_instance.rs` (one process per user), `main.rs` (startup order),
`screens/browse.rs::open_submission`, `scripts/package-desktop.sh` (macOS `CFBundleURLTypes`).

## How GameBanana 1-Click works

GameBanana's wiki page "1-Click Mod Installers" (`https://gamebanana.com/wikis/1999`, read through
`apiv11/Wiki/1999`) is the only public spec:

- A mod manager registers a custom URL scheme with the OS. GameBanana shows a button for it on
  compatible mod pages of the games it is enabled for.
- The link is `schema:[URL_TO_ARCHIVE]`, or with extra parameters
  `schema:[URL_TO_ARCHIVE],[MOD_TYPE],[MOD_ID]`. Which form a manager gets "is decided upon
  between GameBanana and the mod installer programmer(s) during integration development".
- GameBanana checks archive compatibility per game and per manager server side, and hides the
  button when the check fails.
- Mod authors opt out with an empty `.disable_gb1click` (every manager) or
  `.disable_gb1click_{protocol}` (one manager, for Sigil `.disable_gb1click_sigil`) anywhere in
  the archive.
- Requirements the page lists: the scheme association, HTTP download, ZIP/RAR/7z extraction, and
  a prompt when the archive holds unusual files.

What the two Deadlock managers ship (read from `vendor/`):

- Grimoire (MIT): scheme `grimoire:`, link `grimoire:https://gamebanana.com/mmdl/<fileId>,Mod,<modId>`.
  It registers through electron-builder's `protocols` block and `setAsDefaultProtocolClient` at
  runtime, takes the link from `argv` on Windows and from the `second-instance` event, and keeps
  one process with `requestSingleInstanceLock`. Its download host check accepts
  `gamebanana.com`, `www.`, `files.`, `mods.` and `filecacheNN.gamebanana.com`.
- Deadlock Mod Manager (GPL-3.0, behaviour only): schemes `deadlock-mod-manager:`,
  `deadlock-modmanager:` and `dlmm:` through Tauri's deep-link and single-instance plugins. Its
  tests use `https://gamebanana.com/mmdl/12,Mod,42` and `https://files.gamebanana.com/mmdl/13,Sound,42`
  and reject other hosts, `http:` and types other than Mod and Sound.

The `mmdl/<fileId>` path segment is the GameBanana file id; the third field is the submission id.

## Link formats Sigil accepts

Scheme: `sigil` (any case). Every other link is refused with one short reason in the status line, shown as a toast
("Could not open the link: it points somewhere other than GameBanana.").

| Form | Example | Opens |
| --- | --- | --- |
| GameBanana 1-Click | `sigil:https://gamebanana.com/mmdl/1234567,Mod,741478` | Mod 741478, file 1234567 preselected, Install dialog open |
| GameBanana 1-Click, sound | `sigil:https://gamebanana.com/mmdl/13,Sound,42` | Sound 42, file 13 |
| Sigil link | `sigil://install/mod/741478` | Mod 741478, details only |
| Sigil link with file | `sigil://install/mod/741478/1234567` | Mod 741478, file 1234567 preselected |
| Handler wrapper | `sigil://open/<one of the above>` | What the inner link opens |

Rules, all enforced in `deeplink::parse` and covered by its tests:

- At most 512 characters, printable ASCII only, no backslash, no whitespace or control characters.
- 1-Click: `https://` only; host exactly one of `install::GAMEBANANA_DOWNLOAD_HOSTS`
  (`gamebanana.com`, `www.gamebanana.com`, `files.gamebanana.com`); no user info, no port; path
  exactly `/mmdl/<id>` or `/dl/<id>`; no query or fragment; type `Mod` or `Sound`; exactly three
  comma fields. The archive-only form (`sigil:https://gamebanana.com/mmdl/<id>`) is refused because
  Sigil needs the mod id to show the mod before an install.
- Ids: 1 to 10 ASCII digits, no leading zero, at most 4294967295.
- One trailing `/` is tolerated (some launchers add one). The wrapper is accepted once and only
  around a `sigil:` link.
- A launch that carries a `sigil:` argument and anything else exits before Rune reads its
  arguments. The registered command passes exactly one argument, so extra arguments mean a link
  broke out of the command's quotes to pass flags such as `--snap`.

Sigil never fetches the URL inside a link. It takes the two ids, opens the mod through the normal
Browse reads (`gamebanana.com/apiv11/<Model>/<id>/ProfilePage`, already on the network
allowlist), and installs the file by the download link the API returned, through the normal
install pipeline and its own HTTPS host allowlist. A link adds no host to the allowlist.

When the details arrive: a file that is no longer on the mod leaves the details open with "Could
not find that file on the mod. Pick one under Files."; an install already running keeps the
details open and says so. Closing the details or opening another mod drops the pending link.

## Registration

The Settings row Integrations > Links > "Open Install with Sigil links" is on by default. Sigil
writes the handler at every start while it is on (only the values that differ), and removes it
when it is turned off. The choice is a marker file, `<data>/links-off`, present when off.

Windows, per user, no admin (`HKEY_CURRENT_USER`):

```text
Software\Classes\sigil                      (default) = "URL:Sigil Install Link"
Software\Classes\sigil                      "URL Protocol" = ""
Software\Classes\sigil\DefaultIcon          (default) = "\"<exe>\",0"
Software\Classes\sigil\shell\open\command   (default) = "\"<exe>\" \"sigil://open/%1\""
```

Turning links off deletes `Software\Classes\sigil` (`RegDeleteTreeW`). The command wraps `%1`
because Rune's single-instance hand-off forwards only `scheme://` arguments to the running app;
GameBanana's opaque `sigil:https://...` form would otherwise be dropped on a second launch. The
wrapper turns it into `sigil://open/sigil:https://...`.

macOS: the app bundle declares the scheme (`CFBundleURLTypes`, name
`dev.deadlock.harness.install-link`, scheme `sigil`) in `scripts/package-desktop.sh`.
LaunchServices registers it when the bundle is installed; Rune receives the link as a get-URL
Apple event (`App::on_open_url`, see `vendor/rune/crates/rune/src/open_url/incoming.rs`).
Nothing is written at runtime. Turning the setting off makes Sigil ignore links (macOS still
starts or activates Sigil for them). A bare `cargo run` binary has no bundle and gets no links.

Linux: `$XDG_DATA_HOME/applications/sigil-links.desktop` (default `~/.local/share/applications`)
with `MimeType=x-scheme-handler/sigil;` and `Exec="<exe>" sigil://open/%u`, then
`xdg-mime default sigil-links.desktop x-scheme-handler/sigil`. Off removes the file.

## Single instance

`main` runs, in order: the `cli` and `agent-host` helper dispatch, the extra-argument check above,
then (skipped for headless runs: `--snap`, `--tree`, scripts, `--list-scenes`) the claim:

- Windows and Linux: `rune::single_instance("dev.deadlock.harness")`. Windows claims a named pipe
  per user session; a later launch calls `AllowSetForegroundWindow` for the first process, sends
  its arguments and exits. Linux binds a Unix socket in `$XDG_RUNTIME_DIR` beside a lock file.
  Links go to `App::on_open_url`; a launch without links brings the window forward.
- macOS: LaunchServices already keeps one process per bundle and sends links to it. For launches
  that bypass it (a terminal, `open -n`), Sigil takes an exclusive `flock` on `<data>/sigil.lock`
  (two first launches can't both win) and serves `<data>/sigil.sock` (mode 600). A later launch
  connects, sends `sigil-instance 1\n`, a length and its NUL-separated arguments, waits up to
  500 ms for one byte back and exits. It retries for up to 2 s while the first Sigil starts, and
  runs on its own if nobody answers. A data folder path too long for a socket runs unclaimed.

The running Sigil handles each link in `deeplink::on_open_url`, which also brings the window to
the front (`AppHandle::open`). No network is involved.

Cost: on the launch thread the macOS claim is one `open` and one `flock`; binding the socket and
serving it run on a thread of their own. The unit test measured the claim at 0.14 to 0.26 ms
(debug build, data folder already there, thread spawn included) and a second launch's hand-off
at about 2 ms (it includes waiting for the first to bind). Rune quotes 63 to 82 us for its
Windows pipe answer (release). Headless runs skip the claim, so the perf harness in
`.scratch/perf/` (which runs headless) does not see it.

## Getting listed on GameBanana

There is no public form. The wiki says the link format is agreed with GameBanana during
integration, and a forum question on that page about the process has no answer. What to do:

1. Publish Sigil as a Tool on GameBanana (the listed managers each link to a Tool page, for example
   HedgeModManager at `gamebanana.com/tools/6311`), with a download link to the GitHub releases.
2. Ask GameBanana staff to enable a 1-Click manager for Deadlock (game id 20948), through the
   site's support contact or the GameBanana Discord, and copy the Deadlock section's admins.
3. Send them:
   - Manager name `Sigil`, scheme `sigil`, Tool page and homepage links, and an icon (they show a
     small manager icon beside the button; send a square PNG, 32 and 64 px).
   - The format: `sigil:[URL_TO_ARCHIVE],[MOD_TYPE],[MOD_ID]`, the same extended form Grimoire
     (`grimoire:`) and Deadlock Mod Manager (`dlmm:`) use.
   - Types: `Mod` and `Sound` for Deadlock.
   - Compatibility rule for their server-side check: the archive contains at least one
     `pakNN_dir.vpk` (or any `.vpk`, which Sigil renumbers).
   - The opt-out file `.disable_gb1click_sigil`.
   - Platforms: Windows (registry handler) and macOS (bundle URL type).

Until GameBanana enables it, `sigil://install/mod/<id>` links work from any page that links them,
and a `sigil:https://gamebanana.com/mmdl/...` link can be tested by hand (Windows: `Win+R`, paste
the link; macOS with the bundle installed: `open 'sigil:https://...'`).

## Not done or not verified

- Windows registration and hand-off are unverified on a Windows machine. The registry code was
  type-checked against `windows-sys` 0.59 for `x86_64-pc-windows-msvc` in isolation; the whole
  crate does not cross-compile from macOS (C dependencies). That `%1` is substituted inside a
  quoted argument (`"sigil://open/%1"`) matches how other apps register (`"--url=%1"`), but it
  was not tested here, nor how each browser escapes the link before it reaches `%1`.
- The macOS bundle path (LaunchServices, Apple event, activation) was not exercised end to end;
  the single-instance socket was, in unit tests.
- Linux was not built or run; the desktop entry text is unit-tested only.
- The GameBanana listing does not exist yet; the real button and its exact link are unverified.
- Install pipeline follow-ups: honour `.disable_gb1click` and `.disable_gb1click_sigil` inside an
  archive (GameBanana already hides the button for such archives); Rune could accept
  `scheme:https://` launch arguments in `instance::is_link` so the wrapper is not needed.
