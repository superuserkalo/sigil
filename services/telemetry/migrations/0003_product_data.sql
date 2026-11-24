-- Sigil telemetry, schema 0003: opt-in product data, "Help build Sigil" (ADR 0017).
--
-- No table stores an IP address, a network prefix, a user agent or a raw install id.
--
-- `product_events`: one row per accepted record. `install_hash` is a salted SHA-256 of the app's
--   random product install id (src/product.ts `installHash`); `payload` is the record re-built
--   from the Worker's own schema. Kept while the install keeps sharing; deleted at once by POST
--   /v1/product/delete. After 24 months with nothing new from an install, the daily job replaces
--   its `install_hash` with a fresh random value (`u:` prefix) so the history can no longer be
--   tied to that install id.
--
-- `product_contrib`: open-epoch contributions (src/aggregates.ts). The only table where a network
--   bucket sits next to an install hash. `bucket` is 16 hex characters of HMAC-SHA256 under the
--   Worker secret PRODUCT_BUCKET_KEY of the 30-day epoch number and the caller's network prefix
--   (IPv4 /24, IPv6 /48), so it changes every epoch. Rows are deleted when their epoch closes (the
--   daily cron, two days after the epoch ends) and at once by POST /v1/product/delete.
--
-- `product_totals`: per closed epoch and aggregate key that met the floor in that epoch (10
--   eligible installs, 6 buckets, 3 days), counts only (installs, buckets, days). No install hash
--   and no bucket; keys below the floor leave no row. Kept indefinitely. Delete my data
--   cannot reach them because nothing in them points to an install.

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

CREATE TABLE IF NOT EXISTS product_contrib (
  epoch INTEGER NOT NULL,
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  install_hash TEXT NOT NULL,
  bucket TEXT NOT NULL,
  day TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (epoch, kind, key, install_hash)
);

CREATE INDEX IF NOT EXISTS product_contrib_bucket ON product_contrib (epoch, bucket);
CREATE INDEX IF NOT EXISTS product_contrib_install ON product_contrib (install_hash);

CREATE TABLE IF NOT EXISTS product_totals (
  epoch INTEGER NOT NULL,
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  mod_a INTEGER,
  mod_b INTEGER,
  installs INTEGER NOT NULL,
  buckets INTEGER NOT NULL,
  days INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (epoch, kind, key)
);

CREATE INDEX IF NOT EXISTS product_totals_kind ON product_totals (kind, key);
CREATE INDEX IF NOT EXISTS product_totals_pair_a ON product_totals (kind, mod_a);
CREATE INDEX IF NOT EXISTS product_totals_pair_b ON product_totals (kind, mod_b);
