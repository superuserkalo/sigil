-- Catalog enrichment (ADR 0018). Public GameBanana and Steam content only: no player data, no
-- install hash, no bucket. Nothing here joins to the product tables.

-- What the enrichment last saw of each public mod page: enough to tell when it changed and to find
-- duplicate candidates in code. Titles and the description excerpt are public page text.
CREATE TABLE catalog_mods (
  mod_id INTEGER PRIMARY KEY,
  mod_updated INTEGER NOT NULL,
  added_at INTEGER NOT NULL DEFAULT 0,
  title TEXT NOT NULL,
  title_norm TEXT NOT NULL,
  category TEXT NOT NULL DEFAULT '',
  subcategory TEXT NOT NULL DEFAULT '',
  submitter_id INTEGER NOT NULL DEFAULT 0,
  desc_excerpt TEXT NOT NULL DEFAULT '',
  desc_sig TEXT NOT NULL DEFAULT '',
  gb_rated INTEGER NOT NULL DEFAULT 0,
  post_count INTEGER NOT NULL DEFAULT 0,
  seen_at INTEGER NOT NULL
);
CREATE INDEX catalog_mods_title ON catalog_mods (title_norm);
CREATE INDEX catalog_mods_desc ON catalog_mods (desc_sig);

-- One row per mod and named judgment. `data` is JSON with each answer's probability (and the
-- subject box as four fractions for the cover judgment). `source` says what the answer was made
-- from (`u:<mod_updated>` or `p:<post count>`); a different source, version or model re-runs it.
CREATE TABLE mod_tags (
  mod_id INTEGER NOT NULL,
  judgment TEXT NOT NULL,
  version INTEGER NOT NULL,
  source TEXT NOT NULL,
  model TEXT NOT NULL,
  data TEXT NOT NULL,
  enriched_at INTEGER NOT NULL,
  PRIMARY KEY (mod_id, judgment)
);
CREATE INDEX mod_tags_enriched ON mod_tags (enriched_at);

-- One row per official patch note, published shape kept with the raw line answers.
CREATE TABLE patch_tags (
  patch_id TEXT PRIMARY KEY,
  version INTEGER NOT NULL,
  model TEXT NOT NULL,
  title TEXT NOT NULL,
  posted_at INTEGER NOT NULL,
  data TEXT NOT NULL,
  enriched_at INTEGER NOT NULL
);

-- Small key-value state: list cursor, cached hero names, running token and dollar totals, the
-- built tag index and its ETag, and a lease so two runs never overlap.
CREATE TABLE enrich_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
