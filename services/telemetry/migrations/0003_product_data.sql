-- Sigil telemetry, schema 0003: opt-in product data, "Help build Sigil" (ADR 0017).
--
-- `product_events` holds one row per accepted record. `install_hash` is a salted SHA-256 of the
-- app's random product install id (src/product.ts `installHash`); the raw id, an IP address and
-- a user agent are never stored. `payload` is the record re-built from the Worker's own schema,
-- so it holds only allowlisted fields. Rows are deleted after 13 months by the daily cron and at
-- once by POST /v1/product/delete.
--
-- `product_pairs` holds one row per install and co-installed mod pair, from `enabled_set`
-- records, so GET /v1/product/related can count distinct installs per pair and answer only at
-- 10 or more. It is raw data too: same 13 months, same delete.

CREATE TABLE IF NOT EXISTS product_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at INTEGER NOT NULL,
  install_hash TEXT NOT NULL,
  type TEXT NOT NULL,
  version TEXT NOT NULL,
  os TEXT NOT NULL,
  os_build TEXT NOT NULL DEFAULT '',
  game_build INTEGER,
  payload TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS product_events_install ON product_events (install_hash);
CREATE INDEX IF NOT EXISTS product_events_created_at ON product_events (created_at);
CREATE INDEX IF NOT EXISTS product_events_type ON product_events (type, created_at);

CREATE TABLE IF NOT EXISTS product_pairs (
  mod_a INTEGER NOT NULL,
  mod_b INTEGER NOT NULL,
  install_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (mod_a, mod_b, install_hash)
);

CREATE INDEX IF NOT EXISTS product_pairs_b ON product_pairs (mod_b);
CREATE INDEX IF NOT EXISTS product_pairs_install ON product_pairs (install_hash);
CREATE INDEX IF NOT EXISTS product_pairs_updated_at ON product_pairs (updated_at);
