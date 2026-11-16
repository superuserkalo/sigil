import { readFile, readdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export const DEADLOCK_APP_ID = "1422450";

const GAMEINFO_RELATIVE = join("game", "citadel", "gameinfo.gi");
const CITADEL_RELATIVE = join("game", "citadel");
const ADDONS_RELATIVE = join("game", "citadel", "addons");
const GAME_RELATIVE = join("steamapps", "common", "Deadlock");

export type DeadlockInstall = {
  steamRoot: string;
  gameRoot: string;
  citadelDir: string;
  gameinfoPath: string;
  addonsDir: string;
};

export type GameInfoStatus = {
  present: boolean;
  addonsSearchPath: boolean;
  line?: number;
};

export type AddonSlot = {
  slot: number;
  fileName: string;
  sizeBytes: number;
};

export type AddonsStatus = {
  directory: string;
  present: boolean;
  slots: AddonSlot[];
  chunkFiles: string[];
  ignoredFiles: string[];
  nextFreeSlot: number | null;
};

export type InstallDiscovery = {
  install: DeadlockInstall | null;
  searchedRoots: string[];
};

export type GameStatus = {
  install: DeadlockInstall | null;
  searchedRoots: string[];
  gameinfo: GameInfoStatus | null;
  addons: AddonsStatus | null;
};

export type InstallDiscoveryOptions = {
  steamRoots?: readonly string[];
  platform?: NodeJS.Platform;
  home?: string;
  env?: NodeJS.ProcessEnv;
};

export function parseLibraryFolders(vdf: string): string[] {
  const libraries: string[] = [];
  const pattern = /"path"\s+"((?:[^"\\]|\\.)*)"/g;

  for (const match of vdf.matchAll(pattern)) {
    const raw = match[1];
    if (raw === undefined || raw.length === 0) continue;
    libraries.push(raw.replace(/\\(["\\])/g, "$1"));
  }

  return libraries;
}

export function defaultSteamRoots(
  platform: NodeJS.Platform = process.platform,
  home: string = homedir(),
  env: NodeJS.ProcessEnv = process.env,
): string[] {
  if (platform === "darwin") {
    return [join(home, "Library", "Application Support", "Steam")];
  }

  if (platform === "win32") {
    const programFilesX86 = env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)";
    const programFiles = env.ProgramFiles ?? "C:\\Program Files";
    return [join(programFilesX86, "Steam"), join(programFiles, "Steam")];
  }

  return [
    join(home, ".steam", "steam"),
    join(home, ".steam", "root"),
    join(home, ".local", "share", "Steam"),
  ];
}

export function formatSlotFileName(slot: number): string {
  if (!Number.isInteger(slot) || slot < 1 || slot > 99) {
    throw new RangeError(`Addon slot must be an integer from 1 to 99, received ${slot}`);
  }

  return `pak${slot.toString().padStart(2, "0")}_dir.vpk`;
}

export async function discoverDeadlockInstall(
  options: InstallDiscoveryOptions = {},
): Promise<InstallDiscovery> {
  const configuredRoots = options.steamRoots;
  const seedRoots =
    configuredRoots ??
    defaultSteamRoots(options.platform ?? process.platform, options.home ?? homedir(), options.env ?? process.env);

  const searchedRoots: string[] = [];

  for (const root of seedRoots) {
    for (const candidate of await expandSteamRoot(root)) {
      if (!searchedRoots.includes(candidate)) searchedRoots.push(candidate);
    }
  }

  for (const steamRoot of searchedRoots) {
    const gameRoot = join(steamRoot, GAME_RELATIVE);
    const citadelDir = join(gameRoot, CITADEL_RELATIVE);

    if (!(await isDirectory(citadelDir))) continue;

    return {
      install: {
        steamRoot,
        gameRoot,
        citadelDir,
        gameinfoPath: join(gameRoot, GAMEINFO_RELATIVE),
        addonsDir: join(gameRoot, ADDONS_RELATIVE),
      },
      searchedRoots,
    };
  }

  return { install: null, searchedRoots };
}

export async function readGameInfoStatus(gameinfoPath: string): Promise<GameInfoStatus> {
  let text: string;

  try {
    text = await readFile(gameinfoPath, "utf8");
  } catch (error) {
    if (isNotFound(error)) return { present: false, addonsSearchPath: false };
    throw error;
  }

  const lines = text.split(/\r?\n/);

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (line === undefined) continue;

    const withoutComment = line.replace(/\/\/.*$/, "");
    if (/^\s*Game\s+citadel[\\/]addons\s*$/i.test(withoutComment)) {
      return { present: true, addonsSearchPath: true, line: index + 1 };
    }
  }

  return { present: true, addonsSearchPath: false };
}

export async function readAddonsStatus(addonsDir: string): Promise<AddonsStatus> {
  let entries;

  try {
    entries = await readdir(addonsDir, { withFileTypes: true });
  } catch (error) {
    if (isNotFound(error)) {
      return {
        directory: addonsDir,
        present: false,
        slots: [],
        chunkFiles: [],
        ignoredFiles: [],
        nextFreeSlot: 1,
      };
    }
    throw error;
  }

  const slots: AddonSlot[] = [];
  const chunkFiles: string[] = [];
  const ignoredFiles: string[] = [];

  for (const entry of entries) {
    if (!entry.isFile()) {
      ignoredFiles.push(`${entry.name}/`);
      continue;
    }

    const slotMatch = /^pak(\d{2})_dir\.vpk$/i.exec(entry.name);

    if (slotMatch) {
      const slot = Number(slotMatch[1]);
      if (slot >= 1 && slot <= 99) {
        let sizeBytes = 0;
        try {
          sizeBytes = (await stat(join(addonsDir, entry.name))).size;
        } catch {
          // The file vanished between listing and stat. Report it with size 0.
        }
        slots.push({ slot, fileName: entry.name, sizeBytes });
        continue;
      }
    }

    if (/^pak\d{2}_\d{3}\.vpk$/i.test(entry.name)) {
      chunkFiles.push(entry.name);
      continue;
    }

    ignoredFiles.push(entry.name);
  }

  slots.sort((left, right) => left.slot - right.slot);

  const used = new Set(slots.map((slot) => slot.slot));
  let nextFreeSlot: number | null = null;

  for (let slot = 1; slot <= 99; slot += 1) {
    if (!used.has(slot)) {
      nextFreeSlot = slot;
      break;
    }
  }

  return {
    directory: addonsDir,
    present: true,
    slots,
    chunkFiles: chunkFiles.sort(),
    ignoredFiles: ignoredFiles.sort(),
    nextFreeSlot,
  };
}

export async function readGameStatus(options: InstallDiscoveryOptions = {}): Promise<GameStatus> {
  const discovery = await discoverDeadlockInstall(options);

  if (discovery.install === null) {
    return {
      install: null,
      searchedRoots: discovery.searchedRoots,
      gameinfo: null,
      addons: null,
    };
  }

  const gameinfo = await readGameInfoStatus(discovery.install.gameinfoPath);
  const addons = await readAddonsStatus(discovery.install.addonsDir);

  return {
    install: discovery.install,
    searchedRoots: discovery.searchedRoots,
    gameinfo,
    addons,
  };
}

export function formatGameStatus(status: GameStatus): string {
  const lines: string[] = [];

  if (status.install === null) {
    lines.push("Deadlock install: not found");
    lines.push("Searched Steam roots:");
    for (const root of status.searchedRoots) lines.push(`  ${root}`);
    lines.push("Install Deadlock through Steam or pass a steam root explicitly.");
    return lines.join("\n");
  }

  lines.push(`Deadlock install: ${status.install.gameRoot}`);

  if (status.gameinfo === null || !status.gameinfo.present) {
    lines.push("gameinfo.gi: missing");
  } else if (status.gameinfo.addonsSearchPath) {
    lines.push(`gameinfo.gi: addons SearchPaths present at line ${status.gameinfo.line ?? "?"}`);
  } else {
    lines.push("gameinfo.gi: addons SearchPaths missing, mods will not load until repaired");
  }

  if (status.addons === null || !status.addons.present) {
    lines.push("Addon folder: missing");
  } else if (status.addons.slots.length === 0) {
    lines.push(`Addon folder: ${status.addons.directory} (no slots used)`);
  } else {
    const used = status.addons.slots.map((slot) => String(slot.slot).padStart(2, "0")).join(", ");
    lines.push(`Addon slots in use: ${used}`);
  }

  if (status.addons !== null) {
    lines.push(
      status.addons.nextFreeSlot === null
        ? "Next free slot: none, all 99 slots are used"
        : `Next free slot: ${formatSlotFileName(status.addons.nextFreeSlot)}`,
    );

    if (status.addons.ignoredFiles.length > 0) {
      lines.push(`Ignored files: ${status.addons.ignoredFiles.join(", ")}`);
    }
    if (status.addons.chunkFiles.length > 0) {
      lines.push(`Chunk files: ${status.addons.chunkFiles.join(", ")}`);
    }
  }

  return lines.join("\n");
}

async function expandSteamRoot(root: string): Promise<string[]> {
  const roots = [root];
  const libraryFile = join(root, "steamapps", "libraryfolders.vdf");

  try {
    const vdf = await readFile(libraryFile, "utf8");
    for (const library of parseLibraryFolders(vdf)) {
      if (!roots.includes(library)) roots.push(library);
    }
  } catch (error) {
    if (!isNotFound(error)) throw error;
  }

  return roots;
}

async function isDirectory(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isDirectory();
  } catch (error) {
    if (isNotFound(error)) return false;
    throw error;
  }
}

function isNotFound(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}
