# 0018 Local catalog search index, with optional term expansion checked by TypeSafe Jev

Status: accepted (Worker deployed 2026-11-25 with migration 0006 applied; the desktop app downloads
the index and matches Browse search locally, `deadlock_core::catalog_index`). Term expansion is on
under the $1 daily cap as its live check; file labels are off.
Date: 2026-11-25, narrowed the same day after a product review; the curated alias list removed the
same day (index version 3); likes, content rating and added time, alias checks, publish-time term
cleanup and a judging-aware backfill cursor added the same day after a search review (still
version 3, migration 0008)

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
  `stringId * 8 + class` (0 description word, 1 category, 2 author, 3 hero, 4 verified term,
  5 verified generic visual term).
- Per mod facts from the same list pages (no extra request): `l` GameBanana's like count
  (integer), `n: 1` when the page has content ratings (omitted otherwise), `a` the added time in
  seconds. Added as new keys under version 3: the shipped app accepts only 3 and ignores keys it
  does not know. Stored in `catalog_mods` (migration 0008: `likes`, `rated`, `facts_at`;
  `added_at` existed). Likes are kept out of the content hash, so a likes-only change never asks
  a model; it counts as an index change, which is still published at most hourly.
- Each mod's `u` is its newest change in ms: its text (page time), its facts (`facts_at`, also
  set when the Worker first sees the mod) or its verified terms (`enriched_at`). `?since=` reads
  `u`, so a delta carries likes, newly seen mods and new terms, not only text edits.
- There is no alias, nickname or synonym list of any kind, hand-made or generated. A curated
  table (hero nicknames, mod-kind and style synonyms, German words) shipped briefly and was
  removed on 2026-11-25: it had to be kept up by hand, went stale when heroes were added, and
  added no hits in the recall eval (23 of 40 with and without it). The index carries `v`,
  `built`, `strings` and `mods` only; dropping the table bumped `PUBLISH_VERSION` to 3.
- `INDEX_BUILD` (in `publish.ts`, stored in the index meta, never sent) is bumped whenever what
  the index publishes changes. The stored index was only rebuilt on a version change or a mod
  change, so a publish-logic change could wait; a new build number rebuilds on the next run. This
  is why the live index showed no class 5: it was built at 10:20 UTC, before class 5 was added
  (10:34 UTC), and nothing forced a rebuild. Build 2 carries class 5, the cleanup below and the
  facts. A change seen inside the hourly window is remembered (`index_dirty`) and published by the
  first run after it.
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

The Rust port (`rust/crates/deadlock-core/src/catalog_index/matcher.rs`) runs the reference's
cases as tests and adds the owner's adjustments after looking at live data (2026-11-25):

- Generated terms (class 4, and class 5 below) shorter than 3 characters are ignored ("hd").
- Class 4 terms (reference, character, franchise, alias and theme) count fully at 2.5. Class 5 is
  reserved for the generic visual terms ("model", "gun", "physics") and counts 0.5, less than a
  description word. The Worker packs visual terms as 5 (index build 2; a test pins it). A visual
  term that is also a description word keeps class 0. Classes 6 and 7 are ignored.
- Title matches always outrank other matches: results sort by how many query words the title
  matches, then by score, then by likes when the card is known, then by id.
- Accent folding covers Latin-1 and Latin Extended-A letters and full-width ASCII (the app has
  no Unicode normalisation table); the index strings arrive already normalised by the Worker.

### In the app (built 2026-11-25)

- Download: `GET /v1/catalog/tags` with `If-None-Match`, at start when the saved copy is older
  than three hours and at most every three hours after (15 minutes after a network failure),
  through the proxy and never in offline mode. Full downloads only: a `?since=` delta cannot
  say which mods left the index, and the whole index is about 300 KB on the wire.
- The body is untrusted: at most 8 MB on the wire (by its length header) and 40 MB decoded,
  lists capped while they parse (mods, strings, fields per mod), strings at most 512 bytes,
  version 3 only. A body that fails is dropped and the saved file stays; a corrupt saved file
  reads as no index. Field ids outside the string table are dropped, as the reference skips them.
