# 0018 Catalog tags with TypeSafe Jev, and local search over an expanded index

Status: accepted (Worker built and tested; migration 0006 not applied, not deployed; the app does
not fetch the index yet)
Date: 2026-11-25

## Context

GameBanana's Deadlock catalog has about 2,500 mods and grows. Its own metadata is thin: a title
(often useless, "my first mod", "v2 final"), a category, a free-text description, screenshots and
files with names. Players search for what a mod is ("counter strike", "gas mask", "gojo"), not
for its title, and Browse cannot tell a recolour from a model swap, the main download from an old
version, a working mod from one that broke with the last patch, or which screenshot makes a good
banner.

TypeSafe's Jev returns typed judgments (Choice, Noul yes/no probability, Score, Box on images),
many questions per request against one shared state, priced per input token ($0.042 per million),
output free. The owner approved (2026-11-25) using Jev as a building block wherever it improves
the experience, on public content only, with no player text leaving the PC for search.

## Decision

### Who calls what

- The Worker (`services/telemetry/src/catalog/`), not the app, calls TypeSafe
  (`https://api.typesafe.ai/v1/systemone`) and Cloudflare Workers AI (the `AI` binding). The
  TypeSafe key is the Worker secret `TYPESAFE_API_KEY`, read only from `env`. The app never calls
  TypeSafe or Workers AI and never ships the key.
- Fail closed: without the key nothing is fetched or sent; without the `AI` binding the
  search-term step is skipped and every other judgment runs. The routes always answer with what
  is stored.

### The hard rule: public content only

Only public catalog content goes to TypeSafe or Workers AI: mod titles, descriptions, category
names, file names, sizes and notes, update notes, public screenshots, public comment text (each
commenter becomes `c1`, `c2`; no name or member id is sent) and the official Steam patch notes.
Never anything from the product, crash, diagnostics or hash tables, no install hash, no network
marker, no search text below the product floor. Enforced in code at the one call site
(`typesafe.ts`): the body may hold only `model`, `state`, `questions`, `assets`, and every object
key anywhere in the state must be on a fixed allowlist; a body that fails is never sent. Tests pin
both lists and check that a player's search text stored in `product_events` never appears.

GameBanana text is untrusted. It goes only into the `state` as data: questions point at it by
path (`` `links[0].label` ``, `` `terms[3]` ``) and never quote it. HTML is cut to 20 KB and
scanned linearly; entity decoding never throws. The Workers AI system prompt says the user
message is page data and that instructions inside it are ignored. One broken page is recorded as
failed and the run goes on.

The one exception is the learning loop below, which reads search terms that already passed the
product floor (ADR 0017).

### Small named judgments, one request per mod

