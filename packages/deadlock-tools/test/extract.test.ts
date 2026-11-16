import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { extractFromVpk } from "../src/extract.ts";
import { buildVpk, withTempFixture, type FixtureEntry } from "./vpk-fixture.ts";

const entries: FixtureEntry[] = [
  {
    path: "models/heroes_wip/billy/billy.vmdl_c",
    data: Buffer.from("model"),
    preload: Buffer.from("PRE"),
    chunk: 0,
  },
  {
    path: "materials/models/heroes_wip/billy/billy_body.vmat_c",
    data: Buffer.from("material"),
  },
  {
    path: "sounds/heroes/billy/vo_intro.vsnd_c",
    data: Buffer.from("sound"),
    chunk: 1,
  },
];

test("extracts directory, chunk and preload data at the original game paths", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const destination = join(root, "workspace", "extracted");
    const result = await extractFromVpk({
      dirVpkPath: directoryPath,
      entryPaths: entries.map((entry) => entry.path),
      destination,
    });

    assert.equal(result.errors.length, 0);
    assert.equal(result.files.length, 3);
    assert.equal(result.totalBytes, 8 + 8 + 5);
    assert.equal(await readFile(join(destination, entries[0]!.path), "utf8"), "PREmodel");
    assert.equal(await readFile(join(destination, entries[1]!.path), "utf8"), "material");
    assert.equal(await readFile(join(destination, entries[2]!.path), "utf8"), "sound");
  });
});

test("rejects traversal, unknown paths and duplicate requests", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const result = await extractFromVpk({
      dirVpkPath: directoryPath,
      entryPaths: ["../pak01_dir.vpk", "missing.vmdl_c", entries[0]!.path, entries[0]!.path],
      destination: join(root, "out"),
    });

    assert.equal(result.files.length, 1);
    assert.equal(result.errors.length, 2);
    assert.ok(result.errors.some((issue) => issue.path === "../pak01_dir.vpk"));
    assert.ok(result.errors.some((issue) => issue.path === "missing.vmdl_c"));
  });
});

test("does not overwrite unless asked", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const destination = join(root, "out");
    const request = {
      dirVpkPath: directoryPath,
      entryPaths: [entries[1]!.path],
      destination,
    };

    await extractFromVpk(request);
    await writeFile(join(destination, entries[1]!.path), "edited");

    const skipped = await extractFromVpk(request);
    assert.equal(skipped.files.length, 0);
    assert.equal(skipped.skipped.length, 1);
    assert.equal(await readFile(join(destination, entries[1]!.path), "utf8"), "edited");

    const overwritten = await extractFromVpk({ ...request, overwrite: true });
    assert.equal(overwritten.files.length, 1);
    assert.equal(await readFile(join(destination, entries[1]!.path), "utf8"), "material");
  });
});

test("reports a CRC mismatch and leaves the file alone", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const chunkPath = join(root, "pak01_000.vpk");
    const chunk = await readFile(chunkPath);
    chunk[0] = (chunk[0]! + 1) & 0xff;
    await writeFile(chunkPath, chunk);

    const result = await extractFromVpk({
      dirVpkPath: directoryPath,
      entryPaths: [entries[0]!.path],
      destination: join(root, "out"),
    });

    assert.equal(result.files.length, 0);
    assert.equal(result.errors.length, 1);
    assert.match(result.errors[0]!.message, /CRC32/);
  });
});

test("refuses more files than one call allows", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const result = await extractFromVpk({
      dirVpkPath: directoryPath,
      entryPaths: entries.map((entry) => entry.path),
      destination: join(root, "out"),
      maxFiles: 1,
    });

    assert.equal(result.files.length, 0);
    assert.match(result.errors[0]!.message, /limited to 1/);
  });
});
