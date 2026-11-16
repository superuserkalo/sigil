import { join, relative, resolve, isAbsolute } from "node:path";
import { defineExtension, defineTool, section } from "@earendil-works/pi-durable";
import { arr, bool, int, obj, opt, str } from "@earendil-works/pi-schema";
import { extractFromVpk, formatExtractResult } from "./extract.ts";
import {
  defaultGameIndexPath,
  formatIndexSummary,
  formatSearchResults,
  gameVpkPath,
  loadGameIndex,
  searchGameIndex,
} from "./game-index.ts";
import { formatGameStatus, readGameStatus } from "./paths.ts";
import { formatPayloadIssues, hasErrors, validatePayload } from "./validate.ts";

const RULES = `Deadlock uses Valve's Source 2 engine. The internal game name is citadel.
Mods load from game/citadel/addons as pakNN_dir.vpk, where NN is 01 through 99.
Lower numbers win when two mods ship the same file. Other names and subfolders are ignored.
A mod replaces files by exact game path. One wrong folder or letter and the game silently uses its own file.
A payload contains compiled files only (names ending in _c) at their exact game paths.
Never include source assets, tool caches, scripts/*.vdata_c, *.vnmskel_c, archives or executables.
gameinfo.gi must list "Game citadel/addons" above "Game citadel" or no mods load. Game updates reset it.
The game index lists every path in game/citadel/pak01_dir.vpk. Search it before guessing a path.
Extraction writes game files into the mod workspace. It never writes to the game install.
Decompiling extracted _c files requires Source 2 Viewer, which is not bundled yet.
Compiling needs the Windows CSDK 12 toolchain. Packing, inspecting, merging and validating work cross-platform.
Only the main process may write to a game install, and only after explicit user approval.
Keep every mod cosmetic. No gameplay data, no skeleton replacement, no matchmaking advantage.`;

export type DeadlockExtensionOptions = {
  /** Mod workspace. Extraction and the index cache stay inside it. */
  workspaceDir: string;
  /** Override the index cache directory. Defaults to `<workspace>/.dah/index`. */
  cacheDir?: string;
};

