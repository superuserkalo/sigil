/**
 * Retention.
 *
 * Diagnostics are kept 30 days, crashes 90 and raw product data 13 months, counted from when
 * the row was created. The daily cron trigger calls this after closing product epochs
 * (`aggregates.ts`). Diagnostics and crashes carry no identifier; raw product rows carry only a
 * salted hash of the install id and can also be deleted on request (`docs/privacy.md`).
 */

const DAY_MS = 24 * 60 * 60 * 1000;

export const DIAGNOSTICS_RETENTION_MS = 30 * DAY_MS;
export const CRASH_RETENTION_MS = 90 * DAY_MS;
/** Raw product data (ADR 0017): 13 months, counted as 396 days. */
export const PRODUCT_RETENTION_MS = 396 * DAY_MS;

export type RetentionResult = {
  readonly diagnostics: number;
  readonly crashes: number;
  readonly product: number;
};

export async function runRetention(db: D1Database, now: number): Promise<RetentionResult> {
  const results = await db.batch([
    db
      .prepare("DELETE FROM diagnostics WHERE created_at < ?1")
      .bind(now - DIAGNOSTICS_RETENTION_MS),
    db.prepare("DELETE FROM crashes WHERE created_at < ?1").bind(now - CRASH_RETENTION_MS),
    db.prepare("DELETE FROM product_events WHERE created_at < ?1").bind(now - PRODUCT_RETENTION_MS),
    // Totals whose key players wrote (search text, hardware names) are not kept forever.
    db
      .prepare("DELETE FROM product_totals WHERE kind IN ('search', 'hardware') AND created_at < ?1")
      .bind(now - PRODUCT_RETENTION_MS),
  ]);

  return {
    diagnostics: results[0]?.meta.changes ?? 0,
    crashes: results[1]?.meta.changes ?? 0,
    product: (results[2]?.meta.changes ?? 0) + (results[3]?.meta.changes ?? 0),
    // Stray contributions are not deleted here: a closing that has not finished would lose the
    // rows its totals need. `closeEpochs` deletes them, bounded, and carries on the next day.
  };
}
