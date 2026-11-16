import { Tool, ToolError } from "tiny-agent-sdk";
import type { Item } from "tiny-agent-sdk";
import type { DahRsCli } from "./dah-rs.ts";
import { hasErrors, validatePayload } from "./payload.ts";

const EMPTY_PARAMETERS: Item = {
  type: "object",
  properties: {},
  additionalProperties: false,
};

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;
}

function stringList(value: unknown, name: string): string[] {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) {
    throw new ToolError(`${name} must be an array of strings.`);
  }
  return value as string[];
}

function optionalInteger(value: unknown): number | undefined {
  return typeof value === "number" && Number.isInteger(value) ? value : undefined;
}

function compact(value: unknown): string {
  const encoded = JSON.stringify(value);
  return encoded === undefined ? "null" : encoded;
}

export function createDeadlockTools(options: { cli: DahRsCli }): Tool[] {
  const { cli } = options;

  return [
    new Tool({
      name: "game_status",
      description:
        "Report the Deadlock install, the gameinfo.gi SearchPaths status and the addon slots in use. Read-only.",
      parameters: EMPTY_PARAMETERS,
      idempotent: true,
      handler: () => cli.json(["doctor", "--json"]),
    }),

    new Tool({
      name: "game_index",
      description:
        "Build or load the cached index of game VPK paths. Reports entry counts, sources and any change since the previous index. Read-only toward the game install.",
      parameters: {
        type: "object",
        properties: {
          refresh: {
            type: ["boolean", "null"],
            description: "Rebuild the index even when the cache matches the game files. Pass null to keep the cache.",
          },
        },
        required: ["refresh"],
        additionalProperties: false,
      },
      idempotent: false,
      handler: (args) => cli.json(["index", ...(args.refresh === true ? ["--refresh"] : []), "--json"]),
    }),

    new Tool({
      name: "search_paths",
      description:
        "Search the game index for exact game paths with sizes. Filter by text, hero codename, compiled file type or folder. Read-only.",
      parameters: {
        type: "object",
        properties: {
          query: {
            type: ["string", "null"],
            description: "Case-insensitive text that must appear in the path. Pass null with a filter.",
          },
          hero: {
            type: ["string", "null"],
            description: "Hero codename, for example billy or punkgoat. Pass null to skip.",
          },
          type: {
            type: ["string", "null"],
            description: "Compiled extension, for example vmdl_c or vtex_c. Pass null to skip.",
          },
          folder: {
            type: ["string", "null"],
            description: "Path prefix, for example models/heroes_wip. Pass null to skip.",
          },
          limit: {
            type: ["integer", "null"],
            minimum: 1,
            maximum: 1000,
            description: "Maximum paths to return. Pass null for the default of 50.",
          },
        },
        required: ["query", "hero", "type", "folder", "limit"],
        additionalProperties: false,
      },
      idempotent: true,
      handler: (args) => {
        const query = optionalString(args.query);
        const hero = optionalString(args.hero);
        const type = optionalString(args.type);
        const folder = optionalString(args.folder);

        if (query === undefined && hero === undefined && type === undefined && folder === undefined) {
          throw new ToolError("Provide query, hero, type or folder. At least one filter is required.");
        }

        const limit = optionalInteger(args.limit);

        return cli.json([
          "find",
          query ?? "",
          ...(hero === undefined ? [] : ["--hero", hero]),
          ...(type === undefined ? [] : ["--type", type]),
          ...(folder === undefined ? [] : ["--folder", folder]),
          ...(limit === undefined ? [] : ["--limit", String(limit)]),
          "--json",
        ]);
      },
    }),

    new Tool({
      name: "extract_paths",
      description:
        "Extract exact game paths from the game VPK into the mod workspace. It writes inside the workspace only and keeps compiled files compiled. Not a game install write.",
      parameters: {
        type: "object",
        properties: {
          paths: {
            type: "array",
            items: { type: "string" },
            minItems: 1,
            description: "Exact game paths from search_paths.",
          },
          destination: {
            type: ["string", "null"],
            description: "Folder inside the mod workspace. Pass null for `extracted`, with game paths preserved below it.",
          },
          overwrite: {
            type: ["boolean", "null"],
            description: "Replace files that already exist. Pass null or false to keep them.",
          },
        },
        required: ["paths", "destination", "overwrite"],
        additionalProperties: false,
      },
      idempotent: false,
      handler: (args) => {
        const paths = stringList(args.paths, "paths");
        if (paths.length === 0) throw new ToolError("Pass at least one exact game path.");
        const destination = optionalString(args.destination);

        return cli.json([
          "extract",
          ...paths,
          ...(destination === undefined ? [] : ["--out", destination]),
          ...(args.overwrite === true ? ["--overwrite"] : []),
          "--json",
        ]);
      },
    }),

    new Tool({
      name: "project_check",
      description:
        "Check the current mod project against the current game files: changed, missing and moved paths, gameinfo.gi and the addon slot. Read-only.",
      parameters: EMPTY_PARAMETERS,
      idempotent: true,
      handler: () => cli.json(["mod", "check", "--json"]),
    }),

    new Tool({
      name: "patch_notes",
      description:
        "Fetch or read cached official Deadlock patch posts from Steam. Reports titles, dates and which posts are new. Does not replace the game index or project check.",
      parameters: {
        type: "object",
        properties: {
          limit: {
            type: ["integer", "null"],
            minimum: 1,
            maximum: 50,
            description: "Maximum posts to return. Pass null for the default of 10.",
          },
          refresh: {
            type: ["boolean", "null"],
            description: "Fetch from Steam even when the cache is fresh. Pass null to use the cache.",
          },
        },
        required: ["limit", "refresh"],
        additionalProperties: false,
      },
      idempotent: false,
      handler: (args) => {
        const limit = optionalInteger(args.limit);
        return cli.json([
          "patches",
          ...(limit === undefined ? [] : ["--limit", String(limit)]),
          ...(args.refresh === true ? ["--refresh"] : []),
          "--json",
        ]);
      },
    }),

    new Tool({
      name: "validate_payload",
      description:
        "Validate a proposed VPK entry list against the Deadlock payload rules before packing. Pure function, no file access. Reports errors and warnings.",
      parameters: {
        type: "object",
        properties: {
          entries: {
            type: "array",
            items: { type: "string" },
            description: "VPK entry paths, relative to the game root.",
          },
          stagingFolder: {
            type: ["string", "null"],
            description: "Staging folder name that must not appear as the first path segment. Pass null to skip.",
          },
        },
        required: ["entries", "stagingFolder"],
        additionalProperties: false,
      },
      idempotent: true,
      handler: (args) => {
        const entries = stringList(args.entries, "entries");
        const stagingFolder = optionalString(args.stagingFolder);
        const issues = validatePayload(
          entries,
          stagingFolder === undefined ? {} : { stagingFolder },
        );

        return compact({ ok: !hasErrors(issues), issues });
      },
    }),
  ];
}
