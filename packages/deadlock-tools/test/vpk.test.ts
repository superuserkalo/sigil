import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import {
  chunkPathFor,
  crc32,
  readVpkDirectory,
  readVpkEntryData,
} from "../src/vpk.ts";
import { buildVpk, withTempFixture, type FixtureEntry } from "./vpk-fixture.ts";

const entries: FixtureEntry[] = [
  {
    path: "models/heroes_wip/billy/billy.vmdl_c",
    data: Buffer.from("model-data"),
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
  { path: "readme.txt", data: Buffer.from("hello") },
  { path: "docs/notes", data: Buffer.from("notes") },
];

test("reads a version 2 directory with directory, chunk and preload data", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const directory = await readVpkDirectory(directoryPath);

    assert.equal(directory.header.version, 2);
    assert.equal(directory.entries.length, 5);
    assert.equal(directory.skipped, 0);
    assert.equal(directory.duplicates, 0);

    const model = directory.byPath.get("models/heroes_wip/billy/billy.vmdl_c");
    assert.ok(model);
    assert.equal(model.preloadBytes, 3);
    assert.equal(model.preload.toString(), "PRE");
    assert.equal(model.length, 10);
    assert.equal(model.totalLength, 13);
    assert.equal(model.archiveIndex, 0);
    assert.deepEqual(await readVpkEntryData(directory, model), Buffer.from("PREmodel-data"));
    assert.equal(crc32(await readVpkEntryData(directory, model)), model.crc32);

    const material = directory.byPath.get("materials/models/heroes_wip/billy/billy_body.vmat_c");
    assert.ok(material);
    assert.equal(material.archiveIndex, 0x7fff);
    assert.equal((await readVpkEntryData(directory, material)).toString(), "material");

    const sound = directory.byPath.get("sounds/heroes/billy/vo_intro.vsnd_c");
    assert.ok(sound);
    assert.equal(sound.archiveIndex, 1);
    assert.equal((await readVpkEntryData(directory, sound)).toString(), "sound");

    assert.equal(directory.byPath.get("readme.txt")?.path, "readme.txt");
    assert.equal(directory.byPath.get("docs/notes")?.path, "docs/notes");

    assert.equal(chunkPathFor(directoryPath, 2), join(root, "pak01_002.vpk"));
  });
});

test("reads a version 1 directory", async () => {
  await withTempFixture(buildVpk(entries, 1), async (_root, directoryPath) => {
    const directory = await readVpkDirectory(directoryPath);

    assert.equal(directory.header.version, 1);
    assert.equal(directory.header.headerSize, 12);
    assert.equal(directory.entries.length, 5);
    assert.equal((await readVpkEntryData(directory, directory.entries[0]!)).toString(), "PREmodel-data");
  });
});

test("skips unsafe paths and counts duplicate paths", async () => {
  const fixture = buildVpk([
    { path: "../escape.vmdl_c", data: Buffer.from("bad") },
    { path: "models/ok.vmdl_c", data: Buffer.from("first") },
    { path: "models/ok.vmdl_c", data: Buffer.from("second") },
  ]);

  await withTempFixture(fixture, async (_root, directoryPath) => {
    const directory = await readVpkDirectory(directoryPath);

    assert.equal(directory.entries.length, 2);
    assert.equal(directory.skipped, 1);
    assert.equal(directory.duplicates, 1);
    assert.equal(
      (await readVpkEntryData(directory, directory.byPath.get("models/ok.vmdl_c")!)).toString(),
      "second",
    );
  });
});

test("rejects a file that is not a VPK", async () => {
  await withTempFixture(buildVpk(entries), async (root) => {
    const path = join(root, "junk.vpk");
    await writeFile(path, "not a vpk");

    await assert.rejects(() => readVpkDirectory(path), /not a VPK|too small|truncated/i);
  });
});

test("rejects a tree that runs past the file", async () => {
  const fixture = buildVpk(entries);
  const corrupted = Buffer.from(fixture.directory);
  corrupted.writeUInt32LE(1024 * 1024, 8);

  await withTempFixture({ directory: corrupted, chunks: fixture.chunks }, async (_root, directoryPath) => {
    await assert.rejects(() => readVpkDirectory(directoryPath), /tree|remain/i);
  });
});

test("reads a real byte range from a chunk file", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const directory = await readVpkDirectory(directoryPath);
    const sound = directory.byPath.get("sounds/heroes/billy/vo_intro.vsnd_c");
    assert.ok(sound);

    const chunk = await readFile(join(root, "pak01_001.vpk"));
    assert.equal(chunk.toString(), "sound");
  });
});
