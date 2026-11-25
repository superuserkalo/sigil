-- Sigil telemetry, schema 0004: what product data gives back (ADR 0017, "players who use this
-- also use", mod health, frame rates on PCs like yours, search gaps).
--
-- No new table. The same epoch machinery (src/aggregates.ts) gains kinds: `attempt`, `use`,
-- `conflict`, `fps`, `search_zero`, `search_few` and `install_day`, with the same floor and the same
-- distinct-contributor rules as `pair`.
--
-- `product_contrib.value`, `value2`: for kind `fps` only, the install's latest benchmark average and
--   1% low frame rate for that key. Like every contribution row it lives only until its epoch
--   closes (75 days at most) and goes at once with Delete my data.
-- `product_totals.weeks`: for kind `install_day` only, the eligible installs of a mod that met the
--   floor in that epoch, counted by the day of the epoch they installed it on, as five whole numbers
--   for days 0-6, 7-13, 14-15, 16-22 and 23-29 ("3,4,1,6,9"). Counts only.
-- `product_totals.median`, `low`: for kind `fps` only, the median of those values over the eligible
--   installs of a key that met the floor in that epoch. Counts and two medians, no install hash, no
--   bucket and no single install's value.

ALTER TABLE product_contrib ADD COLUMN value INTEGER;
ALTER TABLE product_contrib ADD COLUMN value2 INTEGER;
ALTER TABLE product_totals ADD COLUMN median INTEGER;
ALTER TABLE product_totals ADD COLUMN low INTEGER;
ALTER TABLE product_totals ADD COLUMN weeks TEXT;

CREATE INDEX IF NOT EXISTS product_totals_epoch ON product_totals (kind, epoch);
