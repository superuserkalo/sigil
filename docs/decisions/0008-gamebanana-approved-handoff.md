# 0008 GameBanana publishing is an approved browser handoff

Status: accepted
Date: 2026-11-16

## Context

Publishing is a Tier 5 action: the user wants to put a finished mod on GameBanana without handing
their account to the app. GameBanana's public API exposes authentication and read endpoints, but
not a documented mod submission endpoint. A scraped web session or an undocumented upload form
would be fragile, hard to test, and outside a stable contract.

The product already promises BYOK-style local control: no passwords in the repository, no silent
network effects, and explicit approval before anything leaves the machine.

## Decision

- `dah publish prepare` produces a release folder with a ZIP archive, metadata, install notes and a
  SHA-256 of the archive.
- `dah publish upload` is a plan by default. With `--approve` it writes
  `<publish>/<version>/upload.json` in state `awaiting_user`, records the archive hash and submit
  URL, and optionally opens the GameBanana submission page.
- The user signs in and uploads the archive in their own browser. The app never reads or stores a
  GameBanana password or session cookie.
- After the upload, `dah publish confirm --url <submission>` re-checks the archive hash, fetches the
  submission page and verifies that it names the project, then records state `verified`.
  `--skip-verify` records `recorded` instead. `--allow-http` exists only for a local test server.
- `dah publish status` reports the durable state: `awaiting_user`, `verified` or `recorded`.
- The whole flow is covered by a local mock submission test. A real public submission requires the
  user's account and is not automated.

## Consequences

- No GameBanana credential is ever stored or forwarded.
- The handoff is auditable: archive hash, timestamps, submit URL and submission URL live next to the
  release.
- The flow works today without depending on an undocumented API.
- A real API integration can replace the browser step later without changing the preparation,
  approval or state model.
- The app cannot detect a deletion or edit of the submission beyond the one verification request.

## Alternatives

- Scrape the web session or post the undocumented submit form: brittle, likely against the site's
  terms, and untestable without an account.
- Require a GameBanana API password: no documented upload endpoint exists to use it.
- No upload path at all: fails the Tier 5 goal.
