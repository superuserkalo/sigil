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

export { deadlockExtension } from "./tool.ts";