Each judgment has a name and its own version; bumping one re-runs only that one. Code decides which
apply, builds every stale judgment's questions over one shared state, and sends one request per
mod (the docs' speculative fan-out). Answers are stored with their probabilities; thresholds live
in `publish.ts`, so a threshold change needs no new inference.

| Judgment | Questions (primitive) | Published when |
| --- | --- | --- |
| `kind` | Choice over 11 kinds (recolour, model swap, skin, HUD, sound, music, animation, effects, map, crosshair, quality of life) plus a companion Noul "does one of these kinds fit" | fits >= 0.7 and chosen p >= 0.6; fits < 0.3 publishes `other` |
| `style` | 10 Nouls: anime, realistic, cute, horror, meme, festive, crossover, minimal, colourful, dark | each >= 0.75 |
| `hero` | Choice over the hero list read at run time from GameBanana's Skins categories, plus "no specific hero" and "several heroes"; the subcategory hero, when there is one, wins (exact lookup in code) | p >= 0.7 |
| `nsfw` | Noul from text and pictures | >= 0.8, only ever as a hint next to GameBanana's own flag; never clears it |
| `cover` | Per screenshot (up to 4): Score "clear as a small card", Score "works as a 21:9 banner", Noul "large baked-in text", Box around the main subject | best card index; banner index when its score >= 0.5 and baked-in text < 0.7, with the box |
| `facts` | Nouls: requires another mod, has variants, replaces a hero model, includes sounds, changes how abilities look, special install steps; Choice which linked mod is required; Choice install kind (manual files, extra tool, incompatible named mod); Choice state (finished, work in progress, abandoned) | Nouls >= 0.7; Choices p >= 0.6 (state 0.7) |
| `files` | For 2+ files: Choice main file; per file a Choice label (main, variant, optional add-on, old version, source files) | p >= 0.6 |
| `updates` | Per update (latest 3): Choice kind (fix for a game update, new content, bug fix, cosmetic) and Noul "says it restores compatibility with the latest patch" | p >= 0.6, Noul >= 0.7 |
| `health` | With at least 3 distinct commenters in the last 120 days (counted in code): Nouls "several players report it broken since an update", "players report it works after the latest patch", "a player reports a conflict with a named mod" | >= 0.7; counts and the time window are stored, never names or text |
| `dupes` | For candidates found in code (same normalised title, or the same long description by another author): Noul "same mod uploaded again" | >= 0.85 |
| `terms` | One Noul per proposed search term (below) | per class, below |
| patch lines | Per official patch note line: Choice system (hero model or animation, abilities or items, UI, sounds, map, other) and Noul "changes how something looks or sounds"; the hero is an exact lookup in code | p >= 0.6, Noul >= 0.7 |

The thresholds start from the docs (a Noul at 0.7 reads as yes; a Choice acts on the chosen
option's probability) and are raised where a wrong visible tag costs more than a missing one
(styles, NSFW hint, duplicates). `.scratch/typesafe/eval.md` holds the hand-checked sample. Jev's
model is pinned (`jev-1.14.0`); a new version re-runs every judgment.

### Search: document expansion checked by Jev, matched locally

A curated franchise list only covers what we thought of. Instead:

1. Generate. Workers AI (`@cf/meta/llama-4-scout-17b-16e-instruct`, natively multimodal, JSON
   schema output, $0.27 per million input and $0.85 per million output tokens) proposes typed
   facets twice per mod: once from the public text, once from the cover picture. Facets:
   franchises or sources with the ways players type them (Counter-Strike 1.6 gives
   `counter-strike`, `cs`, `cs 1.6`, `counter strike`, `gign`, `counter-terrorist`), characters
   with nicknames, what is shown in plain words (gas mask, maid outfit, neon), and themes. Lower
   case, at most 40 characters a term, at most 24 terms per source.
2. Verify. Jev checks each proposed term with a Noul against the title, description and
   screenshots (the docs' cascade pattern); other names of a reference share its answer. This is
   what stops invented references. Kept: references >= 0.7, visual and theme terms >= 0.75.
3. Index. `GET /v1/catalog/tags` serves one shared string table, the alias table (hero nicknames,
   tag synonyms, a few German words: the only curated list), and per mod the title, term ids packed
   as `stringId * 8 + class`, typed facets (`f` franchises, `ch` characters), and the published
   tags. Classes say where a term came from, so the matcher can weight them: 0 reference from the
   text, 1 tag (hero, kind, style, fact), 2 visual, 3 theme, 4 reference from the picture, 5 learned.
4. Match locally. The app downloads the index and matches typed text on the PC
   (`services/telemetry/src/catalog/match.ts` is the reference the Rust app ports):
   - Normalise (lower case, accents and punctuation dropped), split into words; stop words (the,
     a, of, mod, and a few German ones) are scored but not required.
   - Each word is a unit with alternatives from the alias table; three- and two-word phrases are
     looked up first (a phrase's words each get the canonical phrase as an alternative).
   - A unit matches a token exactly (1.0), as a prefix of at least 3 letters (0.8), or one edit away
     (insert, delete, substitute or swap neighbours) for words of 4 letters or more (0.7); a unit
     also matches a multi-word term written without spaces (`counterstrike`).
   - Field weights: title 3, text reference 2.5, tag 2, picture reference 2, learned 2, visual 1.5,
     theme 1, each times idf = ln(1 + (N - df + 0.5) / (df + 0.5)) over the mods carrying the term.
     A multi-word term hit by only some of its words, by a unit that is not the whole term, counts
     0.6.
   - Per unit, the mod scores its best field. Exact title equality adds 1000; every unit found in
     the title adds 30. Ties break by mod id.
   - Every non-stop unit must match somewhere; when no mod passes, any match counts instead.

No player text leaves the PC for search. The query never goes to the Worker, TypeSafe or Workers
AI.

### Learning loop

Once a day the Worker reads the search-gap aggregate (terms with no result or no click that passed
the product floor: 10 installs, 6 networks, 3 days in one closed week; scrubbed on the PC and on
the Worker). A term is used only when it has no personal-data shape (5 or more digits in a row,
`@`, a link, over 40 characters or 4 words) and every word already exists in the public catalog
vocabulary (public titles, verified facets and aliases). Up to three terms a day go to Jev with up
to eight candidate mods from the local matcher; a Score per candidate judges relevance, and a term
scoring >= 0.67 becomes a learned association for that mod. Learned associations stay out of the
public index until the owner sets `LEARNED_TERMS_PUBLISH = "1"`. ADR 0017 records that
floor-passed search aggregates may feed these associations.

### Upkeep

- Every 10 minutes (and as the last, separately wrapped step of the daily job) a run reads the
  newest updates, the newest uploads and one backfill page of GameBanana's list; it reads a page
  only when its timestamp moved, and calls models only when the content hash (title, categories,
  text, files, screenshot URLs) changed or a judgment's version did. Each judgment row carries the
  hash it was made on and is written in one batch with the mod's judged hash at the end, so a run
  that fails midway is retried and its old tags are served marked stale (`st: 1`). Comment health
  refreshes at most every 3 days.
- Withdrawn, trashed, withheld or private pages, and mods unseen in the list for 14 days, leave
  the index.
- Caps (config): at most 45 outside calls a run (under the free plan's 50), `ENRICH_RUN_MODS`
  (default 6) mods a run, `ENRICH_DAILY_USD` (default 25) a day, and a run stops before it would
  pass 25 dollars. Every run logs mods sent, unchanged, failed and withdrawn, tokens and dollars,
  and the running totals. A lease stops two runs overlapping.
- The index is rebuilt at most hourly, gzipped and stored in chunks in D1, served with an ETag and
  `Cache-Control: public, max-age=10800`; `?since=<ms>` returns only mods enriched after that time
  with their own string table.

### What is stored

`catalog_mods` (what the run last saw of each public page, content and judged hash),
`mod_tags` (per mod and judgment: version, source, model, the answers with probabilities, the
cover box as four fractions, the hash it was made on), `patch_tags` (per patch line classes),
`enrich_state` (cursor, cached hero names, spend tallies, the built index, the lease). No player
data. Optional `INDEX_EMBEDDINGS = "1"` adds per-mod int8 embeddings (Workers AI
`bge-small-en-v1.5`, 384 dimensions reduced to 256 by a fixed seeded projection).

### Cost

Workers AI, measured on the eval sample: see `.scratch/typesafe/eval.md` (about $0.0006 a mod for
both generator calls). Jev, estimated before a live run (the key is only a Worker secret): about
4,000 text tokens and four screenshots per first enrichment, roughly 10,000 input tokens or
$0.0004 a mod; the full catalog is about $1 for Jev and $1.5 for Workers AI, and daily upkeep for
the few dozen mods that change is cents. Every run logs the real token count; the eval doc says
how to measure it.

## Rejected alternatives

- Sending the player's query to a server (the Worker, TypeSafe or a model) for natural-language
  search: player text would leave the PC for a third party. Search is made smart at index time
  instead.
- A curated franchise and alias list: covers only what we thought of. Kept only as a small
  supplement (hero nicknames, tag synonyms, German words).
- One big "classify this mod" prompt: no calibrated probabilities, no per-judgment versions, and
  every change re-runs everything.
- A shipped embedding model for concept queries: possible later addition (the optional embeddings
  field is a start); it would need the same model on the PC to embed the query.

## Consequences

- TypeSafe and Cloudflare Workers AI become processors of public catalog content (privacy page).
- The app can download one public index by default (owner approved 2026-11-25) and gets better
  Browse and search with no player data involved. It does not call the routes yet.
- Images and Box questions are "dreamer-ready" in TypeSafe's docs: request formats and accuracy
  may change. The cover judgment is the most likely to need a version bump.
