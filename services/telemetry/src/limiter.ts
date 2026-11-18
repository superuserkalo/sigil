/**
 * Rate limiting without storing a caller address.
 *
 * The Rate Limiting binding counts per key. The key here is a SHA-256 of the caller address and a
 * salt that exists only in this isolate and is replaced at each UTC midnight, so a stored count
 * cannot be traced back to an address after the salt is gone. Nothing derived from the address is
 * written to D1 or to a log line.
 *
 * Cloudflare's binding is approximate and per location; that is fine for an opt-in collector whose
 * job is to stay available, not to enforce a quota.
 */

import { sha256Hex, toHex } from "./hash";

type Salt = { readonly day: string; readonly value: string };

let salt: Salt | null = null;

function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}

function dailySalt(now: number): string {
  const day = utcDay(now);
  if (salt !== null && salt.day === day) return salt.value;

  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  const value = toHex(bytes);
  salt = { day, value };
  return value;
}

export async function limiterKey(request: Request, now: number): Promise<string> {
  const address = request.headers.get("cf-connecting-ip") ?? "unknown";
  return sha256Hex(`${dailySalt(now)}\n${address}`);
}

/** True when the request is inside the allowance. */
export async function allowRequest(
  request: Request,
  limiter: RateLimit,
  now: number,
): Promise<boolean> {
  const result = await limiter.limit({ key: await limiterKey(request, now) });
  return result.success;
}
