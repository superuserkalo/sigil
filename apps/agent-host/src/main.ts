import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { createAgent } from "./agent.ts";
import { ConfigError, loadConfig } from "./config.ts";
import { DahRsCli, defaultDahRsBinary } from "./dah-rs.ts";
import { AgentHost } from "./host.ts";
import { redact, redactSecrets } from "./redact.ts";

const MAX_LINE_CHARS = 1_000_000;

function log(line: string): void {
  process.stderr.write(`[agent-host] ${redactSecrets(line)}\n`);
}

function writeLine(value: unknown): void {
  try {
    const encoded = JSON.stringify(redact(value));
    process.stdout.write(`${encoded ?? "null"}\n`);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`[agent-host] could not write an envelope: ${redactSecrets(message)}\n`);
  }
}

async function main(): Promise<void> {
  let config;
  try {
    config = loadConfig();
  } catch (error) {
    const message = error instanceof ConfigError ? error.message : String(error);
    log(message);
    writeLine({ id: null, ok: false, error: { code: "config_error", message } });
    process.exitCode = 1;
    return;
  }

  try {
    mkdirSync(config.workspaceDir, { recursive: true });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    log(`could not create the workspace ${config.workspaceDir}: ${message}`);
    writeLine({
      id: null,
      ok: false,
      error: {
        code: "workspace_error",
        message: `Could not create the mod workspace ${config.workspaceDir}. Set DAH_WORKSPACE_DIR to a writable folder.`,
      },
    });
    process.exitCode = 1;
    return;
  }

  const cli = new DahRsCli({
    binary: defaultDahRsBinary(),
    cwd: config.workspaceDir,
    env: {
      ...process.env,
      DAH_DATA_DIR: config.dataDir,
      DAH_WORKSPACE_DIR: config.workspaceDir,
    },
  });
  const agent = createAgent(config, cli);

  let stopping = false;
  let finished = false;

  const rl = createInterface({
    input: process.stdin,
    crlfDelay: Infinity,
    terminal: false,
  });

  const host: AgentHost = new AgentHost({
    agent,
    sessionDir: config.sessionDir,
    send: writeLine,
    log,
    onShutdown: () => finish(),
  });

  function finish(): void {
    if (finished) return;
    finished = true;
    stopping = true;
    rl.close();
    process.stdin.pause();
    // Let queued stdout writes flush, then leave even if a stream is still open.
    setTimeout(() => process.exit(0), 250);
  }

  async function cleanup(reason: string): Promise<void> {
    if (finished) return;
    stopping = true;
    log(`stopping: ${reason}`);
    try {
      await host.shutdown();
    } catch (error) {
      log(`shutdown failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    finish();
  }

  async function dispatch(line: string): Promise<void> {
    if (stopping) return;

    if (line.length > MAX_LINE_CHARS) {
      writeLine({
        id: null,
        ok: false,
        error: {
          code: "line_too_large",
          message: `Input line exceeds ${MAX_LINE_CHARS} characters.`,
        },
      });
      return;
    }

    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      writeLine({
        id: null,
        ok: false,
        error: { code: "invalid_json", message: "Input line is not valid JSON." },
      });
      return;
    }

    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      writeLine({
        id: null,
        ok: false,
        error: { code: "invalid_command", message: "A command must be a JSON object." },
      });
      return;
    }

    const command = parsed as Record<string, unknown>;
    await host.handle(command);
  }

  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (trimmed.length === 0) return;
    void dispatch(trimmed);
  });

  rl.on("close", () => {
    void cleanup("stdin closed");
  });

  process.on("SIGINT", () => {
    void cleanup("SIGINT");
  });

  process.on("SIGTERM", () => {
    void cleanup("SIGTERM");
  });

  process.stdout.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "EPIPE") {
      process.exit(0);
    }
    log(`stdout error: ${error.message}`);
  });

  process.on("uncaughtException", (error) => {
    log(`uncaught exception: ${error.message}`);
  });

  process.on("unhandledRejection", (reason) => {
    log(`unhandled rejection: ${reason instanceof Error ? reason.message : String(reason)}`);
  });

  log(
    `TinyAgent host ready: provider=${config.provider} model=${config.model} sessions=${config.sessionDir} workspace=${config.workspaceDir} dah-rs=${cli.binary}`,
  );
}

await main();
