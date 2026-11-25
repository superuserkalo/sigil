/**
 * Retention.
 *
 * Diagnostics are kept 30 days and crashes 90, counted from when the row was created. Product
 * data (ADR 0017, owner decision 2026-11-25) is kept as long as possible:
 *
 * - `product_events` (raw records) are not deleted by age. They stay while the install keeps
 *   sharing and until the player presses Delete my data. When an install has sent nothing for
 *   24 months, `unlinkInactive` replaces its `install_hash` with a fresh random value (one per
 *   install, made here and stored nowhere else): the history stays, but it can no longer be tied
 *   to that install id or extended by it.
 * - `product_totals` (counts above the floor) are kept indefinitely.
 * - `product_contrib`, the only product table with a network marker, keeps its short life: its
 *   epoch, and the 21-day hard backstop below.
 * - `crosshair_uses` and `crosshair_daily` (ADR 0019), the community crosshair rows with a daily
 *   network bucket, are deleted 2 days after they were written; `crosshair_reports`, the
 *   per-crosshair report markers, after 90 days.
 */

import { CROSSHAIR_DEDUPE_MS, CROSSHAIR_REPORT_MS } from "./crosshairs/routes";

const DAY_MS = 24 * 60 * 60 * 1000;

export const DIAGNOSTICS_RETENTION_MS = 30 * DAY_MS;
export const CRASH_RETENTION_MS = 90 * DAY_MS;
/** An install that sent nothing for this long has its raw rows unlinked (24 months). */
export const PRODUCT_INACTIVE_MS = 730 * DAY_MS;
/** Installs unlinked per run, so the job stays short; the rest follow the next day. */
const UNLINK_BUDGET = 50;
/** Unlinked rows carry this prefix, so they are never picked again. */
export const UNLINKED_PREFIX = "u:";
/**
 * The hard limit for `product_contrib`, the only table that links a network bucket to an install:
 * an epoch (7 days), its grace (2 days) and a backlog for a closing that keeps failing. Rows
 * older than this go whether or not their epoch was closed; their totals are lost, the link is not
 * kept.
 */
export const CONTRIB_BACKSTOP_MS = 21 * DAY_MS;

export type RetentionResult = {
  readonly diagnostics: number;
  readonly crashes: number;
  /** Contribution rows the backstop deleted. Above zero means closing fell behind. */
  readonly backstop: number;
};

export async function runRetention(db: D1Database, now: number): Promise<RetentionResult> {
  const results = await db.batch([
    db
      .prepare("DELETE FROM diagnostics WHERE created_at < ?1")
      .bind(now - DIAGNOSTICS_RETENTION_MS),
    db.prepare("DELETE FROM crashes WHERE created_at < ?1").bind(now - CRASH_RETENTION_MS),
    // The backstop: never depends on closing.
    db.prepare("DELETE FROM product_contrib WHERE created_at < ?1").bind(now - CONTRIB_BACKSTOP_MS),
    // Community crosshairs (ADR 0019): daily-bucket rows 2 days, report markers 90 days.
    db.prepare("DELETE FROM crosshair_uses WHERE created_at < ?1").bind(now - CROSSHAIR_DEDUPE_MS),
    db.prepare("DELETE FROM crosshair_daily WHERE created_at < ?1").bind(now - CROSSHAIR_DEDUPE_MS),
    db.prepare("DELETE FROM crosshair_reports WHERE created_at < ?1").bind(now - CROSSHAIR_REPORT_MS),
  ]);

  return {
    diagnostics: results[0]?.meta.changes ?? 0,
    crashes: results[1]?.meta.changes ?? 0,
    backstop: results[2]?.meta.changes ?? 0,
  };
}

/**
 * Unlink installs that sent nothing for 24 months: every `product_events` row of one such install
 * gets the same fresh random value in place of its hash, in one statement, so its history stays
 * whole. At most `UNLINK_BUDGET` installs per run. Returns how many installs were unlinked.
 */
export async function unlinkInactive(db: D1Database, now: number): Promise<number> {
  const { results } = await db
    .prepare(
      `SELECT install_hash FROM product_events
       WHERE install_hash NOT LIKE '${UNLINKED_PREFIX}%'
       GROUP BY install_hash HAVING MAX(created_at) < ?1
       LIMIT ?2`,
    )
    .bind(now - PRODUCT_INACTIVE_MS, UNLINK_BUDGET)
    .all<{ install_hash: string }>();
  for (const { install_hash } of results) {
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    const fresh = UNLINKED_PREFIX + Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
    await db.prepare("UPDATE product_events SET install_hash = ?2 WHERE install_hash = ?1").bind(install_hash, fresh).run();
  }
  return results.length;
}
