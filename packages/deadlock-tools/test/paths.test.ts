import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { after, test } from "node:test";
import {
  discoverDeadlockInstall,
  formatSlotFileName,
  parseLibraryFolders,
  readAddonsStatus,
  readGameInfoStatus,
} from "../src/paths.ts";

const scratchRoot = join(process.cwd(), ".scratch", "tmp");
const created: string[] = [];

async function scratch(prefix: string): Promise<string> {
  await mkdir(scratchRoot, { recursive: true });
  const directory = await mkdtemp(join(scratchRoot, prefix));
  created.push(directory);
  return directory;
}

after(async () => {
  for (const directory of created) {
    await rm(directory, { recursive: true, force: true });
  }
});

test("parseLibraryFolders reads paths and unescapes backslashes", () => {
  const vdf = String.raw`
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
	}
	"1"
	{
		"path"		"/Volumes/Games/SteamLibrary"
	}
}
`;

  assert.deepEqual(parseLibraryFolders(vdf), [
    "C:\\Program Files (x86)\\Steam",
    "/Volumes/Games/SteamLibrary",
  ]);
});

test("parseLibraryFolders ignores entries without a path", () => {
  assert.deepEqual(parseLibraryFolders(`"libraryfolders" { "0" { "label" "x" } }`), []);
});

test("readGameInfoStatus detects the addons line and ignores comments", async () => {
  const directory = await scratch("gameinfo-");
  const file = join(directory, "gameinfo.gi");

  await writeFile(
    file,
    ["SearchPaths", "{", "\t// Game citadel/addons", "\tGame citadel", "}"].join("\n"),
  );

  assert.deepEqual(await readGameInfoStatus(file), { present: true, addonsSearchPath: false });

  await writeFile(
    file,
    ["SearchPaths", "{", "\tGame citadel/addons", "\tGame citadel", "}"].join("\n"),
  );

  assert.deepEqual(await readGameInfoStatus(file), {
    present: true,
    addonsSearchPath: true,
    line: 3,
  });
});

test("readGameInfoStatus reports a missing file", async () => {
  const directory = await scratch("gameinfo-missing-");
  assert.deepEqual(await readGameInfoStatus(join(directory, "gameinfo.gi")), {
    present: false,
    addonsSearchPath: false,
  });
});

test("readAddonsStatus sorts slots and finds the next free one", async () => {
  const directory = await scratch("addons-");
  const addons = join(directory, "addons");
  await mkdir(addons);
  await mkdir(join(addons, "nested"));

  for (const [name, contents] of [
    ["pak03_dir.vpk", "three"],
    ["pak01_dir.vpk", "one"],
    ["pak02_000.vpk", "chunk"],
    ["pak00_dir.vpk", "ignored"],
    ["goldcase.vpk", "ignored"],
  ] as const) {
    await writeFile(join(addons, name), contents);
  }

  const status = await readAddonsStatus(addons);

  assert.equal(status.present, true);
  assert.deepEqual(
    status.slots.map((slot) => [slot.slot, slot.fileName, slot.sizeBytes]),
    [
      [1, "pak01_dir.vpk", 3],
      [3, "pak03_dir.vpk", 5],
    ],
  );
  assert.deepEqual(status.chunkFiles, ["pak02_000.vpk"]);
  assert.deepEqual(status.ignoredFiles, ["goldcase.vpk", "nested/", "pak00_dir.vpk"]);
  assert.equal(status.nextFreeSlot, 2);
});

test("readAddonsStatus handles a missing folder", async () => {
  const directory = await scratch("addons-missing-");
  const status = await readAddonsStatus(join(directory, "addons"));

  assert.equal(status.present, false);
  assert.equal(status.nextFreeSlot, 1);
  assert.deepEqual(status.slots, []);
});

test("formatSlotFileName pads and validates", () => {
  assert.equal(formatSlotFileName(1), "pak01_dir.vpk");
  assert.equal(formatSlotFileName(99), "pak99_dir.vpk");
  assert.throws(() => formatSlotFileName(0), RangeError);
  assert.throws(() => formatSlotFileName(100), RangeError);
  assert.throws(() => formatSlotFileName(1.5), RangeError);
});

test("discoverDeadlockInstall follows libraryfolders.vdf", async () => {
  const directory = await scratch("steam-");
  const secondLibrary = join(directory, "second-library");
  const citadel = join(secondLibrary, "steamapps", "common", "Deadlock", "game", "citadel");
  await mkdir(citadel, { recursive: true });
  await mkdir(join(directory, "steamapps"), { recursive: true });
  await writeFile(
    join(directory, "steamapps", "libraryfolders.vdf"),
    `"libraryfolders" { "0" { "path" "${secondLibrary.replace(/\\/g, "\\\\")}" } }`,
  );

  const discovery = await discoverDeadlockInstall({ steamRoots: [directory] });

  assert.equal(discovery.install?.gameRoot, join(secondLibrary, "steamapps", "common", "Deadlock"));
  assert.deepEqual(discovery.searchedRoots, [directory, secondLibrary]);
});

test("discoverDeadlockInstall reports searched roots when nothing is found", async () => {
  const directory = await scratch("steam-empty-");
  const discovery = await discoverDeadlockInstall({ steamRoots: [directory] });

  assert.equal(discovery.install, null);
  assert.deepEqual(discovery.searchedRoots, [directory]);
});
