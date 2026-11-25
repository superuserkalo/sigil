-- Catalog search index (ADR 0018). Public GameBanana content only: no player data, no install
-- hash, no bucket. Nothing here joins to the product tables.

-- What the upkeep job last read of each public mod page: the text the index is built from, a hash
-- of it, and the hash the optional judgments were made on.
CREATE TABLE catalog_mods (
  mod_id INTEGER PRIMARY KEY,
  mod_updated INTEGER NOT NULL,
  added_at INTEGER NOT NULL DEFAULT 0,
  title TEXT NOT NULL,
  category TEXT NOT NULL DEFAULT '',
  subcategory TEXT NOT NULL DEFAULT '',
  author TEXT NOT NULL DEFAULT '',
  -- Public summary and description as plain text, at most 1,500 characters.
  description TEXT NOT NULL DEFAULT '',
  -- Public file names, sizes and notes (JSON), at most 8, for the flagged file labels.
  files TEXT NOT NULL DEFAULT '[]',
  -- SHA-256 of title, categories, author, text and files: a page bump with the same content costs
  -- nothing.
  content_hash TEXT NOT NULL DEFAULT '',
  -- The content hash the stored judgments were made on, written in the same batch as they are.
  judged_hash TEXT NOT NULL DEFAULT '',
  -- The page timestamp the text above was read for.
  text_updated INTEGER NOT NULL DEFAULT 0,
  -- 1 when GameBanana withdrew, trashed or hid the page: dropped from the index.
  withdrawn INTEGER NOT NULL DEFAULT 0,
  seen_at INTEGER NOT NULL
);

-- One row per mod and flagged judgment (`terms`, `files`). `data` is JSON with each answer's
-- probability; `hash` is the content hash it was made on. A different version, model or hash
-- re-runs it.
CREATE TABLE mod_tags (
  mod_id INTEGER NOT NULL,
  judgment TEXT NOT NULL,
  version INTEGER NOT NULL,
  model TEXT NOT NULL,
  data TEXT NOT NULL,
  hash TEXT NOT NULL DEFAULT '',
  enriched_at INTEGER NOT NULL,
  PRIMARY KEY (mod_id, judgment)
);

-- Small key-value state: list cursor, the spend tallies, the built index (gzipped, in chunks) and
-- its ETag, and a lease so two runs never overlap.
CREATE TABLE enrich_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
