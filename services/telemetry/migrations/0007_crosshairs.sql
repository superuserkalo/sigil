-- Sigil telemetry, schema 0007: the community crosshair library (ADR 0019, owner approved
-- 2026-11-25). Players publish a crosshair (a name and its settings) from the app; others browse
-- and apply. No accounts.
--
-- `crosshairs` holds only the name, the canonical config (src/crosshairs/config.ts), its SHA-256,
--   counts, flags, the created time and day, and a SHA-256 of the delete token (NULL for Sigil's
--   own rows, which no token deletes). No install id, player id or address.
-- `crosshair_uses`: one row per daily network bucket and crosshair, so an apply counts once per
--   network per day. `crosshair_daily`: one row per counted publish, report or use, for the
--   per-network daily caps and the overall daily publish cap; it does not say which crosshair. The
--   daily bucket is an HMAC under PRODUCT_BUCKET_KEY of the UTC day and the caller's /24 or /48.
--   The daily job deletes both tables' rows 2 days after they were written (src/retention.ts).
-- `crosshair_reports`: one row per reporting network and crosshair. The marker is an HMAC under
--   PRODUCT_BUCKET_KEY of the crosshair id and the network prefix, without a day, so a network
--   counts once per crosshair however many days it reports, and a marker cannot be joined across
--   crosshairs or to any other table. Kept 90 days, then deleted. No address is stored.

CREATE TABLE IF NOT EXISTS crosshairs (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  config TEXT NOT NULL,
  config_hash TEXT NOT NULL UNIQUE,
  uses INTEGER NOT NULL DEFAULT 0,
  reports INTEGER NOT NULL DEFAULT 0,
  official INTEGER NOT NULL DEFAULT 0,
  hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  created_day TEXT NOT NULL,
  delete_token_hash TEXT
);

