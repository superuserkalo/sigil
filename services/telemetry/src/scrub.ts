/**
 * Server side scrubbing for crash reports and diagnostics.
 *
 * The desktop app scrubs before it sends. This runs again because the app cannot be trusted to be
 * the only client, and because a future client may forget. The rules are ported from Grimoire's
 * `electron/main/services/diagnosticSanitize.ts` (MIT, see `THIRD_PARTY_NOTICES.md`) and extended
 * with the key shapes Sigil's own providers use.
 *
 * Home paths keep their structure and lose only the user name, so a report stays useful. Secrets
 * become opaque placeholders. Order matters: specific patterns run before generic ones, and the
 * generic `token=...` rule runs last so it cannot swallow a value a better rule would name.
 */

const PLACEHOLDERS: ReadonlyArray<{ readonly pattern: RegExp; readonly replacement: string }> = [
  // Home paths: keep the structure, drop the account name.
  { pattern: /\/home\/[^/\s"'`]+/g, replacement: "/home/<user>" },
  { pattern: /\/Users\/[^/\s"'`]+/g, replacement: "/Users/<user>" },
  { pattern: /([A-Za-z]:\\{1,2}Users\\{1,2})[^\\\s"'`]+/gi, replacement: "$1<user>" },
  { pattern: /([A-Za-z]:\\{1,2}home\\{1,2})[^\\\s"'`]+/gi, replacement: "$1<user>" },
  { pattern: /([A-Za-z]:\/Users\/)[^/\s"'`]+/gi, replacement: "$1<user>" },

  // Steam ids and the account id the stats API uses.
  { pattern: /\b7656119\d{10}\b/g, replacement: "<steamid64>" },
  { pattern: /\[U:1:\d+\]|\bU:1:\d+\b/g, replacement: "<steamid3>" },
  { pattern: /\bSTEAM_[0-5]:[01]:\d+\b/g, replacement: "<steamid2>" },
  { pattern: /(account_id=|\/players?\/)(\d{4,12})/g, replacement: "$1<accountid>" },

  // Provider and service keys, longest and most specific shapes first.
  { pattern: /\bsk-ant-[A-Za-z0-9_-]{16,}\b/g, replacement: "<key>" },
  { pattern: /\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}\b/g, replacement: "<key>" },
  { pattern: /\bgithub_pat_[A-Za-z0-9_]{20,}\b/g, replacement: "<key>" },
  { pattern: /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}\b/g, replacement: "<key>" },
  { pattern: /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/g, replacement: "<key>" },
  { pattern: /\bAKIA[0-9A-Z]{16}\b/g, replacement: "<key>" },
  { pattern: /\bAIza[0-9A-Za-z_-]{30,}\b/g, replacement: "<key>" },

  // Auth headers, bearer tokens and JWTs.
  { pattern: /(Authorization:\s*(?:Bearer|Basic)\s+)\S+/gi, replacement: "$1<token>" },
  { pattern: /(Bearer\s+)[A-Za-z0-9._~+/-]{10,}=*/g, replacement: "$1<token>" },
  { pattern: /\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\b/g, replacement: "<jwt>" },

  // A pasted email, then any value glued to a token-like name.
  { pattern: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g, replacement: "<email>" },
  {
    pattern:
      /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|auth[_-]?token|token|secret|password|passwd)\s*[:=]\s*)("[^"]*"|'[^']*'|\S+)/gi,
    replacement: "$1<redacted>",
  },

  // The install id is the one identifier Sigil makes. A report never needs it.
  {
    pattern: /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi,
    replacement: "<uuid>",
  },
];

export function scrub(text: string): string {
  let out = text;
  for (const rule of PLACEHOLDERS) out = out.replace(rule.pattern, rule.replacement);
  return out;
}
