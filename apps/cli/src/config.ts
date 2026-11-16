import { homedir } from "node:os";
import { join } from "node:path";

export type CliPaths = {
  dataDir: string;
  storagePath: string;
  credentialsPath: string;
  workspaceDir: string;
};

export function resolveCliPaths(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  home: string = homedir(),
): CliPaths {
  const dataDir = env.DAH_DATA_DIR ?? defaultDataDir(platform, home, env);
  const workspaceDir = env.DAH_WORKSPACE_DIR ?? join(dataDir, "workspace");

  return {
    dataDir,
    storagePath: join(dataDir, "agent.sqlite"),
    credentialsPath: join(dataDir, "credentials.json"),
    workspaceDir,
  };
}

function defaultDataDir(
  platform: NodeJS.Platform,
  home: string,
  env: NodeJS.ProcessEnv,
): string {
  if (platform === "darwin") {
    return join(home, "Library", "Application Support", "DeadlockAgentHarness");
  }

  if (platform === "win32") {
    return join(env.APPDATA ?? join(home, "AppData", "Roaming"), "DeadlockAgentHarness");
  }

  return join(env.XDG_DATA_HOME ?? join(home, ".local", "share"), "deadlock-agent-harness");
}
