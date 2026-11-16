import { mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { readVpkDirectory, type VpkHeader } from "./vpk.ts";

/**
 * A cached, searchable list of the paths inside the game's main VPK.
 *
 * The cache stores metadata only. Extracting a file re-reads the VPK tree so we
 * never need to trust a cache that a user or another process could have edited.
 */

export const GAME_INDEX_SCHEMA = 1;

export type IndexEntry = {
  path: string;
  crc32: number;
  preloadBytes: number;
  archiveIndex: number;
  offset: number;
  length: number;
};

export type GameIndexSource = {
  path: string;
  size: number;
  mtimeMs: number;
};

export type GameIndex = {
  schema: typeof GAME_INDEX_SCHEMA;
  source: GameIndexSource;
  builtAt: string;
  header: VpkHeader;
  entries: IndexEntry[];
  skipped: number;
  duplicates: number;
};

export type LoadGameIndexOptions = {
  dirVpkPath: string;
  /** Omit to index without a cache. */
  cachePath?: string;
  force?: boolean;
};

export type LoadGameIndexResult = {
  index: GameIndex;
  fromCache: boolean;
  cachePath?: string;
  cacheError?: string;
};

export type SearchGameIndexOptions = {
  query?: string;
  /** Hero codename, for example `billy` or `punkgoat`. */
  hero?: string;
  /** Compiled extension, with or without a leading dot, for example `vmdl_c`. */
  extension?: string;
  /** Path prefix, for example `models/heroes_wip`. */
  folder?: string;
  limit?: number;
};

export type SearchGameIndexResult = {
  total: number;
  entries: IndexEntry[];
  truncated: boolean;
};

export type HeroSummary = {
  codename: string;
  entries: number;
};

export function defaultGameIndexPath(cacheDir: string): string {
  return join(cacheDir, "citadel-pak01.json");
}

export function gameVpkPath(gameRoot: string): string {
  return join(gameRoot, "game", "citadel", "pak01_dir.vpk");
}

export async function loadGameIndex(
  options: LoadGameIndexOptions,
): Promise<LoadGameIndexResult> {
  const sourceStats = await stat(options.dirVpkPath);
  const source: GameIndexSource = {
    path: options.dirVpkPath,
    size: sourceStats.size,
    mtimeMs: sourceStats.mtimeMs,
  };

  let cacheError: string | undefined;

  if (options.cachePath !== undefined && options.force !== true) {
    const cached = await readCache(options.cachePath, source);

    if (cached !== undefined) {
      return {
        index: cached,
        fromCache: true,
        cachePath: options.cachePath,
      };
    }
  }

  const directory = await readVpkDirectory(options.dirVpkPath, { keepPreload: false });
  const index: GameIndex = {
    schema: GAME_INDEX_SCHEMA,
    source,
    builtAt: new Date().toISOString(),
    header: directory.header,
    entries: [...directory.byPath.values()].map((entry) => ({
      path: entry.path,
      crc32: entry.crc32,
      preloadBytes: entry.preloadBytes,
      archiveIndex: entry.archiveIndex,
      offset: entry.offset,
      length: entry.length,
    })),
    skipped: directory.skipped,
    duplicates: directory.duplicates,
  };

  if (options.cachePath !== undefined) {
    try {
      await writeCache(options.cachePath, index);
    } catch (error) {
      cacheError = error instanceof Error ? error.message : String(error);
    }
  }

  return {
    index,
    fromCache: false,
    ...(options.cachePath === undefined ? {} : { cachePath: options.cachePath }),
    ...(cacheError === undefined ? {} : { cacheError }),
  };
}

export function searchGameIndex(
  index: GameIndex,
  options: SearchGameIndexOptions = {},
): SearchGameIndexResult {
  const query = options.query?.trim().toLowerCase() ?? "";
  const tokens = query.length === 0 ? [] : query.split(/\s+/);
  const hero = options.hero?.trim().toLowerCase();
  const extension = normalizeExtension(options.extension);
  const folder = options.folder?.trim().replace(/^\/+|\/+$/g, "").toLowerCase();
  const limit = clampLimit(options.limit);
  const matches: IndexEntry[] = [];

  for (const entry of index.entries) {
    const path = entry.path.toLowerCase();

    if (tokens.some((token) => !path.includes(token))) continue;
    if (hero !== undefined && !matchesHero(path, hero)) continue;
    if (extension !== undefined && !path.endsWith(`.${extension}`)) continue;
    if (folder !== undefined && folder.length > 0 && !path.startsWith(`${folder}/`)) continue;

    matches.push(entry);
  }

  matches.sort((left, right) => compareMatches(left, right, query));

  return {
    total: matches.length,
    entries: matches.slice(0, limit),
    truncated: matches.length > limit,
  };
}

export function listHeroes(index: GameIndex): HeroSummary[] {
  const counts = new Map<string, number>();

  for (const entry of index.entries) {
    const codename = heroFromPath(entry.path);
    if (codename === undefined) continue;

    counts.set(codename, (counts.get(codename) ?? 0) + 1);
  }

  return [...counts.entries()]
    .map(([codename, entries]) => ({ codename, entries }))
    .sort((left, right) => right.entries - left.entries || left.codename.localeCompare(right.codename));
}

export function topExtensions(index: GameIndex, limit = 8): Array<{ extension: string; entries: number }> {
  const counts = new Map<string, number>();

  for (const entry of index.entries) {
    const dot = entry.path.lastIndexOf(".");
    const extension = dot === -1 ? "(none)" : entry.path.slice(dot + 1).toLowerCase();
    counts.set(extension, (counts.get(extension) ?? 0) + 1);
  }

  return [...counts.entries()]
    .map(([extension, entries]) => ({ extension, entries }))
    .sort((left, right) => right.entries - left.entries || left.extension.localeCompare(right.extension))
    .slice(0, limit);
}

export function formatIndexSummary(
  index: GameIndex,
  loaded: { fromCache: boolean; cachePath?: string; cacheError?: string },
): string {
  const lines = [
    `Game index: ${index.source.path}`,
    `Entries: ${index.entries.length}`,
    `VPK version: ${index.header.version}`,
  ];

  if (index.skipped > 0) lines.push(`Skipped entries: ${index.skipped} (unsafe or empty paths)`);
  if (index.duplicates > 0) lines.push(`Duplicate paths: ${index.duplicates} (last entry wins)`);

  const heroes = listHeroes(index);
  if (heroes.length > 0) {
    const shown = heroes.slice(0, 24).map((hero) => `${hero.codename} (${hero.entries})`);
    lines.push(`Heroes: ${shown.join(", ")}${heroes.length > shown.length ? ", ..." : ""}`);
  }

  const extensions = topExtensions(index);
  if (extensions.length > 0) {
    lines.push(`Top extensions: ${extensions.map((item) => `${item.extension} (${item.entries})`).join(", ")}`);
  }

  lines.push(
    loaded.fromCache
      ? `Cache: ${loaded.cachePath ?? "memory"}`
      : `Cache: rebuilt${loaded.cachePath === undefined ? " (not written)" : ` at ${loaded.cachePath}`}`,
  );

  if (loaded.cacheError !== undefined) lines.push(`Cache warning: ${loaded.cacheError}`);

  return lines.join("\n");
}

export function formatSearchResults(
  result: SearchGameIndexResult,
  options: SearchGameIndexOptions,
): string {
  if (result.total === 0) return "No matching paths in the game index.";

  const filters: string[] = [];
  if (options.query !== undefined && options.query.length > 0) filters.push(`query "${options.query}"`);
  if (options.hero !== undefined) filters.push(`hero ${options.hero}`);
  if (options.extension !== undefined) filters.push(`type ${options.extension}`);
  if (options.folder !== undefined) filters.push(`folder ${options.folder}`);

  const header =
    filters.length === 0
      ? `Showing ${result.entries.length} of ${result.total} paths`
      : `Showing ${result.entries.length} of ${result.total} paths matching ${filters.join(", ")}`;
  const lines = [header];

  for (const entry of result.entries) {
    lines.push(
      `${entry.path} (${entry.length + entry.preloadBytes} bytes, archive ${entry.archiveIndex === 0x7fff ? "dir" : entry.archiveIndex})`,
    );
  }

  if (result.truncated) lines.push("Output truncated. Narrow the query or raise the limit.");
  return lines.join("\n");
}

async function readCache(cachePath: string, source: GameIndexSource): Promise<GameIndex | undefined> {
  let text: string;

  try {
    text = await readFile(cachePath, "utf8");
  } catch {
    return undefined;
  }

  try {
    const parsed: unknown = JSON.parse(text);
    if (!isGameIndex(parsed)) return undefined;
    if (
      parsed.source.path !== source.path ||
      parsed.source.size !== source.size ||
      parsed.source.mtimeMs !== source.mtimeMs
    ) {
      return undefined;
    }

    return parsed;
  } catch {
    return undefined;
  }
}

async function writeCache(cachePath: string, index: GameIndex): Promise<void> {
  await mkdir(dirname(cachePath), { recursive: true });
  const temporary = `${cachePath}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(index)}\n`, "utf8");
  await rename(temporary, cachePath);
}

