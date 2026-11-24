import path from "node:path";
import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

/**
 * Tests run inside the Workers runtime with the real bindings from `wrangler.jsonc`: Analytics
 * Engine, D1 and the Rate Limiting namespace. Migrations are read here, in Node, and handed to the
 * worker through a test-only binding that `test/apply-migrations.ts` applies before each test file.
 */
export default defineConfig(async () => {
  const migrations = await readD1Migrations(path.join(import.meta.dirname, "migrations"));

  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.jsonc" },
        miniflare: {
          // Test-only product secrets: the real ones are Worker secrets, never in the repo.
          bindings: {
            TEST_MIGRATIONS: migrations,
            PRODUCT_SALT: "test-only-product-salt-0123456789abcdef",
            PRODUCT_BUCKET_KEY: "test-only-bucket-key-0123456789abcdef",
          },
        },
      }),
    ],
    test: {
      setupFiles: ["./test/apply-migrations.ts"],
    },
  };
});
