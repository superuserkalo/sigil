/**
 * The one place this Worker talks to TypeSafe (Jev, `POST /v1/systemone`).
 *
 * HARD RULE: only public catalog content may be sent: GameBanana mod titles, categories,
 * descriptions, file names, sizes and notes, and search terms a generator proposed from those.
 * Never anything a player sent: no install hash, no bucket, no search text, nothing from the
 * product, crash, diagnostics or hash tables. `assertPublicOnly` enforces this by shape: the body
 * may hold only `model`, `state` and `questions`, and every object key anywhere in the state must be
 * on `STATE_KEYS`. A body that fails the check is never sent. Tests pin both lists.
 *
 * The key is the Worker secret `TYPESAFE_API_KEY`, read from `env` only. Without it nothing is sent.
 */

import { Budget, BudgetSpent, RemoteError, type Net } from "./net";

export const TYPESAFE_URL = "https://api.typesafe.ai/v1/systemone";
/** Pinned: thresholds were set against this version. A new version re-runs every judgment. */
export const JEV_MODEL = "jev-1.14.0";
/** Input price per token in US dollars ($0.042 per million). Output is free. */
export const USD_PER_TOKEN = 0.042 / 1_000_000;

/** Top-level fields a request body may carry. */
export const BODY_KEYS: ReadonlySet<string> = new Set(["model", "state", "questions"]);

/** Every object key allowed anywhere inside `state`. All are public catalog fields. */
export const STATE_KEYS: ReadonlySet<string> = new Set(["title", "category", "subcategory", "description", "files", "name", "size_mb", "note", "terms"]);

export type Question =
  | { readonly type: "noul"; readonly instructions: unknown; readonly criteria?: { readonly true: unknown; readonly false: unknown } }
  | { readonly type: "choice"; readonly instructions: unknown; readonly criteria: Readonly<Record<string, unknown>> }
  | { readonly type: "score"; readonly instructions: unknown; readonly criteria: readonly unknown[] };

export type SystemOneBody = {
  readonly model: string;
  readonly state: unknown;
  readonly questions: Readonly<Record<string, Question>>;
};

export type Answer =
  | { readonly type: "noul"; readonly noul: number }
  | { readonly type: "choice"; readonly choice: string; readonly confidence: number; readonly probabilities: Record<string, number> }
  | { readonly type: "score"; readonly score: number; readonly confidence: number; readonly probabilities: Record<string, number> };

export type SystemOneResult = {
  readonly model: string;
  readonly answers: Readonly<Record<string, Answer>>;
  readonly inputTokens: number;
};

/** Throws when the body has a field outside the allowlists. */
export function assertPublicOnly(body: SystemOneBody, stateKeys: ReadonlySet<string> = STATE_KEYS): void {
  for (const key of Object.keys(body)) {
    if (!BODY_KEYS.has(key)) throw new Error(`typesafe body field not allowed: ${key}`);
  }
  const walk = (value: unknown, path: string): void => {
    if (Array.isArray(value)) {
      value.forEach((item, index) => walk(item, `${path}[${index}]`));
      return;
    }
    if (typeof value === "object" && value !== null) {
      for (const [key, inner] of Object.entries(value)) {
        if (!stateKeys.has(key)) throw new Error(`typesafe state field not allowed: ${path}.${key}`);
        walk(inner, `${path}.${key}`);
      }
    }
  };
  walk(body.state, "state");
}

/** The key, when the owner set it. Fail closed: no key, no call. */
export function typesafeKey(env: Env): string | null {
  const key = (env as Env & { readonly TYPESAFE_API_KEY?: string }).TYPESAFE_API_KEY;
  return typeof key === "string" && key.trim().length >= 16 ? key.trim() : null;
}

/** A request TypeSafe rejected for its content (400, 413, 422): retried without images, then skipped. */
export class RejectedRequest extends Error {
  override readonly name = "RejectedRequest";
}

export async function askJev(
  net: Net,
  budget: Budget,
  key: string,
  body: SystemOneBody,
  stateKeys: ReadonlySet<string> = STATE_KEYS,
): Promise<SystemOneResult> {
  // Public catalog content only: see the rule at the top of this file. Checked on every call.
  assertPublicOnly(body, stateKeys);
  if (!budget.take()) throw new BudgetSpent("subrequest budget spent");
  const headers: Record<string, string> = {
    authorization: `Bearer ${key}`,
    "content-type": "application/json",
  };
  const response = await net.fetch(TYPESAFE_URL, { method: "POST", headers, body: JSON.stringify(body) });
  if (response.status === 400 || response.status === 413 || response.status === 422) {
    await response.body?.cancel();
    throw new RejectedRequest(`typesafe answered ${response.status}`);
  }
  if (!response.ok) {
    await response.body?.cancel();
    throw new RemoteError("typesafe", response.status);
  }
  const parsed = (await response.json()) as { model?: unknown; answers?: unknown; usage?: { input_tokens?: unknown } };
  if (typeof parsed.answers !== "object" || parsed.answers === null) throw new RemoteError("typesafe", 502);
  return {
    model: typeof parsed.model === "string" ? parsed.model : body.model,
    answers: parsed.answers as Record<string, Answer>,
    inputTokens: typeof parsed.usage?.input_tokens === "number" ? parsed.usage.input_tokens : 0,
  };
}
