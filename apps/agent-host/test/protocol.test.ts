import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("../src/main.ts", import.meta.url));

function asRecord(value: unknown): Record<string, unknown> {
  assert.ok(value !== null && typeof value === "object" && !Array.isArray(value));
  return value as Record<string, unknown>;
}

test("host answers list_sessions, state and shutdown over JSONL", async () => {
  const directory = await mkdtemp(join(tmpdir(), "dah-agent-host-"));
  const child = spawn(process.execPath, ["--import", "tsx", entry], {
    env: {
      ...process.env,
      DAH_AGENT_PROVIDER: "chat",
      DAH_AGENT_MODEL: "test-model",
      DAH_AGENT_BASE_URL: "http://127.0.0.1:9/v1",
      DAH_AGENT_API_KEY: "not-used-by-this-test",
      DAH_DATA_DIR: directory,
      DAH_WORKSPACE_DIR: join(directory, "workspace"),
      DAH_RS_BIN: join(directory, "missing-dah-rs"),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });

  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });

  const queue: unknown[] = [];
  const waiters: Array<(value: unknown) => void> = [];
  let buffer = "";

  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    buffer += chunk;
    for (let newline = buffer.indexOf("\n"); newline >= 0; newline = buffer.indexOf("\n")) {
      const line = buffer.slice(0, newline);
      buffer = buffer.slice(newline + 1);
      if (line.trim().length === 0) continue;
      const value: unknown = JSON.parse(line);
      const waiter = waiters.shift();
      if (waiter) waiter(value);
      else queue.push(value);
    }
  });

  const nextEnvelope = (): Promise<Record<string, unknown>> => {
    const queued = queue.shift();
    if (queued !== undefined) return Promise.resolve(asRecord(queued));

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`timed out waiting for an envelope. stderr:\n${stderr}`));
      }, 10_000);
      waiters.push((value) => {
        clearTimeout(timer);
        try {
          resolve(asRecord(value));
        } catch (error) {
          reject(error as Error);
        }
      });
    });
  };

  const send = (value: unknown): void => {
    child.stdin.write(`${JSON.stringify(value)}\n`);
  };

  const exited = new Promise<number | null>((resolve) => {
    child.on("exit", (code) => resolve(code));
  });

  try {
    send({ id: "1", type: "list_sessions" });
    const listed = await nextEnvelope();
    assert.equal(listed.ok, true);
    assert.deepEqual(listed.sessionIds, []);
    assert.equal(listed.sessionId, null);

    send({ id: "2", type: "state" });
    const state = await nextEnvelope();
    assert.equal(state.ok, true);
    assert.equal(state.sessionId, null);
    assert.equal(asRecord(state.state).status, "idle");

    send({ id: "3", type: "new_session" });
    const created = await nextEnvelope();
    assert.equal(created.ok, true);
    assert.match(String(created.sessionId), /^s_/);

    send({ id: "4", type: "shutdown" });
    const stopped = await nextEnvelope();
    assert.equal(stopped.ok, true);
    assert.equal(stopped.stopped, true);

    const code = await exited;
    assert.equal(code, 0, stderr);
  } finally {
    if (child.exitCode === null) child.kill("SIGKILL");
    await rm(directory, { recursive: true, force: true });
  }
});