- Browse: with an index, text typed in the Mods section is matched on every keystroke. Cards
  come from the saved catalog copy, earlier reads, or one `Mod/Multi` request by id for up to
  50 missing matches once typing pauses (ids only). Results keep match order and every grid
  filter. GameBanana's title search still answers with no index, in Sounds, WiPs and the mixed
  feed, with the Featured filter (a row read by id does not carry it), and offline or after a
  failed card read when no match has a known card.

### Upkeep (kept)

- Every 10 minutes (and as the last, separately wrapped step of the daily job) a run reads
  GameBanana's newest updates, newest uploads and one backfill page (50 a page), and reads the
  public text only of mods whose page timestamp moved, 50 per `Mod/Multi` request. A content hash
  per mod (title, categories, author, text, files) tells real changes from page bumps. Withdrawn,
  trashed, withheld or private pages, and mods unseen for 14 days, leave the index. The index is
  rebuilt at most hourly. None of this calls a model.
- The backfill cursor moves to the next page once every mod on it has its text and, while a model
  step runs, none of them still waits for its judgments (not reached because a run cap stopped
  the loop, or failed this run). Before, the cursor moved once the texts were read, so with 20
  mods judged a run most of each 50-mod page waited a whole lap (about 8.7 hours). A 50-mod page
  now takes three runs. A mod whose judging fails (generator error, a rejected request, a bad
  answer) counts a failure on its content hash (`judge_failures`); at 3 its missing judgments are
  written as judged-as-failed for that hash (`{ na: 1, failed: 1 }`, nothing published) and the
  cursor moves on. A new content hash or judgment version tries again. The daily spend cap does
  not hold the cursor: the free text upkeep keeps walking.
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
- Aliases are checked on their own (terms judgment version 5): one more Noul per alias in the same
  request, "is `terms[i]` a name players would commonly type for `terms[j]`", by path, within the
  same caps. An alias is published only when its reference passes (0.7) and its own check passes
  (0.7). Before, an alias inherited its reference's answer, which let "cc2", "tpc" and "nm"
  through. Re-checking is targeted: a version 4 row with no alias stays current
  (`Judgment.accepts`), a version 4 row with an alias is stale and is asked again when its mod is
  next on a listed page (the backfill reaches every mod within a lap). Until then its aliases are
  not published (no own check stored); its other terms are.
- Publish-time cleanup (`publish.ts`): a generated term equal to any Deadlock hero name is dropped,
  with the hero list taken from the data (every distinct Skins subcategory in `catalog_mods`), not
  only the mod's own hero ("paige" and "mina" on a Hololive icon pack). Generated terms shorter
  than 3 characters are dropped (and not asked about). Thresholds and filters live here, so a
  change needs no new inference, only an `INDEX_BUILD` bump.
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
Measured on the live index of 2026-11-25 (2,606 mods): 833 KB, 323 KB gzipped before the facts;
with `l`, `n`, `a` and the hero and short-term cleanup about 889 KB, 344 KB gzipped (+6%).

## Rejected alternatives

- Sending the player's query to a server, TypeSafe or a model for natural-language search: player
  text would leave the PC for a third party.
- A curated franchise list: covers only what we thought of.
- A curated alias table (hero nicknames, kind and style synonyms, German words): built, then
  removed (2026-11-25). Hand upkeep, stale on new heroes, no measured gain.
- Generated hero nicknames (the generator proposes, Jev checks, per hero from the data):
  considered as the self-maintaining replacement and not built, by the owner's call. Prefix and
  one-edit matching already cover most short forms.
- Removed after review (2026-11-25), not built: a model-made NSFW hint (GameBanana's own
  content-rating flag is published as `n` instead), cover and banner picks with a
  subject box, terms from the cover picture, a model guess at the hero (the subcategory lookup
  stays), facts such as "requires another mod" and "work in progress", update-note classes,
  comment health, duplicate detection, patch-note line classes, a learning loop from search gaps,
  document embeddings, and kind and style tags. None had evidence that it earned its cost.

## Consequences

- Cloudflare Workers AI and TypeSafe become processors of public catalog content only when the
  owner turns a flag on (privacy page).
- The app downloads one public index by default and searches it on the PC, offline too for the
  cards it has seen. It does not call `/v1/catalog/mod/<id>`.
