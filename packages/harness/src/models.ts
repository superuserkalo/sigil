import {
  createModels,
  type CredentialStore,
  type MutableModels,
} from "@earendil-works/pi-ai";
import { anthropicProvider } from "@earendil-works/pi-ai/providers/anthropic";
import { deepseekProvider } from "@earendil-works/pi-ai/providers/deepseek";
import { googleProvider } from "@earendil-works/pi-ai/providers/google";
import { openaiProvider } from "@earendil-works/pi-ai/providers/openai";
import { openrouterProvider } from "@earendil-works/pi-ai/providers/openrouter";
import { xaiProvider } from "@earendil-works/pi-ai/providers/xai";
import type { ModelRef } from "@earendil-works/pi-durable";

export const PROVIDER_FACTORIES = {
  anthropic: anthropicProvider,
  openai: openaiProvider,
  google: googleProvider,
  openrouter: openrouterProvider,
  xai: xaiProvider,
  deepseek: deepseekProvider,
} as const;

export type ByokProvider = keyof typeof PROVIDER_FACTORIES;

export const PROVIDER_ORDER = Object.keys(PROVIDER_FACTORIES) as ByokProvider[];

export type ProviderStatus = {
  provider: ByokProvider;
  configured: boolean;
  models: readonly string[];
  source?: string;
  error?: string;
};

export function isByokProvider(value: string): value is ByokProvider {
  return Object.hasOwn(PROVIDER_FACTORIES, value);
}

export function createModelCatalog(credentials?: CredentialStore): MutableModels {
  const models = createModels(credentials === undefined ? {} : { credentials });

  for (const provider of PROVIDER_ORDER) {
    models.setProvider(PROVIDER_FACTORIES[provider]());
  }

  return models;
}

export async function saveApiKey(
  credentials: CredentialStore,
  provider: ByokProvider,
  key: string,
): Promise<void> {
  const trimmed = key.trim();

  if (trimmed.length === 0) throw new Error("API key is empty");

  await credentials.modify(provider, async () => ({ type: "api_key", key: trimmed }));
}

export async function firstAvailableModel(
  models: MutableModels,
): Promise<ModelRef | undefined> {
  for (const provider of PROVIDER_ORDER) {
    let available;

    try {
      available = await models.getAvailable(provider);
    } catch {
      continue;
    }

    const model = available[0];
    if (model !== undefined) return { provider, modelId: model.id };
  }

  return undefined;
}

export async function providerStatus(models: MutableModels): Promise<ProviderStatus[]> {
  const statuses: ProviderStatus[] = [];

  for (const provider of PROVIDER_ORDER) {
    let configured = false;
    let source: string | undefined;
    let error: string | undefined;
    let available: readonly { id: string }[] = [];

    try {
      const auth = await models.checkAuth(provider);
      configured = auth !== undefined;
      source = auth?.source;
      available = await models.getAvailable(provider);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }

    statuses.push({
      provider,
      configured,
      models: available.slice(0, 12).map((model) => model.id),
      ...(source === undefined ? {} : { source }),
      ...(error === undefined ? {} : { error }),
    });
  }

  return statuses;
}
