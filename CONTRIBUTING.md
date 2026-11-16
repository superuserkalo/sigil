# Contributing

Status: active.
Updated: 2026-11-16

Read `AGENTS.md` first. It contains the operating rules, the repository layout and the Deadlock
constraints. `CONTEXT.md` defines the vocabulary.

## Before a change

- `pnpm typecheck` and `pnpm test` must pass.
- Add or update a test when you change a contract, especially payload validation and game discovery.
- Update the affected doc. If the change moves a boundary, add a decision record under
  `docs/decisions/`.
- Keep mods cosmetic. Do not add gameplay data, skeleton replacement or anything that gives an
  advantage in matchmaking.

## Commits

Conventional subject, imperative mood:

```text
feat(cli): add doctor
fix(tools): reject vdata payloads
docs: record build backend decision
```

Stage explicit paths. Do not commit `vendor/`, `.scratch/`, credentials, SQLite stores, VPKs or
logs.

## Pull requests

- One concern per pull request.
- Explain what changed, why, and how you verified it.
- Do not include unrelated refactors.
- Do not claim a stub works. Mark phase status honestly in the docs.

## License

By contributing you agree that your contribution is licensed under the MIT license in `LICENSE`.
Third-party material keeps its own license and attribution; see `THIRD_PARTY_NOTICES.md`.
