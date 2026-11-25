-- Sigil telemetry, schema 0005: product epochs are weekly (ADR 0017, owner decision 2026-11-25).
--
-- Epoch numbers were 30-day periods (about 700 in late 2026) and are now 7-day periods (about
-- 2970). Rows written under the old numbering must not mix with the new: this removes any
-- contribution or total whose epoch number is below 1500, a number no weekly epoch after 1998 has.
-- The tables were empty when the change was made, so it is a guard and deletes nothing real.
-- Apply after deploying the weekly code, so nothing the old code wrote in between survives.
--
-- `product_totals.weeks` (0004) is no longer written: Rising compares whole weekly epochs now.
-- The column stays, unused and empty.

DELETE FROM product_contrib WHERE epoch < 1500;
DELETE FROM product_totals WHERE epoch < 1500;
