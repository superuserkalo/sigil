import { defineExtension, defineTool, section } from "@earendil-works/pi-durable";
import { arr, obj, opt, str } from "@earendil-works/pi-schema";
import { formatGameStatus, readGameStatus } from "./paths.ts";
import { formatPayloadIssues, hasErrors, validatePayload } from "./validate.ts";

const RULES = `Deadlock uses Valve's Source 2 engine. The internal game name is citadel.
Mods load from game/citadel/addons as pakNN_dir.vpk, where NN is 01 through 99.
Lower numbers win when two mods ship the same file. Other names and subfolders are ignored.
A mod replaces files by exact game path. One wrong folder or letter and the game silently uses its own file.
A payload contains compiled files only (names ending in _c) at their exact game paths.
Never include source assets, tool caches, scripts/*.vdata_c, *.vnmskel_c, archives or executables.
gameinfo.gi must list "Game citadel/addons" above "Game citadel" or no mods load. Game updates reset it.
Compiling needs the Windows CSDK 12 toolchain. Packing, inspecting, merging and validating work cross-platform.
Only the main process may write to a game install, and only after explicit user approval.
Keep every mod cosmetic. No gameplay data, no skeleton replacement, no matchmaking advantage.`;

export const deadlockExtension = defineExtension({
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
