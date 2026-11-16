import { execFile } from "node:child_process";
import type { ExecFileException } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ToolError } from "tiny-agent-sdk";
import { redactSecrets } from "./redact.ts";

const SECRET_ENV = ["DAH_AGENT_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"];
const TIMEOUT_MS = 120_000;
const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;
const EXCERPT_CHARS = 2000;

interface ExecOutcome {
  error: ExecFileException | null;
  stdout: string;
  stderr: string;
}

/** The configured binary, the repo debug build, or PATH. */
export function defaultDahRsBinary(
  env: NodeJS.ProcessEnv = process.env,
  moduleUrl: string = import.meta.url,
): string {
  const configured = env.DAH_RS_BIN?.trim();
  if (configured) return configured;

  const repoRoot = resolve(dirname(fileURLToPath(moduleUrl)), "../../..");
  const debugBinary = join(repoRoot, "rust", "target", "debug", "dah-rs");
  if (existsSync(debugBinary)) return debugBinary;

  // The crate currently names its binary `dah`; accept the build when it exists.
  const packageBinary = join(repoRoot, "rust", "target", "debug", "dah");
  if (existsSync(packageBinary)) return packageBinary;

  return "dah-rs";
}

function excerpt(text: string): string {
  const trimmed = text.trim();
  const redacted = redactSecrets(trimmed);
  return redacted.length <= EXCERPT_CHARS
    ? redacted
    : `${redacted.slice(0, EXCERPT_CHARS)}\n[... excerpt truncated ...]`;
}

function compact(value: unknown): string {
  const encoded = JSON.stringify(value);
  return encoded === undefined ? "null" : encoded;
}

function parseJson(text: string): { parsed: true; value: unknown } | { parsed: false } {
  const trimmed = text.trim();
  if (!trimmed) return { parsed: false };
  try {
    return { parsed: true, value: JSON.parse(trimmed) as unknown };
  } catch {
    return { parsed: false };
  }
}

/**
 * Runs the native CLI with an argument array. Never through a shell string, so
 * a path or query cannot become a second command. The child does not inherit
 * provider credentials.
 */
export class DahRsCli {
  readonly binary: string;
  private readonly cwd: string;
  private readonly env: NodeJS.ProcessEnv;

  constructor(options: { binary: string; cwd: string; env?: NodeJS.ProcessEnv }) {
    this.binary = options.binary;
    this.cwd = options.cwd;
    this.env = { ...(options.env ?? process.env) };
    for (const name of SECRET_ENV) delete this.env[name];
  }

  /** Run the CLI and return its compact JSON. Throws a ToolError the model can read. */
  async json(args: readonly string[], signal?: AbortSignal): Promise<string> {
    const outcome = await this.exec(args, signal);

    if (outcome.error && signal?.aborted) throw outcome.error;

    const parsed = parseJson(outcome.stdout);
    if (parsed.parsed) return compact(parsed.value);

    const label = `dah-rs ${args.join(" ")}`.trim();

    if (outcome.error?.code === "ENOENT") {
      throw new ToolError(
        `Could not run the dah-rs CLI at ${this.binary}. Build it with \`cargo build -p dah-rs\` or set DAH_RS_BIN.`,
      );
    }

    if (outcome.error?.killed || outcome.error?.signal) {
      throw new ToolError(
        `${label} stopped after ${TIMEOUT_MS / 1000} s. Narrow the query or retry.${outcome.stderr.trim() ? `\nstderr:\n${excerpt(outcome.stderr)}` : ""}`,
      );
    }

    if (outcome.error) {
      const status = typeof outcome.error.code === "string" ? ` (${outcome.error.code})` : "";
      throw new ToolError(
        `${label} failed${status}.${outcome.stderr.trim() ? `\nstderr:\n${excerpt(outcome.stderr)}` : ""}${outcome.stdout.trim() ? `\nstdout:\n${excerpt(outcome.stdout)}` : ""}`,
      );
    }

    throw new ToolError(
      `${label} returned no JSON on stdout.${outcome.stderr.trim() ? `\nstderr:\n${excerpt(outcome.stderr)}` : ""}`,
    );
  }

  private exec(args: readonly string[], signal?: AbortSignal): Promise<ExecOutcome> {
    return new Promise((resolvePromise) => {
      execFile(
        this.binary,
        [...args],
        {
          cwd: this.cwd,
          env: this.env,
          timeout: TIMEOUT_MS,
          maxBuffer: MAX_OUTPUT_BYTES,
          windowsHide: true,
          ...(signal === undefined ? {} : { signal }),
        },
        (error, stdout, stderr) => {
          resolvePromise({
            error,
            stdout: String(stdout ?? ""),
            stderr: String(stderr ?? ""),
          });
        },
      );
    });
  }
}
