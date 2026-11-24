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
/**
 * The hard limit for `product_contrib`, the only table that links a network bucket to an install:
 * an epoch (30 days), its grace (2 days) and a backlog for a closing that keeps failing. Rows
 * older than this go whether or not their epoch was closed; their totals are lost, the link is not
 * kept.
 */
export const CONTRIB_BACKSTOP_MS = 75 * DAY_MS;

export type RetentionResult = {
  readonly diagnostics: number;
  readonly crashes: number;
  readonly product: number;
  /** Contribution rows the backstop deleted. Above zero means closing fell behind. */
  readonly backstop: number;
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
    // The backstop: never depends on closing.
    db.prepare("DELETE FROM product_contrib WHERE created_at < ?1").bind(now - CONTRIB_BACKSTOP_MS),
  ]);

  return {
    diagnostics: results[0]?.meta.changes ?? 0,
    crashes: results[1]?.meta.changes ?? 0,
    product: (results[2]?.meta.changes ?? 0) + (results[3]?.meta.changes ?? 0),
    backstop: results[4]?.meta.changes ?? 0,
  };
}
