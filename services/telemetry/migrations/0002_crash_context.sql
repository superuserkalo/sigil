-- Sigil telemetry, schema 0002: what kind of crash a report is, and the scrubbed log tail of
-- the session that crashed.
--
-- `kind` is `panic` (the Rust panic hook), `unclean_exit` (the app found its session marker at
-- the next start without a panic: an abort, a stack overflow, a driver crash or a forced quit) or
-- `native` (the Windows exception filter). Rows from before this migration are panics.
-- `context` holds the scrubbed log tail. A repeat of the same fingerprint keeps the newest one.

ALTER TABLE crashes ADD COLUMN kind TEXT NOT NULL DEFAULT 'panic';
ALTER TABLE crashes ADD COLUMN context TEXT NOT NULL DEFAULT '';
