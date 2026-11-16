#!/usr/bin/env node
import { parseArgs } from "node:util";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { AssistantMessage, Message } from "@earendil-works/pi-ai";
import { AssistantEntry, type ModelRef } from "@earendil-works/pi-durable";
import { formatGameStatus, readGameStatus } from "@dah/deadlock-tools";
import {
  createModelCatalog,
  openHarness,
  providerStatus,
  requireProvider,
  saveApiKey,
  JsonFileCredentialStore,
} from "@dah/harness";
import { resolveCliPaths } from "./config.ts";

const USAGE = `dah: Deadlock Agent Harness

Usage:
  dah doctor [--json]                 game install, gameinfo, slots, providers
  dah models                          provider auth status and available models
  dah auth set <provider>             store a provider API key (mode 600)
  dah auth list                       list stored provider ids and credential types
  dah chat [options] <prompt>         run the agent in the workspace

Options:
  --model <provider/modelId>          choose a model for this run
  --workspace <path>                  use a specific mod workspace
  --json                              machine-readable output where supported
  -h, --help                          show this help

Providers: anthropic, openai, google, openrouter, xai, deepseek

Environment:
  DAH_DATA_DIR, DAH_WORKSPACE_DIR, DAH_API_KEY
`;

async function main(): Promise<void> {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      model: { type: "string" },
      workspace: { type: "string" },
      json: { type: "boolean" },
      help: { type: "boolean", short: "h" },
    },
  });

  const command = positionals[0];

  if (values.help === true || command === undefined) {
    process.stdout.write(USAGE);
    return;
  }

  switch (command) {
    case "doctor":
      await doctor(values.json === true);
      return;
    case "models":
      await models();
      return;
    case "auth":
      await auth(positionals.slice(1));
      return;
    case "chat":
      await chat(positionals.slice(1).join(" ").trim(), values.model, values.workspace);
      return;
    default:
      process.stderr.write(`Unknown command "${command}"\n\n${USAGE}`);
      process.exitCode = 1;
  }
}

async function doctor(json: boolean): Promise<void> {
  const paths = resolveCliPaths();
  const status = await readGameStatus();

  const credentials = new JsonFileCredentialStore(paths.credentialsPath);
  const catalog = createModelCatalog(credentials);
  const providers = await providerStatus(catalog);

  if (json) {
    process.stdout.write(`${JSON.stringify({ game: status, providers, paths }, null, 2)}\n`);
    return;
  }

  process.stdout.write(`${formatGameStatus(status)}\n\n`);

  for (const provider of providers) {
    const state = provider.configured ? "configured" : "not configured";
    const source = provider.source === undefined ? "" : ` (${provider.source})`;
    const error = provider.error === undefined ? "" : ` error: ${provider.error}`;
    process.stdout.write(`${provider.provider}: ${state}${source}${error}\n`);
  }

  process.stdout.write(`\nData: ${paths.dataDir}\nWorkspace: ${paths.workspaceDir}\n`);
}

async function models(): Promise<void> {
  const paths = resolveCliPaths();
  const credentials = new JsonFileCredentialStore(paths.credentialsPath);
  const catalog = createModelCatalog(credentials);
  const providers = await providerStatus(catalog);

  for (const provider of providers) {
    const state = provider.configured ? "configured" : "not configured";
    process.stdout.write(`${provider.provider}: ${state}\n`);

    if (provider.models.length === 0) {
      process.stdout.write("  (no models available)\n");
      continue;
    }

    for (const model of provider.models) process.stdout.write(`  ${model}\n`);
  }
}

