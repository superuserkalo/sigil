-- Sigil telemetry, schema 0001.
--
-- Analytics Engine holds usage events; it needs no table here. D1 holds the three records that
-- outlive a request: deduplicated crash reports, player-sent diagnostics and the mod hash index.
--
-- No column stores an IP address, a user agent, a Steam id or a raw install id. Timestamps are
-- milliseconds since the Unix epoch as integers, so comparisons in the cron job are exact.

CREATE TABLE IF NOT EXISTS crashes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at INTEGER NOT NULL,
  version TEXT NOT NULL,
  os TEXT NOT NULL,
  message TEXT NOT NULL,
  backtrace TEXT NOT NULL,
  fingerprint TEXT NOT NULL UNIQUE,
  repeats INTEGER NOT NULL DEFAULT 1,
  last_seen_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS crashes_created_at ON crashes (created_at);

CREATE TABLE IF NOT EXISTS diagnostics (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at INTEGER NOT NULL,
  code TEXT NOT NULL UNIQUE,
  version TEXT NOT NULL,
  os TEXT NOT NULL,
  summary TEXT NOT NULL,
  log TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS diagnostics_created_at ON diagnostics (created_at);

CREATE TABLE IF NOT EXISTS mod_hashes (
  sha256 TEXT NOT NULL,
  size INTEGER NOT NULL,
  gamebanana_mod_id INTEGER NOT NULL,
  file_id INTEGER NOT NULL,
  reports INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (sha256, size, gamebanana_mod_id, file_id)
);

CREATE INDEX IF NOT EXISTS mod_hashes_reports ON mod_hashes (sha256, size, reports DESC);

-- One row per install that contributed a mapping, keyed by a salted SHA-256 of the install id,
-- so `mod_hashes.reports` counts distinct installs and one client cannot inflate it.
CREATE TABLE IF NOT EXISTS hash_contributors (
  sha256 TEXT NOT NULL,
  size INTEGER NOT NULL,
  gamebanana_mod_id INTEGER NOT NULL,
  file_id INTEGER NOT NULL,
  contributor TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (sha256, size, gamebanana_mod_id, file_id, contributor)
);
