# 0018 Local catalog search index, with optional term expansion checked by TypeSafe Jev

Status: accepted (Worker deployed 2026-11-25 with migration 0006 applied; the app does not fetch
the index yet). Term expansion is on under the $1 daily cap as its live check; file labels are off.
Date: 2026-11-25, narrowed the same day after a product review; the curated alias list removed the
same day (index version 3)

## Context

GameBanana's Deadlock catalog has about 2,500 mods. Players search for what a mod is ("counter
strike", "gojo", "gas mask"), often with typos, and titles are often useless. Sending what a
player types to a server, or to a model, would put player text in a third party's hands. The owner
approved (2026-11-25) that the app downloads a public search index by default and matches searches
on the PC.

## Decision

### The index (kept)

- The Worker (`services/telemetry/src/catalog/`) builds one public index from GameBanana's own
  public text: per mod the title, up to 60 description words (stop words dropped), the category
  and subcategory, the hero (GameBanana's hero subcategory under Skins, an exact lookup in code),
  and the author. One shared string table; per mod the title and field ids packed as
  `stringId * 8 + class` (0 description word, 1 category, 2 author, 3 hero, 4 verified term).
- There is no alias, nickname or synonym list of any kind, hand-made or generated. A curated
  table (hero nicknames, mod-kind and style synonyms, German words) shipped briefly and was
  removed on 2026-11-25: it had to be kept up by hand, went stale when heroes were added, and
  added no hits in the recall eval (23 of 40 with and without it). The index carries `v`,
  `built`, `strings` and `mods` only; dropping the table bumped `PUBLISH_VERSION` to 3.
- `GET /v1/catalog/tags` serves it gzipped from D1 with an ETag and
  `Cache-Control: public, max-age=10800`; `?since=<ms>` returns only mods whose text changed
  after that time, with their own string table. No consent gate: the request carries no id and
  no product data and the answer is the same for everyone. Rate limited like the other routes.

### Local matching (kept)

The app matches typed text on the PC; nothing typed leaves it. `match.ts` is the reference the
Rust app ports:

1. Normalise (lower case, accents and punctuation dropped) and split into words. Stop words (the,
   a, of, mod, and a few German ones) are scored but not required.
2. Each word is a unit. No word is rewritten into another: misspellings and short forms are
   covered by the tolerance in step 3, outside names by the flagged verified terms.
3. A unit matches a token exactly (1.0), as a prefix of at least 3 letters (0.8), or one edit away
   (insert, delete, substitute or swap neighbours) for words of 4 letters or more (0.7). A unit
   also matches a multi-word field written without spaces (`counterstrike`).
4. Field weights: title 3, verified term 2.5, hero 2, category 1.5, author 1.5, description word 1,
   each times idf = ln(1 + (N - df + 0.5) / (df + 0.5)). A multi-word field hit by only some of
   its words, by a unit that is not the whole field, counts 0.6.
5. Per unit, the mod scores its best field. Exact title equality adds 1000; every unit found in
   the title adds 30. Ties break by mod id.
6. Every non-stop unit must match somewhere; when no mod passes, any match counts instead.

### Upkeep (kept)

- Every 10 minutes (and as the last, separately wrapped step of the daily job) a run reads
  GameBanana's newest updates, newest uploads and one backfill page (50 a page), and reads the
  public text only of mods whose page timestamp moved, 50 per `Mod/Multi` request. A content hash
  per mod (title, categories, author, text, files) tells real changes from page bumps. Withdrawn,
  trashed, withheld or private pages, and mods unseen for 14 days, leave the index. The index is
  rebuilt at most hourly. None of this calls a model.
- At most 45 outside calls a run (under the free plan's 50), a lease so runs never overlap, every
  run logs mods read, unchanged, withdrawn, judged and failed. `CATALOG_INDEX = "1"` (a Worker
  var) turns the upkeep on; anything else is the off switch.

### Under evaluation, off by default

- Term expansion (`CATALOG_EXPANSION = "1"`): Cloudflare Workers AI
  (`@cf/meta/llama-4-scout-17b-16e-instruct`) proposes, from a mod's public title, category and
  description, the outside franchises and characters it references with the ways players type
  them (CS gives counter-strike, cs, cs 1.6, gign), plain visual words and themes. TypeSafe Jev
  (`jev-1.14.0`, pinned) checks each proposed term with a Noul; references are kept at 0.7, visual
  and theme terms at 0.75. The Deadlock hero a skin replaces is removed in code first.
- File labels (`CATALOG_FILE_LABELS = "1"`): for a mod with several files, a Jev Choice for the main
  file and a label per file (main, variant, add-on, old version, source). Served by
  `GET /v1/catalog/mod/<id>` as a pre-selection hint only: the app always shows the file picker.
- Both run only for mods whose content hash changed, never without `TYPESAFE_API_KEY` (a Worker
  secret, never in the app), at most `ENRICH_RUN_MODS` (default 20) mods a run and
  `ENRICH_DAILY_USD` (default $1) a day. Each judgment row carries the content hash it was made
  on and is written in one batch with the mod's judged hash, so a run that fails midway is retried
  and its old terms are marked stale (`st: 1`).
- Evidence so far (`.scratch/typesafe/eval.md`): on 40 hand-made queries against the real catalog,
  top-5 recall is 23 for plain text and 35 with the generator's terms before Jev's check (an
  upper bound); rerun after the alias table was removed, unchanged. The removed alias table had
  scored 23 as well. No live Jev answer exists yet: the key is only a Worker secret.
  The thresholds are the TypeSafe docs' starting points. Expansion is turned on only after a live
  run confirms the verified terms; file labels only after their accuracy is hand-checked.

### The hard rule: public content only

Only public GameBanana content goes to Workers AI or TypeSafe: titles, categories, descriptions,
file names, sizes and notes, and terms the generator proposed from those. Never anything from the
product, crash, diagnostics or hash tables, no install hash, no search text. Enforced at the one
TypeSafe call site (`typesafe.ts`): the body may hold only `model`, `state` and `questions`, and
every object key in the state must be on a fixed allowlist; a body that fails is never sent.
GameBanana text and generated terms sit in the state as data and questions point at them by path,
never quote them. HTML is cut to 20 KB and scanned linearly; entity decoding never throws.

### Cost

Index upkeep: no model cost. With both flags on: Workers AI about $0.00023 a mod (measured), Jev
about 580 input tokens or $0.000024 a mod (estimated from real request sizes), so about $0.65 for
the whole catalog once and about a cent a day after.

### Size

The real catalog's index (2,547 mods) is 756 KB, 292 KB gzipped; scaled to 7,000 mods about
710 KB gzipped. Full downloads with an ETag are fine at this size; deltas exist for growth.

## Rejected alternatives

- Sending the player's query to a server, TypeSafe or a model for natural-language search: player
  text would leave the PC for a third party.
- A curated franchise list: covers only what we thought of.
- A curated alias table (hero nicknames, kind and style synonyms, German words): built, then
  removed (2026-11-25). Hand upkeep, stale on new heroes, no measured gain.
- Generated hero nicknames (the generator proposes, Jev checks, per hero from the data):
  considered as the self-maintaining replacement and not built, by the owner's call. Prefix and
  one-edit matching already cover most short forms.
- Removed after review (2026-11-25), not built: an NSFW hint, cover and banner picks with a
  subject box, terms from the cover picture, a model guess at the hero (the subcategory lookup
  stays), facts such as "requires another mod" and "work in progress", update-note classes,
  comment health, duplicate detection, patch-note line classes, a learning loop from search gaps,
  document embeddings, and kind and style tags. None had evidence that it earned its cost.

## Consequences

- Cloudflare Workers AI and TypeSafe become processors of public catalog content only when the
  owner turns a flag on (privacy page).
- The app can download one public index by default and search offline. It does not call the
  routes yet.
