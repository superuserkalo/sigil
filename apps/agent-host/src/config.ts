import { homedir } from "node:os";
import { resolve, join } from "node:path";

export type ProviderKind = "openai" | "anthropic" | "chat";

export interface HostConfig {
  provider: ProviderKind;
  model: string;
  baseUrl: string | undefined;
  apiKey: string | undefined;
  dataDir: string;
  sessionDir: string;
  workspaceDir: string;
  dahRsBin: string | undefined;
}

export class ConfigError extends Error {
  readonly code = "config_error";

  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

/** Matches rust/crates/dah-rs/src/paths.rs so the CLI and the host share one data directory. */
function appDataLeaf(): string {
  return process.platform === "linux" ? "deadlock-agent-harness" : "DeadlockAgentHarness";
}

function platformDataDir(): string {
  const env = process.env;
  const home = homedir();

  if (process.platform === "darwin") {
    return join(home, "Library", "Application Support");
  }

  if (process.platform === "win32") {
    const roaming = nonEmpty(env.APPDATA);
    return roaming ?? join(home, "AppData", "Roaming");
  }

  return nonEmpty(env.XDG_DATA_HOME) ?? join(home, ".local", "share");
}

function nonEmpty(value: string | undefined): string | undefined {
  if (value === undefined) return undefined;
  const trimmed = value.trim();
  return trimmed.length === 0 ? undefined : trimmed;
}

function expandHome(value: string): string {
  if (value === "~") return homedir();
  if (value.startsWith("~/")) return join(homedir(), value.slice(2));
  return value;
}

export function dataDir(env: NodeJS.ProcessEnv = process.env): string {
  const configured = nonEmpty(env.DAH_DATA_DIR);
  if (configured !== undefined) return resolve(expandHome(configured));
  return join(platformDataDir(), appDataLeaf());
}

export function sessionDir(env: NodeJS.ProcessEnv = process.env): string {
  return join(dataDir(env), "agent-sessions");
}

export function workspaceDir(env: NodeJS.ProcessEnv = process.env): string {
  const configured = nonEmpty(env.DAH_WORKSPACE_DIR);
  if (configured !== undefined) return resolve(expandHome(configured));
  return join(dataDir(env), "workspace");
}

function providerOf(value: string | undefined): ProviderKind {
  const provider = nonEmpty(value) ?? "openai";
  if (provider === "openai" || provider === "anthropic" || provider === "chat") return provider;
  throw new ConfigError(
    `DAH_AGENT_PROVIDER must be openai, anthropic or chat; received "${provider}".`,
  );
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): HostConfig {
  const provider = providerOf(env.DAH_AGENT_PROVIDER);
  const model = nonEmpty(env.DAH_AGENT_MODEL);

  if (model === undefined) {
    throw new ConfigError(
      "DAH_AGENT_MODEL is required. Set it to the exact model id for the configured provider. The host does not choose a default model.",
    );
  }

  const nativeKey = provider === "anthropic" ? env.ANTHROPIC_API_KEY : env.OPENAI_API_KEY;
  const apiKey = nonEmpty(env.DAH_AGENT_API_KEY) ?? nonEmpty(nativeKey);

  if (apiKey === undefined && provider !== "chat") {
    const fallback = provider === "anthropic" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY";
    throw new ConfigError(
      `No API key for the ${provider} provider. Set DAH_AGENT_API_KEY or ${fallback} in the host environment.`,
    );
  }

  return {
    provider,
    model,
    baseUrl: nonEmpty(env.DAH_AGENT_BASE_URL),
    apiKey,
    dataDir: dataDir(env),
    sessionDir: sessionDir(env),
    workspaceDir: workspaceDir(env),
    dahRsBin: nonEmpty(env.DAH_RS_BIN),
  };
}
