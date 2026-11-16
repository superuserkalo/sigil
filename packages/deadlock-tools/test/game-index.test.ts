import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import {
  defaultGameIndexPath,
  gameVpkPath,
  listHeroes,
  loadGameIndex,
  searchGameIndex,
  topExtensions,
} from "../src/game-index.ts";
import { buildVpk, withTempFixture, type FixtureEntry } from "./vpk-fixture.ts";

const entries: FixtureEntry[] = [
  { path: "models/heroes_wip/billy/billy.vmdl_c", data: Buffer.from("model"), chunk: 0 },
  { path: "materials/models/heroes_wip/billy/billy_body.vmat_c", data: Buffer.from("material") },
  { path: "sounds/heroes/billy/vo_intro.vsnd_c", data: Buffer.from("sound") },
  { path: "models/heroes_wip/punkgoat/punkgoat.vmdl_c", data: Buffer.from("goat") },
  { path: "panorama/images/heroes/billy_icon.vtex_c", data: Buffer.from("icon") },
];

test("builds, caches and reloads an index", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const cachePath = defaultGameIndexPath(join(root, "cache"));

    const first = await loadGameIndex({ dirVpkPath: directoryPath, cachePath });
    assert.equal(first.fromCache, false);
    assert.equal(first.index.entries.length, 5);
    assert.equal(first.index.header.version, 2);
    assert.equal(first.cacheError, undefined);
    assert.ok((await readFile(cachePath, "utf8")).length > 0);

    const second = await loadGameIndex({ dirVpkPath: directoryPath, cachePath });
    assert.equal(second.fromCache, true);
    assert.deepEqual(second.index.entries, first.index.entries);

    const forced = await loadGameIndex({ dirVpkPath: directoryPath, cachePath, force: true });
    assert.equal(forced.fromCache, false);
  });
});

test("rebuilds when the cached source changes or the cache is corrupt", async () => {
  await withTempFixture(buildVpk(entries), async (root, directoryPath) => {
    const cachePath = defaultGameIndexPath(join(root, "cache"));
    await loadGameIndex({ dirVpkPath: directoryPath, cachePath });

    await writeFile(cachePath, "{}", "utf8");
    const afterCorrupt = await loadGameIndex({ dirVpkPath: directoryPath, cachePath });
    assert.equal(afterCorrupt.fromCache, false);

    const original = await readFile(directoryPath);
    await writeFile(directoryPath, Buffer.concat([original, Buffer.from([0])]));
    const afterChange = await loadGameIndex({ dirVpkPath: directoryPath, cachePath });
    assert.equal(afterChange.fromCache, false);
  });
});

test("searches by text, hero, type, folder and limit", async () => {
  await withTempFixture(buildVpk(entries), async (_root, directoryPath) => {
    const { index } = await loadGameIndex({ dirVpkPath: directoryPath });

    assert.equal(searchGameIndex(index, { query: "billy" }).total, 4);
    assert.equal(searchGameIndex(index, { hero: "billy" }).total, 3);
    assert.equal(searchGameIndex(index, { hero: "punkgoat" }).total, 1);
    assert.equal(searchGameIndex(index, { extension: "vmdl_c" }).total, 2);
    assert.equal(searchGameIndex(index, { extension: ".vtex_c" }).total, 1);
    assert.equal(searchGameIndex(index, { folder: "models/heroes_wip" }).total, 2);
    assert.equal(searchGameIndex(index, { query: "billy", extension: "vmdl_c" }).total, 1);

    const limited = searchGameIndex(index, { query: "billy", limit: 2 });
    assert.equal(limited.entries.length, 2);
    assert.equal(limited.total, 4);
    assert.equal(limited.truncated, true);
  });
});

test("summarizes heroes and extensions", async () => {
  await withTempFixture(buildVpk(entries), async (_root, directoryPath) => {
    const { index } = await loadGameIndex({ dirVpkPath: directoryPath });

    const heroes = listHeroes(index);
    assert.deepEqual(
      heroes.map((hero) => hero.codename),
      ["billy", "punkgoat"],
    );
    assert.equal(heroes[0]?.entries, 3);

    const extensions = topExtensions(index);
    assert.equal(extensions[0]?.extension, "vmdl_c");
  });
});

test("resolves the citadel VPK path", () => {
  assert.equal(gameVpkPath("/games/Deadlock"), "/games/Deadlock/game/citadel/pak01_dir.vpk");
  assert.equal(defaultGameIndexPath("/cache"), "/cache/citadel-pak01.json");
});

test("indexes duplicate paths once and keeps the last entry", async () => {
  await withTempFixture(
    buildVpk([
      { path: "models/dup.vmdl_c", data: Buffer.from("first") },
      { path: "models/dup.vmdl_c", data: Buffer.from("second") },
    ]),
    async (_root, directoryPath) => {
      const { index } = await loadGameIndex({ dirVpkPath: directoryPath });

      assert.equal(index.entries.length, 1);
      assert.equal(index.duplicates, 1);
      assert.equal(index.entries[0]?.length, 6);
    },
  );
});

