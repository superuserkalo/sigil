-- Catalog search index, more per-mod facts and bounded judging (ADR 0018). Public GameBanana
-- content only, read from the same list pages as before (no extra requests). `added_at` already
-- exists (0006) and now feeds the index too.

-- GameBanana's like count. Not part of the content hash: a likes-only change never re-runs a model.
ALTER TABLE catalog_mods ADD COLUMN likes INTEGER NOT NULL DEFAULT 0;
-- 1 when GameBanana says the page has content ratings (`_bHasContentRatings`).
ALTER TABLE catalog_mods ADD COLUMN rated INTEGER NOT NULL DEFAULT 0;
-- When likes, the content-rating flag or the added time last changed (ms), so `?since=` deltas
-- carry those changes too.
ALTER TABLE catalog_mods ADD COLUMN facts_at INTEGER NOT NULL DEFAULT 0;
-- Failed judging attempts on the current content hash. Reset when the content changes or a
-- judgment is written; at 3 the mod is marked judged-as-failed for that hash and the backfill
-- cursor moves on.
ALTER TABLE catalog_mods ADD COLUMN judge_failures INTEGER NOT NULL DEFAULT 0;