export function createDeadlockExtension(options: DeadlockExtensionOptions) {
  const workspaceDir = resolve(options.workspaceDir);
  const cachePath = defaultGameIndexPath(options.cacheDir ?? join(workspaceDir, ".dah", "index"));

  return defineExtension({
    name: "deadlock",
    sections: [section("deadlock_rules", () => RULES)],
    tools: [
      defineTool({
        name: "deadlock_game_status",
        description:
          "Report the Deadlock install, the gameinfo.gi SearchPaths status, and the addon slots in use. Read-only.",
        parameters: obj({}),
        replay: "safe",
        execute: async () => {
          const status = await readGameStatus();
          return {
            output: [{ type: "text" as const, text: formatGameStatus(status) }],
          };
        },
      }),
      defineTool({
        name: "deadlock_game_index",
        description:
          "Build or load a cached index of every path in the game's main VPK. Reports entry count, heroes and file types. Read-only.",
        parameters: obj({
          refresh: opt(bool({ description: "Rebuild even when the cache matches the VPK." })),
        }),
        replay: "safe",
        execute: async (args) => {
          const install = await requireInstall();

          if (typeof install === "string") return errorResult(install);

          try {
            const loaded = await loadGameIndex({
              dirVpkPath: gameVpkPath(install.gameRoot),
              cachePath,
              force: args.refresh === true,
            });

            return {
              output: [
                {
                  type: "text" as const,
                  text: formatIndexSummary(loaded.index, {
                    fromCache: loaded.fromCache,
                    ...(loaded.cachePath === undefined ? {} : { cachePath: loaded.cachePath }),
                    ...(loaded.cacheError === undefined ? {} : { cacheError: loaded.cacheError }),
                  }),
                },
              ],
            };
          } catch (error) {
            return errorResult(`Could not index the game: ${messageOf(error)}`);
          }
        },
      }),
      defineTool({
        name: "deadlock_search_paths",
        description:
          "Search the game index by text, hero codename, compiled file type or folder. Returns exact game paths with sizes. Read-only.",
        parameters: obj({
          query: opt(str({ description: "Case-insensitive text that must appear in the path." })),
          hero: opt(str({ description: "Hero codename, for example billy or punkgoat." })),
          type: opt(str({ description: "Compiled extension, for example vmdl_c or vtex_c." })),
          folder: opt(str({ description: "Path prefix, for example models/heroes_wip." })),
          limit: opt(int({ minimum: 1, maximum: 500, description: "Maximum paths to return. Default 50." })),
        }),
        replay: "safe",
        execute: async (args) => {
          const install = await requireInstall();

          if (typeof install === "string") return errorResult(install);

          try {
            const loaded = await loadGameIndex({
              dirVpkPath: gameVpkPath(install.gameRoot),
              cachePath,
            });
            const searchOptions = {
              ...(args.query === undefined ? {} : { query: args.query }),
              ...(args.hero === undefined ? {} : { hero: args.hero }),
              ...(args.type === undefined ? {} : { extension: args.type }),
              ...(args.folder === undefined ? {} : { folder: args.folder }),
              ...(args.limit === undefined ? {} : { limit: args.limit }),
            };

            return {
              output: [
                {
                  type: "text" as const,
                  text: formatSearchResults(searchGameIndex(loaded.index, searchOptions), searchOptions),
                },
              ],
            };
          } catch (error) {
            return errorResult(`Could not search the game index: ${messageOf(error)}`);
          }
        },
      }),
      defineTool({
        name: "deadlock_extract",
        description:
          "Extract exact game paths from the game VPK into the mod workspace. Writes inside the workspace only. Does not decompile; compiled _c files stay compiled.",
        parameters: obj({
          paths: arr(str(), { description: "Exact game paths from deadlock_search_paths." }),
          destination: opt(
            str({
              description:
                "Destination folder inside the mod workspace. Default `extracted`. Game paths are preserved below it.",
            }),
          ),
          overwrite: opt(bool({ description: "Replace files that already exist. Default false." })),
        }),
        replay: "unsafe",
        execute: async (args) => {
          const install = await requireInstall();

          if (typeof install === "string") return errorResult(install);

          const destination = resolve(workspaceDir, args.destination ?? "extracted");
          const relativeDestination = relative(workspaceDir, destination);

          if (relativeDestination.startsWith("..") || isAbsolute(relativeDestination)) {
            return errorResult("Destination must stay inside the mod workspace.");
          }

          try {
            const result = await extractFromVpk({
              dirVpkPath: gameVpkPath(install.gameRoot),
              entryPaths: args.paths,
              destination,
              overwrite: args.overwrite === true,
              maxFiles: 200,
            });

            return {
              output: [{ type: "text" as const, text: formatExtractResult(result) }],
              isError: result.errors.length > 0,
            };
          } catch (error) {
            return errorResult(`Could not extract from the game VPK: ${messageOf(error)}`);
          }
        },
      }),
      defineTool({
        name: "deadlock_validate_payload",
        description:
          "Validate a proposed VPK entry list against the Deadlock mod payload rules before packing or deploying. Pure function, no file access.",
        parameters: obj({
          entries: arr(str()),
          stagingFolder: opt(str()),
        }),
        replay: "safe",
        execute: async (args) => {
          const options = args.stagingFolder === undefined ? {} : { stagingFolder: args.stagingFolder };
          const issues = validatePayload(args.entries, options);
          return {
            output: [{ type: "text" as const, text: formatPayloadIssues(issues) }],
            isError: hasErrors(issues),
          };
        },
      }),
    ],
  });
}

/** Extension rooted at the current process directory, for direct use and tests. */
export const deadlockExtension = createDeadlockExtension({ workspaceDir: process.cwd() });

async function requireInstall(): Promise<{ gameRoot: string } | string> {
  const status = await readGameStatus();

  if (status.install === null) {
    return `${formatGameStatus(status)}\n\nSet DAH_GAME_DIR to a Deadlock game folder to use this tool without Steam discovery.`;
  }

  return { gameRoot: status.install.gameRoot };
}

function errorResult(message: string) {
  return {
    output: [{ type: "text" as const, text: message }],
    isError: true,
  };
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
