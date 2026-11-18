import { applyD1Migrations, type D1Migration } from "cloudflare:test";
import { env } from "cloudflare:workers";

/**
 * The migrations array is injected by `vitest.config.ts` through a binding that exists only in the
 * test pool, so it is not part of the generated `Env`. The cast names that one test-only field
 * instead of widening the Worker's types.
 */
const testEnv = env as typeof env & { readonly TEST_MIGRATIONS: D1Migration[] };

await applyD1Migrations(env.DB, testEnv.TEST_MIGRATIONS);
