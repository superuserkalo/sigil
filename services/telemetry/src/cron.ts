/**
 * The daily cron job, kept out of the entry module so tests can call it with a failing step.
 */

import { closeEpochs, epochOf, GRACE_MS } from "./aggregates";
import { runRetention, unlinkInactive } from "./retention";
import { scrub } from "./scrub";
import { runEnrichment } from "./catalog/enrich";

/**
 * The daily job: retention first, then unlinking inactive installs, then closing product epochs. Each step has its own try, so an
 * error or a timeout in one never skips the other.
 */
export async function scheduledJob(
  env: Env,
  now: number,
  cron: string,
  close: (db: D1Database, now: number) => Promise<number[]> = closeEpochs,
  enrich: (env: Env, now: number) => Promise<unknown> = runEnrichment,
): Promise<void> {
  try {
    const result = await runRetention(env.DB, now);
    logLine({ event: "retention", cron, diagnostics: result.diagnostics, crashes: result.crashes });
    if (result.backstop > 0) {
      logLine({ event: "error", what: "product_contrib_backstop", cron, rows: result.backstop });
    }
  } catch (error) {
    logLine({ event: "retention_error", cron, message: describeError(error) });
  }
  try {
    const unlinked = await unlinkInactive(env.DB, now);
    logLine({ event: "product_unlinked", cron, installs: unlinked });
  } catch (error) {
    logLine({ event: "unlink_error", cron, message: describeError(error) });
  }
  try {
    const closed = await close(env.DB, now);
    logLine({ event: "epochs_closed", cron, epochs: closed.length });
  } catch (error) {
    logLine({ event: "epochs_error", cron, message: describeError(error) });
  }
  // An epoch that should have closed more than a day ago means closing failed two runs running.
  try {
    const overdue = await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib WHERE epoch <= ?1")
      .bind(epochOf(now - GRACE_MS - DAY_MS) - 1)
      .first<{ n: number }>();
    if ((overdue?.n ?? 0) > 0) logLine({ event: "error", what: "product_epochs_overdue", cron, rows: overdue?.n ?? 0 });
  } catch (error) {
    logLine({ event: "epochs_check_error", cron, message: describeError(error) });
  }
  // Catalog enrichment (ADR 0018) last, in its own try: a GameBanana, Workers AI or TypeSafe
  // failure never touches the steps above.
  try {
    await enrich(env, now);
  } catch (error) {
    logLine({ event: "enrich_error", cron, message: describeError(error) });
  }
}

const DAY_MS = 24 * 60 * 60 * 1000;

function describeError(error: unknown): string {
  if (error instanceof Error) return scrub(`${error.name}: ${error.message}`).slice(0, 300);
  return "unknown error";
}

/** Structured JSON, one line per record. */
function logLine(fields: Record<string, string | number>): void {
  console.log(JSON.stringify(fields));
}