async function auth(args: readonly string[]): Promise<void> {
  const action = args[0];
  const paths = resolveCliPaths();
  const credentials = new JsonFileCredentialStore(paths.credentialsPath);

  if (action === "list") {
    const stored = await credentials.list();

    if (stored.length === 0) {
      process.stdout.write("No credentials stored.\n");
      return;
    }

    for (const entry of stored) {
      process.stdout.write(`${entry.providerId}: ${entry.type}\n`);
    }
    return;
  }

  if (action !== "set") {
    throw new Error("Use `dah auth set <provider>` or `dah auth list`");
  }

  const rawProvider = args[1];

  if (rawProvider === undefined) {
    throw new Error("Use `dah auth set <provider>`");
  }

  const provider = requireProvider(rawProvider);
  const key = process.env.DAH_API_KEY ?? (await readSecret(`API key for ${provider}: `));

  if (key.trim().length === 0) throw new Error("No key entered");

  await saveApiKey(credentials, provider, key);
  process.stdout.write(`Stored ${provider} credential in ${paths.credentialsPath}\n`);
}

async function chat(
  prompt: string,
  model: string | undefined,
  workspace: string | undefined,
): Promise<void> {
  if (prompt.length === 0) throw new Error("Pass a prompt, for example: dah chat \"recolor Billy's jacket\"");

  const paths = resolveCliPaths();
  const runtime = await openHarness({
    storagePath: paths.storagePath,
    credentialsPath: paths.credentialsPath,
    workspaceDir: workspace ?? paths.workspaceDir,
    ...(model === undefined ? {} : { model: parseModel(model) }),
  });

  try {
    process.stderr.write(`model: ${runtime.model.provider}/${runtime.model.modelId}\n`);

    const submission = await runtime.root.submit(
      { type: "input", content: prompt },
      BACKGROUND_CONTEXT,
    );
    const settled = await submission.wait(BACKGROUND_CONTEXT);

    if (settled.status !== "done" || settled.type !== "input") {
      const reason = settled.status === "unanswered" ? `: ${settled.reason}` : "";
      process.stderr.write(`Run ended ${settled.status}${reason}\n`);
      process.exitCode = 1;
      return;
    }

    const entry = await runtime.root.commit(
      (transaction) => transaction.entry(AssistantEntry, settled.answer),
      BACKGROUND_CONTEXT,
    );

    process.stdout.write(`${textFromMessage(entry?.model?.[0])}\n`);
  } finally {
    await runtime.close();
  }
}

function parseModel(value: string): ModelRef {
  const separator = value.indexOf("/");

  if (separator <= 0 || separator === value.length - 1) {
    throw new Error("Use provider/modelId, for example anthropic/claude-sonnet-4-5");
  }

  return {
    provider: value.slice(0, separator),
    modelId: value.slice(separator + 1),
  };
}

function textFromMessage(message: Message | undefined): string {
  if (message === undefined || message.role !== "assistant") return "";

  const assistant = message as AssistantMessage;
  return assistant.content
    .flatMap((content) => (content.type === "text" ? [content.text] : []))
    .join("");
}

async function readSecret(prompt: string): Promise<string> {
  if (process.stdin.isTTY !== true) {
    const chunks: Buffer[] = [];
    for await (const chunk of process.stdin) chunks.push(Buffer.from(chunk as Buffer));
    return Buffer.concat(chunks).toString("utf8").trim();
  }

  return new Promise((resolve, reject) => {
    const stdin = process.stdin;
    const wasRaw = stdin.isRaw;
    let value = "";

    const cleanup = (): void => {
      stdin.off("data", onData);
      if (wasRaw !== undefined) stdin.setRawMode(wasRaw);
      stdin.pause();
    };

    const onData = (chunk: Buffer): void => {
      for (const character of chunk.toString("utf8")) {
        if (character === "\r" || character === "\n") {
          cleanup();
          process.stdout.write("\n");
          resolve(value.trim());
          return;
        }

        if (character === "\u0003") {
          cleanup();
          process.stdout.write("\n");
          reject(new Error("Cancelled"));
          return;
        }

        if (character === "\u007f" || character === "\b") {
          value = value.slice(0, -1);
          continue;
        }

        value += character;
      }
    };

    process.stdout.write(prompt);
    stdin.setRawMode(true);
    stdin.resume();
    stdin.on("data", onData);
  });
}

main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
