# Contributing to Sigil

Thanks for helping. Sigil is a small fan project, so short, focused reports and changes get handled
fastest. By taking part you agree to the [code of conduct](CODE_OF_CONDUCT.md).

## Report a bug

Open an issue with the [bug report form](https://github.com/superuserkalo/sigil/issues/new/choose).
Include:

- the Sigil version (Settings > About and updates) and your Windows version;
- what you did, what you expected and what happened, as steps someone else can follow;
- the GameBanana link of the mod involved, if there is one;
- the reference code from Send feedback or Send diagnostics, if you sent one, or a screenshot.

Sigil's own error messages have Send feedback, which shows the scrubbed details before anything is
sent and can open a prefilled GitHub issue instead.

Security problems are different: follow [SECURITY.md](SECURITY.md) and do not post details in a
public issue.

## Propose a change before writing it

For anything bigger than a small fix, open an issue (the feature request form works) and describe
the problem and the change you have in mind before you write code. Changes that move a boundary
(process split, storage, network hosts, what gets written to the game) need a decision record in
[docs/decisions/](docs/decisions/) and the maintainer's agreement first.

## Set up and build

You need git, Rust through rustup (the toolchain is pinned in `rust/rust-toolchain.toml` and
installs on first build) and, for the scripts and the Worker, Node 24.

```bash
git clone https://github.com/superuserkalo/sigil.git
cd sigil
./scripts/vendor-rune.sh                                    # pinned Rune checkout in vendor/rune
cargo run --manifest-path rust/Cargo.toml -p dah-desktop    # start the app
```

The app builds and runs on Windows, macOS and Linux for development. Only Windows is released, and
real game features need a Deadlock install. [docs/runbooks/development.md](docs/runbooks/development.md)
has the details: the optional Locker art, headless screenshots, environment variables and the
Windows build.

## Run the tests that matter

```bash
cd rust && cargo test -p dah-desktop -p deadlock-core     # app and domain core
cd services/telemetry && npm ci && npm test               # the Cloudflare Worker, if you touched it
```

If you changed `rust/crates/sigil-viewer`, also run `cargo test -p sigil-viewer`. Tests never need
a game install: they build synthetic VPKs and fake install trees. CI runs the domain core, the app
and the Worker tests on every pull request.

## Code style

- Format Rust with `cargo fmt` (the pinned toolchain includes rustfmt) and keep clippy quiet on the
  code you touch.
- No em dashes in code, comments, UI strings or docs. Use a colon, a period or parentheses.
- Every new UI string needs its German entry in the matching `*_de.rs` or `locales*.rs` table.
- A behaviour change comes with a test in the owning crate, and with an update to the doc that
  describes it ([docs/user-guide.md](docs/user-guide.md), [docs/privacy.md](docs/privacy.md) or the
  relevant runbook).
- Use the words in [CONTEXT.md](CONTEXT.md). [AGENTS.md](AGENTS.md) has the full set of project
  rules; it applies to people too.

## Commits and pull requests

Commit subjects follow Conventional Commits, in the imperative:

```text
feat(desktop): add a load order lock
fix(core): keep the addons line when another tool added one
docs: record the community crosshair library
```

One concern per pull request. Say what changed, why, and how you tested it; the pull request
template asks for exactly that. Stage explicit paths and keep unrelated refactors out.

## What will not be merged

- Third-party analytics or crash services (Google Analytics, Sentry SaaS and the like), or any
  heartbeat.
- Anything that sends player data without an explicit opt-in that is off by default, or a network
  host that is not on the documented allowlist in [docs/privacy.md](docs/privacy.md).
- Code copied from GPL projects. Sigil is MIT. In particular, never copy code from Deadlock Mod
  Manager: it is GPL-3.0. Studying how it behaves is fine; its code is not.
- Game files, VPKs, mods, CSDK binaries, API keys or other secrets in the repository.
- Agent tools that write to the game install. Installs, `gameinfo.gi` edits and launches are app
  actions the player triggers.

## Licence

By contributing you agree that your contribution is licensed under the MIT licence in
[LICENSE](LICENSE). Third-party material keeps its own licence and attribution; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