function isGameIndex(value: unknown): value is GameIndex {
  if (typeof value !== "object" || value === null) return false;

  const index = value as Partial<GameIndex>;
  if (index.schema !== GAME_INDEX_SCHEMA) return false;
  if (typeof index.builtAt !== "string") return false;
  if (typeof index.source !== "object" || index.source === null) return false;
  if (typeof index.header !== "object" || index.header === null) return false;
  if (!Array.isArray(index.entries)) return false;
  if (typeof index.skipped !== "number" || typeof index.duplicates !== "number") return false;

  return index.entries.every((entry) => {
    if (typeof entry !== "object" || entry === null) return false;
    const candidate = entry as Partial<IndexEntry>;
    return (
      typeof candidate.path === "string" &&
      typeof candidate.crc32 === "number" &&
      typeof candidate.preloadBytes === "number" &&
      typeof candidate.archiveIndex === "number" &&
      typeof candidate.offset === "number" &&
      typeof candidate.length === "number"
    );
  });
}

function matchesHero(path: string, hero: string): boolean {
  return new RegExp(`(^|/)heroes[^/]*/${escapeRegExp(hero)}(/|$)`).test(path);
}

function heroFromPath(path: string): string | undefined {
  const match = /(^|\/)heroes[^/]*\/([^/]+)\//i.exec(path);
  return match?.[2]?.toLowerCase();
}

function normalizeExtension(value: string | undefined): string | undefined {
  if (value === undefined) return undefined;
  const normalized = value.trim().toLowerCase().replace(/^\./, "");
  return normalized.length === 0 ? undefined : normalized;
}

function clampLimit(value: number | undefined): number {
  if (value === undefined || !Number.isFinite(value)) return 50;
  return Math.min(Math.max(Math.trunc(value), 1), 500);
}

function compareMatches(left: IndexEntry, right: IndexEntry, query: string): number {
  if (query.length > 0) {
    const score = scorePath(left.path, query) - scorePath(right.path, query);
    if (score !== 0) return score;
  }

  return left.path.length - right.path.length || left.path.localeCompare(right.path);
}

function scorePath(path: string, query: string): number {
  const lowered = path.toLowerCase();
  const name = lowered.slice(lowered.lastIndexOf("/") + 1);

  if (name === query) return 0;
  if (lowered.endsWith(`/${query}`)) return 1;
  if (name.startsWith(query)) return 2;
  return 3;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
