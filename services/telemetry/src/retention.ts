/**
 * Retention.
 *
 * Diagnostics are kept 30 days and crashes 90, counted from when the row was created. The daily
 * cron trigger calls this. Rows are plain text with no identifiers, so the dates are the whole
 * policy (`docs/privacy.md`).
 */

const DAY_MS = 24 * 60 * 60 * 1000;

export const DIAGNOSTICS_RETENTION_MS = 30 * DAY_MS;
export const CRASH_RETENTION_MS = 90 * DAY_MS;

export type RetentionResult = {
  readonly diagnostics: number;
  readonly crashes: number;
};

export async function runRetention(db: D1Database, now: number): Promise<RetentionResult> {
  const results = await db.batch([
    db
      .prepare("DELETE FROM diagnostics WHERE created_at < ?1")
      .bind(now - DIAGNOSTICS_RETENTION_MS),
    db.prepare("DELETE FROM crashes WHERE created_at < ?1").bind(now - CRASH_RETENTION_MS),
  ]);

  return {
    diagnostics: results[0]?.meta.changes ?? 0,
    crashes: results[1]?.meta.changes ?? 0,
  };
}
