import { describe, expect, it, beforeEach } from "vitest";
import { call, clearTables, readJson, SAMPLE_HASH, VERSION } from "./helpers";

type ErrorBody = { readonly error: string };
type Match = {
  readonly gamebananaModId: number;
  readonly fileId: number;
  readonly reports: number;
};
type IdentifyResponse = { readonly matches: readonly Match[] };
type HashesResponse = { readonly reports: number };

const OTHER_HASH = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809";

function contribution(
  sha256: string,
  gamebananaModId: number,
  fileId: number,
  size = 1024,
  installId: string = crypto.randomUUID(),
): Record<string, unknown> {
  return { installId, sha256, size, gamebananaModId, fileId };
}

function identify(sha256: string, size = 1024): Promise<Response> {
  return call(`/v1/identify?sha256=${sha256}&size=${size}`, { method: "GET" });
}

/** Contribute the same mapping from `installs` different installs. */
async function report(sha256: string, modId: number, fileId: number, installs: number, size = 1024): Promise<void> {
  for (let n = 0; n < installs; n++) {
    await call("/v1/hashes", { body: contribution(sha256, modId, fileId, size) });
  }
}

beforeEach(clearTables);

describe("hash contributions", () => {
  it("answers only once two installs agree", async () => {
    const created = await call("/v1/hashes", { body: contribution(SAMPLE_HASH, 1234, 5678) });
    expect(created.status).toBe(202);
    expect(await readJson<HashesResponse>(created)).toEqual({ reports: 1 });
    expect(await readJson<IdentifyResponse>(await identify(SAMPLE_HASH))).toEqual({ matches: [] });

    await call("/v1/hashes", { body: contribution(SAMPLE_HASH, 1234, 5678) });
    const found = await identify(SAMPLE_HASH);
    expect(found.status).toBe(200);
    expect(await readJson<IdentifyResponse>(found)).toEqual({
      matches: [{ gamebananaModId: 1234, fileId: 5678, reports: 2 }],
    });
  });

  it("counts one install once however often it reports", async () => {
    const install = crypto.randomUUID();
    for (let n = 0; n < 5; n++) {
      const response = await call("/v1/hashes", { body: contribution(OTHER_HASH, 4321, 8765, 1024, install) });
      expect(await readJson<HashesResponse>(response)).toEqual({ reports: 1 });
    }
    expect(await readJson<IdentifyResponse>(await identify(OTHER_HASH))).toEqual({ matches: [] });
  });

  it("needs an install id", async () => {
    const response = await call("/v1/hashes", {
      body: { sha256: SAMPLE_HASH, size: 1024, gamebananaModId: 1, fileId: 1 },
    });
    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_install_id");
  });

  it("returns several mods for one file with the most reported first", async () => {
    await report(SAMPLE_HASH, 10, 1, 2);
    await report(SAMPLE_HASH, 20, 2, 3);
    await report(SAMPLE_HASH, 30, 3, 1);

    expect(await readJson<IdentifyResponse>(await identify(SAMPLE_HASH))).toEqual({
      matches: [
        { gamebananaModId: 20, fileId: 2, reports: 3 },
        { gamebananaModId: 10, fileId: 1, reports: 2 },
      ],
    });
  });

  it("normalizes an uppercase hash", async () => {
    await report(OTHER_HASH.toUpperCase(), 30, 3, 2);

    const found = await identify(OTHER_HASH);
    expect((await readJson<IdentifyResponse>(found)).matches).toHaveLength(1);
  });

  it("returns no matches for a file nobody reported", async () => {
    expect(await readJson<IdentifyResponse>(await identify(OTHER_HASH))).toEqual({ matches: [] });
  });

  it("keeps a different size apart from the same hash", async () => {
    await report(SAMPLE_HASH, 40, 4, 2, 2048);
    expect(await readJson<IdentifyResponse>(await identify(SAMPLE_HASH, 1024))).toEqual({ matches: [] });
    expect((await readJson<IdentifyResponse>(await identify(SAMPLE_HASH, 2048))).matches).toHaveLength(1);
  });
});

describe("hash validation", () => {
  it("rejects a malformed sha256", async () => {
    for (const sha256 of ["abc", SAMPLE_HASH.slice(0, 63), `${SAMPLE_HASH}0`, `g${SAMPLE_HASH.slice(1)}`]) {
      const response = await call("/v1/hashes", { body: contribution(sha256, 1, 1) });
      expect(response.status).toBe(400);
      expect((await readJson<ErrorBody>(response)).error).toBe("bad_sha256");
    }
  });

  it("rejects a size that is not a positive integer", async () => {
    for (const size of [0, -1, 1.5, "1024", 9 * 1024 * 1024 * 1024]) {
      const response = await call("/v1/hashes", {
        body: { installId: crypto.randomUUID(), sha256: SAMPLE_HASH, size, gamebananaModId: 1, fileId: 1 },
      });
      expect(response.status).toBe(400);
      expect((await readJson<ErrorBody>(response)).error).toBe("bad_size");
    }
  });

  it("rejects bad GameBanana ids", async () => {
    const modResponse = await call("/v1/hashes", {
      body: { installId: crypto.randomUUID(), sha256: SAMPLE_HASH, size: 1024, gamebananaModId: 0, fileId: 1 },
    });
    expect(modResponse.status).toBe(400);
    expect((await readJson<ErrorBody>(modResponse)).error).toBe("bad_gamebanana_mod_id");

    const fileResponse = await call("/v1/hashes", {
      body: { installId: crypto.randomUUID(), sha256: SAMPLE_HASH, size: 1024, gamebananaModId: 1, fileId: "12" },
    });
    expect(fileResponse.status).toBe(400);
    expect((await readJson<ErrorBody>(fileResponse)).error).toBe("bad_file_id");
  });

  it("rejects a contribution body over 4 KiB", async () => {
    const response = await call("/v1/hashes", {
      rawBody: JSON.stringify({ ...contribution(SAMPLE_HASH, 1, 1), padding: "x".repeat(5000) }),
    });

    expect(response.status).toBe(413);
  });
});

describe("identify validation", () => {
  it("rejects a missing or malformed hash", async () => {
    expect((await call("/v1/identify?size=1024", { method: "GET" })).status).toBe(400);
    expect((await call("/v1/identify?sha256=nope&size=1024", { method: "GET" })).status).toBe(400);
  });

  it("rejects a size that is not a plain positive integer", async () => {
    for (const size of ["0", "-1", "1e6", "1024.5", ""]) {
      const response = await call(`/v1/identify?sha256=${SAMPLE_HASH}&size=${size}`, { method: "GET" });
      expect(response.status).toBe(400);
      expect((await readJson<ErrorBody>(response)).error).toBe("bad_size");
    }
  });

  it("does not echo the caller's query back", async () => {
    const response = await call(`/v1/identify?sha256=${SAMPLE_HASH}&size=1024`, { method: "GET" });
    const text = await response.text();

    expect(text).not.toContain(SAMPLE_HASH);
    expect(text).not.toContain(VERSION);
  });
});
