const PATTERNS: Array<{ pattern: RegExp; replace: (match: string, ...groups: string[]) => string }> = [
  {
    pattern: /\bsk-[A-Za-z0-9_-]{16,}\b/g,
    replace: () => "[REDACTED]",
  },
  {
    pattern: /\bgh[pousr]_[A-Za-z0-9]{20,}\b/g,
    replace: () => "[REDACTED]",
  },
  {
    pattern: /\bgithub_pat_[A-Za-z0-9_]{20,}\b/g,
    replace: () => "[REDACTED]",
  },
  {
    pattern: /\bAKIA[0-9A-Z]{16}\b/g,
    replace: () => "[REDACTED]",
  },
  {
    pattern: /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/g,
    replace: () => "[REDACTED]",
  },
  {
    pattern: /\b(Bearer\s+)[A-Za-z0-9._~+/=-]{16,}/gi,
    replace: (_match, prefix) => `${prefix}[REDACTED]`,
  },
  {
    pattern: /\b((?:DAH_AGENT_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY)\s*[=:]\s*)[^\s"',]{8,}/g,
    replace: (_match, prefix) => `${prefix}[REDACTED]`,
  },
  {
    pattern:
      /\b((?:api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password)\s*["']?\s*[:=]\s*["']?)[A-Za-z0-9._~+/=-]{16,}/gi,
    replace: (_match, prefix) => `${prefix}[REDACTED]`,
  },
];

/** Replace key-shaped strings. The host applies this to every outbound value. */
export function redactSecrets(text: string): string {
  let result = text;
  for (const { pattern, replace } of PATTERNS) {
    result = result.replace(pattern, replace);
  }
  return result;
}

/** Deep copy, redacting every string. Plain JSON values only. */
export function redact<T>(value: T): T {
  if (typeof value === "string") return redactSecrets(value) as T;
  if (Array.isArray(value)) return value.map((item) => redact(item)) as T;
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [redactSecrets(key), redact(item)]),
    ) as T;
  }
  return value;
}
