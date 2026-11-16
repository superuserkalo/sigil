import { mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { ModelThinkingLevel } from "@earendil-works/pi-ai";
import {
  Harness,
  createRegistry,
  type Conversation,
  type ModelRef,
} from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";
import { CodingTools } from "@earendil-works/pi-durable/tools";
import { deadlockExtension } from "@dah/deadlock-tools";
import { JsonFileCredentialStore } from "./credentials.ts";
import {
  PROVIDER_ORDER,
  createModelCatalog,
  firstAvailableModel,
  isByokProvider,
  type ByokProvider,
} from "./models.ts";

export {
  JsonFileCredentialStore,
} from "./credentials.ts";

export {
  PROVIDER_FACTORIES,
  PROVIDER_ORDER,
  createModelCatalog,
  firstAvailableModel,
  isByokProvider,
  providerStatus,
  saveApiKey,
} from "./models.ts";

export type { ByokProvider, ProviderStatus } from "./models.ts";

export type OpenHarnessOptions = {
  storagePath: string;
  credentialsPath: string;
  workspaceDir: string;
  model?: ModelRef;
  thinkingLevel?: ModelThinkingLevel;
};

export type Runtime = {
  harness: Harness;
  root: Conversation;
  models: ReturnType<typeof createModelCatalog>;
  registry: ReturnType<typeof createRegistry>;
  credentials: JsonFileCredentialStore;
  model: ModelRef;
  close: () => Promise<void>;
};

export async function openHarness(options: OpenHarnessOptions): Promise<Runtime> {
  await mkdir(dirname(options.storagePath), { recursive: true });
  await mkdir(dirname(options.credentialsPath), { recursive: true });
  await mkdir(options.workspaceDir, { recursive: true });

  const credentials = new JsonFileCredentialStore(options.credentialsPath);
  const models = createModelCatalog(credentials);

  const model = options.model ?? (await firstAvailableModel(models));

  if (model === undefined) {
    throw new Error(
      "No model provider is configured. Run `pnpm dah auth set <provider>` or export a provider key.",
    );
  }

  const registry = createRegistry();
  registry.install(CodingTools);
  registry.install(deadlockExtension);

  const storage = await openNodeSqliteStorage(options.storagePath);
  const harness = await Harness.open(
    storage,
    {
      models,
      registry,
      env: (target) => new NodeExecutionEnv({ cwd: target.cwd ?? options.workspaceDir }),
    },
    BACKGROUND_CONTEXT,
  );

  const agent = {
    model,
    cwd: options.workspaceDir,
    ...(options.thinkingLevel === undefined ? {} : { thinkingLevel: options.thinkingLevel }),
  };

  const root = await harness.root(BACKGROUND_CONTEXT, { agent });
  harness.resume();

  return {
    harness,
    root,
    models,
    registry,
    credentials,
    model,
    close: () => harness.close(BACKGROUND_CONTEXT),
  };
}

export function requireProvider(value: string): ByokProvider {
  if (!isByokProvider(value)) {
    throw new Error(`Unknown provider "${value}". Known providers: ${PROVIDER_ORDER.join(", ")}.`);
  }

  return value;
}
