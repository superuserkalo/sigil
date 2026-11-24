-- Sigil telemetry, schema 0003: opt-in product data, "Help build Sigil" (ADR 0017).
--
-- No table stores an IP address, a user agent or a raw install id.
--
-- `product_events` holds one row per accepted record. `install_hash` is a salted SHA-256 of the
-- app's random product install id (src/product.ts `installHash`). `payload` is the record
-- re-built from the Worker's own schema, so it holds only allowlisted fields.
--
-- The other tables feed aggregates shown to players and carry the distinct-contributor rule
-- (src/aggregates.ts): `bucket` is 16 hex characters of HMAC-SHA256 of the caller's network
-- prefix (IPv4 /24, IPv6 /48) under the Worker secret PRODUCT_BUCKET_KEY. The address and the
-- prefix are never stored. The bucket is kept on aggregate rows only, never on `product_events`.
--
-- `product_installs`: when an install was first and last seen, on how many days, and whether it
--   may count toward aggregates (`eligible`: its bucket was under the daily cap for new installs).
-- `product_buckets`: installs per bucket over the whole window, for the cap of 3 that count.
-- `product_pairs`: one row per install and co-installed mod pair (from `enabled_set`).
-- `product_keys`: one row per install and aggregate key: mod health, hardware, benchmark scene,
--   search term.
--
-- Raw rows (events, installs, buckets, pairs, keys) go after 13 months in the daily cron and at once on
-- POST /v1/product/delete.

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

CREATE TABLE IF NOT EXISTS product_installs (
  install_hash TEXT PRIMARY KEY,
  first_day TEXT NOT NULL,
  last_day TEXT NOT NULL,
  days_seen INTEGER NOT NULL DEFAULT 1,
  eligible INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS product_installs_updated_at ON product_installs (updated_at);

CREATE TABLE IF NOT EXISTS product_buckets (
  bucket TEXT PRIMARY KEY,
  installs INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS product_pairs (
  mod_a INTEGER NOT NULL,
  mod_b INTEGER NOT NULL,
  install_hash TEXT NOT NULL,
  bucket TEXT NOT NULL,
  day TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (mod_a, mod_b, install_hash)
);

CREATE INDEX IF NOT EXISTS product_pairs_b ON product_pairs (mod_b);
CREATE INDEX IF NOT EXISTS product_pairs_install ON product_pairs (install_hash);
CREATE INDEX IF NOT EXISTS product_pairs_updated_at ON product_pairs (updated_at);

CREATE TABLE IF NOT EXISTS product_keys (
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  install_hash TEXT NOT NULL,
  bucket TEXT NOT NULL,
  day TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (kind, key, install_hash)
);

CREATE INDEX IF NOT EXISTS product_keys_install ON product_keys (install_hash);
CREATE INDEX IF NOT EXISTS product_keys_updated_at ON product_keys (updated_at);