CREATE INDEX IF NOT EXISTS crosshairs_used ON crosshairs (hidden, uses DESC, id);
CREATE INDEX IF NOT EXISTS crosshairs_new ON crosshairs (hidden, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS crosshairs_flagged ON crosshairs (reports) WHERE reports > 0 OR hidden = 1;

CREATE TABLE IF NOT EXISTS crosshair_uses (
  bucket TEXT NOT NULL,
  crosshair_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (bucket, crosshair_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS crosshair_uses_created ON crosshair_uses (created_at);
CREATE INDEX IF NOT EXISTS crosshair_uses_crosshair ON crosshair_uses (crosshair_id);

CREATE TABLE IF NOT EXISTS crosshair_reports (
  marker TEXT PRIMARY KEY,
  crosshair_id TEXT NOT NULL,
  created_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS crosshair_reports_created ON crosshair_reports (created_at);
CREATE INDEX IF NOT EXISTS crosshair_reports_crosshair ON crosshair_reports (crosshair_id);

CREATE TABLE IF NOT EXISTS crosshair_daily (
  bucket TEXT NOT NULL,
  kind TEXT NOT NULL,
  day TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS crosshair_daily_bucket ON crosshair_daily (bucket, kind);
CREATE INDEX IF NOT EXISTS crosshair_daily_day ON crosshair_daily (kind, day);
CREATE INDEX IF NOT EXISTS crosshair_daily_created ON crosshair_daily (created_at);

-- Sigil's own crosshairs, shown as by Sigil: the built-in presets in crosshair.rs and clearly
-- distinct variations. test/crosshairs.spec.ts checks every row against the Worker's validator.
INSERT INTO crosshairs (id, name, config, config_hash, uses, reports, official, hidden, created_at, created_day, delete_token_hash) VALUES
  ('sigil-default', 'Sigil Default', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":0,"colorG":255,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '0bc1c911f59101f341b85a687af1e1699b3553bedb78bc9a5b508f485c401f60', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-game-default', 'Game Default', '{"pipGap":4,"pipGapStatic":false,"pipHeight":16,"pipWidth":2,"pipOpacity":0.5,"pipOutlineBorder":1,"pipOutlineGap":1,"pipOutlineOpacity":0.7,"dotOpacity":0.7,"dotSize":4,"dotOutlineBorder":2,"dotOutlineGap":2,"dotOutlineOpacity":0.7,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '1f37086e2eb955e48e69544f4d5dd5936eaa8528b99e6a9103d6beba1db58d24', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-dot', 'Dot', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":0,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":1,"dotSize":4,"dotOutlineBorder":1,"dotOutlineGap":0,"dotOutlineOpacity":1,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '5ae9a77abc9f3e2ccb717267ca1313422460138e4c055765578cbf2c4acc8b5d', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-small-cross', 'Small Cross', '{"pipGap":2,"pipGapStatic":true,"pipHeight":6,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '77b2e1ea800cbc099ea46f2deef1789afd80e19335f825310d2c49aa11df4f18', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-cross-dot', 'Cross and Dot', '{"pipGap":4,"pipGapStatic":true,"pipHeight":8,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":1,"dotSize":2,"dotOutlineBorder":1,"dotOutlineGap":0,"dotOutlineOpacity":1,"colorR":0,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'a408094be6c3082232baf023d2f8425e3a2d0501225e5c9331210272c0d378c6', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-wide-gap', 'Wide Gap', '{"pipGap":14,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":190,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '0929eab52f7074760e7715c52b9953ac417cd03e10483e68143d2e8ec9c1a7a5', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-thin', 'Thin', '{"pipGap":4,"pipGapStatic":true,"pipHeight":12,"pipWidth":1,"pipOpacity":1,"pipOutlineBorder":0,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":60,"colorB":220,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '8820f530d2553b2490bba8525979c62c72628528f6c2230e825bc64651ba5855', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-bold', 'Bold', '{"pipGap":6,"pipGapStatic":true,"pipHeight":12,"pipWidth":4,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":60,"colorB":60,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '3c3d74573572361fcd32417050164d9eaba9b88ffbf9578182a00694f0bc5c5e', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-red-dot', 'Red Dot', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":0,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":1,"dotSize":4,"dotOutlineBorder":1,"dotOutlineGap":0,"dotOutlineOpacity":1,"colorR":255,"colorG":40,"colorB":40,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'bb43b6f4505a39fc9577d2eb953331da644efc12d47833966cfce702491ad962', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-micro-dot', 'Micro Dot', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":0,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":1,"dotSize":2,"dotOutlineBorder":1,"dotOutlineGap":0,"dotOutlineOpacity":1,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '24e3f5e881869416398b9d49ec8e84b47cd973cb870069bac19495da090977a8', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-big-dot', 'Big Dot No Outline', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":0,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":1,"dotSize":6,"dotOutlineBorder":0,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":0,"colorG":255,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '7b7940420b81db58543ce4eb5b2e62767da4f5b250c44879bcaec66ceae4966e', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-boxed-dot', 'Boxed Dot', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":0,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":1,"dotSize":2,"dotOutlineBorder":1,"dotOutlineGap":8,"dotOutlineOpacity":1,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":255,"outlineColorG":255,"outlineColorB":255}', 'ab8bf2b3ec4c8dd5a72f4421759a2de1e628e40241db0ec246eb82ae559a2266', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-wide-cross', 'Wide Cross', '{"pipGap":10,"pipGapStatic":true,"pipHeight":16,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '328af403a21889a11902b5e0117adf2e01f1332c702380111c1bcdae15f9492c', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-tight-plus', 'Tight Plus', '{"pipGap":0,"pipGapStatic":true,"pipHeight":8,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'd55e3a02d267517b0fe3d700f7052c56179104f3af3e7ff4c2b27e406e0fd3d3', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-thick', 'Thick', '{"pipGap":6,"pipGapStatic":true,"pipHeight":14,"pipWidth":6,"pipOpacity":1,"pipOutlineBorder":2,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '504aea6bc13f5dcba4eabf99f6ec9aafb870bce3fa6a56e44cbcb7563a240c4e', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-hairline', 'Hairline', '{"pipGap":3,"pipGapStatic":true,"pipHeight":8,"pipWidth":1,"pipOpacity":0.9,"pipOutlineBorder":0,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '7b41506a14704488a117021d28c682271afac0f0d8525a39503a22505e32752c', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-clean-green', 'Clean Green', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":0,"pipOutlineGap":0,"pipOutlineOpacity":0,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":0,"colorG":255,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '23f62afc44cd7f468c5fa04b883356ee16ecc11a0dbe281b6a4ea073ecc5123d', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-heavy-outline', 'Heavy Outline', '{"pipGap":5,"pipGapStatic":true,"pipHeight":10,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":2,"pipOutlineGap":1,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":255,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', '89b87a4e3b727f833dd0b0864e452560e770907c47e482712c418f5d4ebf9b87', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-dynamic', 'Dynamic Spread', '{"pipGap":4,"pipGapStatic":false,"pipHeight":10,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":100,"colorG":200,"colorB":255,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'e8b8f3372e4a28d09d30822e607a8ec0f4d3929202339693218006b47e60ba90', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-soft-pink', 'Soft Pink', '{"pipGap":4,"pipGapStatic":true,"pipHeight":8,"pipWidth":2,"pipOpacity":0.8,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":0.6,"dotOpacity":0.8,"dotSize":2,"dotOutlineBorder":1,"dotOutlineGap":0,"dotOutlineOpacity":0.6,"colorR":255,"colorG":130,"colorB":200,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'b053655b6485cf2fc442967254466432a2ce620805d9bd176212c5a6babe16f5', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL),
  ('sigil-long-lines', 'Long Lines', '{"pipGap":8,"pipGapStatic":true,"pipHeight":24,"pipWidth":2,"pipOpacity":1,"pipOutlineBorder":1,"pipOutlineGap":0,"pipOutlineOpacity":1,"dotOpacity":0,"dotSize":8,"dotOutlineBorder":2,"dotOutlineGap":0,"dotOutlineOpacity":0,"colorR":255,"colorG":140,"colorB":0,"outlineColorR":0,"outlineColorG":0,"outlineColorB":0}', 'dc82aa8f7f2b7bd582fcb986466bd37eb167a3354200366f158b6717aed8fa0c', 0, 0, 1, 0, 1795564800000, '2026-11-25', NULL);
