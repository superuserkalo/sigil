/**
 * The daily cron job, kept out of the entry module so tests can call it with a failing step.
 */

import { closeEpochs } from "./aggregates";
import { runRetention } from "./retention";
import { scrub } from "./scrub";

/**
 * The daily job: retention first, then closing product epochs. Each step has its own try, so an
 * error or a timeout in one never skips the other.
 */
export async function scheduledJob(
  env: Env,
  now: number,
  cron: string,
  close: (db: D1Database, now: number) => Promise<number[]> = closeEpochs,
): Promise<void> {
  try {
    const result = await runRetention(env.DB, now);
    logLine({ event: "retention", cron, diagnostics: result.diagnostics, crashes: result.crashes, product: result.product });
  } catch (error) {
    logLine({ event: "retention_error", cron, message: describeError(error) });
  }
  try {
    const closed = await close(env.DB, now);
    logLine({ event: "epochs_closed", cron, epochs: closed.length });
  } catch (error) {
    logLine({ event: "epochs_error", cron, message: describeError(error) });
  }
}

function describeError(error: unknown): string {
  if (error instanceof Error) return scrub(`${error.name}: ${error.message}`).slice(0, 300);
  return "unknown error";
}

/** Structured JSON, one line per record. */
function logLine(fields: Record<string, string | number>): void {
  console.log(JSON.stringify(fields));
}
