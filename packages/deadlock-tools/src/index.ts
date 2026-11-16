export {
  DEADLOCK_APP_ID,
  defaultSteamRoots,
  discoverDeadlockInstall,
  formatGameStatus,
  formatSlotFileName,
  parseLibraryFolders,
  readAddonsStatus,
  readGameInfoStatus,
  readGameStatus,
} from "./paths.ts";

export type {
  AddonSlot,
  AddonsStatus,
  DeadlockInstall,
  GameInfoStatus,
  GameStatus,
  InstallDiscovery,
  InstallDiscoveryOptions,
} from "./paths.ts";

export {
  formatPayloadIssues,
  hasErrors,
  needsChunking,
  validatePayload,
} from "./validate.ts";

export type {
  PayloadIssue,
  PayloadSeverity,
  PayloadValidationOptions,
} from "./validate.ts";

export {
  GAME_INDEX_SCHEMA,
  defaultGameIndexPath,
  formatIndexSummary,
  formatSearchResults,
  gameVpkPath,
  listHeroes,
  loadGameIndex,
  searchGameIndex,
  topExtensions,
} from "./game-index.ts";

export type {
  GameIndex,
  GameIndexSource,
  HeroSummary,
  IndexEntry,
  LoadGameIndexOptions,
  LoadGameIndexResult,
  SearchGameIndexOptions,
  SearchGameIndexResult,
} from "./game-index.ts";

export { extractFromVpk, formatExtractResult } from "./extract.ts";

export type { ExtractIssue, ExtractOptions, ExtractResult, ExtractedFile } from "./extract.ts";

export {
  VPK_DIRECTORY_ARCHIVE,
  VPK_MAGIC,
  chunkPathFor,
  crc32,
  isSafeVpkPath,
  readVpkDirectory,
  readVpkEntryData,
} from "./vpk.ts";

export type { ReadVpkOptions, VpkDirectory, VpkEntry, VpkHeader } from "./vpk.ts";

export { createDeadlockExtension, deadlockExtension } from "./tool.ts";

export type { DeadlockExtensionOptions } from "./tool.ts";
