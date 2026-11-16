import assert from "node:assert/strict";
import { test } from "node:test";
import {
  formatPayloadIssues,
  hasErrors,
  needsChunking,
  validatePayload,
} from "../src/validate.ts";

test("accepts compiled files at game paths", () => {
  const issues = validatePayload([
    "models/heroes_wip/punkgoat/punkgoat.vmdl_c",
    "sounds/heroes/punkgoat/ability_cast.vsnd_c",
    "panorama/images/heroes/punkgoat_icon.vtex_c",
  ]);

  assert.deepEqual(issues, []);
  assert.equal(hasErrors(issues), false);
});

test("rejects an empty payload", () => {
  const issues = validatePayload([]);
  assert.equal(issues[0]?.code, "empty_payload");
  assert.equal(hasErrors(issues), true);
});

test("rejects source assets", () => {
  const issues = validatePayload(["models/heroes_wip/punkgoat/punkgoat.vmdl"]);
  assert.ok(issues.some((issue) => issue.code === "source_file"));
  assert.equal(hasErrors(issues), true);
});

test("rejects gameplay data", () => {
  const issues = validatePayload(["scripts/heroes.vdata_c"]);
  assert.ok(issues.some((issue) => issue.code === "gameplay_data"));
});

test("rejects skeletons", () => {
  const issues = validatePayload(["models/heroes_wip/punkgoat/punkgoat.vnmskel_c"]);
  assert.ok(issues.some((issue) => issue.code === "skeleton"));
});

test("rejects tool caches", () => {
  const issues = validatePayload(["cache_models.soc", "tools_thumbnail_cache.bin"]);
  assert.equal(issues.filter((issue) => issue.code === "tool_cache").length, 2);
});

test("rejects executables and archives", () => {
  const issues = validatePayload(["bin/payload.exe", "bundle.zip"]);
  assert.ok(issues.some((issue) => issue.code === "executable"));
  assert.ok(issues.some((issue) => issue.code === "archive"));
});

test("rejects absolute, backslash and parent paths", () => {
  const issues = validatePayload([
    "/models/a.vmdl_c",
    "C:/models/b.vmdl_c",
    "models\\c.vmdl_c",
    "models/../d.vmdl_c",
  ]);

  assert.ok(issues.some((issue) => issue.code === "absolute_path"));
  assert.ok(issues.some((issue) => issue.code === "backslash_path"));
  assert.ok(issues.some((issue) => issue.code === "invalid_segment"));
});

test("rejects a staging folder prefix", () => {
  const issues = validatePayload(["goldcase/models/a.vmdl_c"], { stagingFolder: "goldcase" });
  assert.ok(issues.some((issue) => issue.code === "staging_folder_prefix"));
});

test("rejects duplicates case-insensitively", () => {
  const issues = validatePayload(["models/A.vmdl_c", "models/a.vmdl_c"]);
  assert.ok(issues.some((issue) => issue.code === "duplicate_path"));
});

test("warns about unknown uncompiled entries", () => {
  const issues = validatePayload(["readme.md"]);
  const warning = issues.find((issue) => issue.code === "not_compiled");

  assert.equal(warning?.severity, "warning");
  assert.equal(hasErrors(issues), false);
});

test("formats issues for the model", () => {
  const text = formatPayloadIssues(validatePayload(["scripts/game.vdata_c"]));
  assert.match(text, /ERROR gameplay_data/);
});

test("flags payloads above the single VPK limit", () => {
  assert.equal(needsChunking(1024), false);
  assert.equal(needsChunking(2 * 1024 * 1024 * 1024), false);
  assert.equal(needsChunking(2 * 1024 * 1024 * 1024 + 1), true);
});
