import { describe, expect, it } from "vitest";
import { diagnosticsCode, CODE_PATTERN } from "../src/codes";
import { EVENT_NAMES, FEATURE_EVENTS, isEventName, isPageName, PAGE_NAMES } from "../src/events";
import { crashFingerprint, fingerprintFrames } from "../src/fingerprint";
import { sha256Hex } from "../src/hash";
import { scrub } from "../src/scrub";
import { isSemver, parseEvents, parseHashes, parseIdentify } from "../src/validate";

const INSTALL_ID = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";

describe("event allowlist", () => {
  it("names every screen and action once", () => {
    expect(new Set(EVENT_NAMES).size).toBe(EVENT_NAMES.length);
    expect(EVENT_NAMES.length).toBe(PAGE_NAMES.length + FEATURE_EVENTS.length);
    expect(EVENT_NAMES.length).toBeGreaterThanOrEqual(40);
    expect(EVENT_NAMES.length).toBeLessThanOrEqual(90);
  });

  it("derives page events from the page list", () => {
    for (const page of PAGE_NAMES) expect(isEventName(`page:${page}`)).toBe(true);
    expect(isEventName("page:not-a-screen")).toBe(false);
    expect(isPageName("not-a-screen")).toBe(false);
  });

  it("contains the actions the app reports today", () => {
    for (const name of ["install:start", "install:done", "profile:apply", "repair:apply", "launch:modded", "launch:vanilla"]) {
      expect(isEventName(name)).toBe(true);
    }
  });
});

describe("event parsing", () => {
  const now = 1_700_000_000_000;

  function body(events: readonly unknown[]): Record<string, unknown> {
    return { installId: INSTALL_ID, version: "0.1.0", os: "linux", events };
  }

  it("normalizes the install id to lowercase", () => {
    const parsed = parseEvents(body([{ name: "page:browse", page: "browse", ts: now }]), now);
    expect(parsed.ok && parsed.value.installId).toBe(INSTALL_ID);
  });

  it("clamps a future timestamp to the server clock", () => {
    const parsed = parseEvents(body([{ name: "page:browse", ts: now + 60_000 }]), now);
    expect(parsed.ok && parsed.value.events[0]?.ts).toBe(now);
  });

  it("drops an unknown page but keeps the event", () => {
    const parsed = parseEvents(body([{ name: "page:browse", page: "mod-detail", ts: now }]), now);
    expect(parsed.ok && parsed.value.events[0]?.page).toBe("");
  });

  it("accepts an event without a page", () => {
    const parsed = parseEvents(body([{ name: "install:done", ts: now }]), now);
    expect(parsed.ok && parsed.value.events[0]?.page).toBe("");
  });

  it("rejects a negative timestamp", () => {
    expect(parseEvents(body([{ name: "page:browse", ts: -1 }]), now).ok).toBe(false);
  });
});

describe("hash parsing", () => {
  const hash = "9F2C8D1B7A6E5F40312C8D1B7A6E5F40312C8D1B7A6E5F40312C8D1B7A6E5F40";

  it("normalizes an uppercase digest", () => {
    const parsed = parseHashes({ installId: crypto.randomUUID(), sha256: hash, size: 10, gamebananaModId: 1, fileId: 2 });
    expect(parsed.ok && parsed.value.sha256).toBe(hash.toLowerCase());
  });

  it("rejects a size expressed in exponent notation", () => {
    const query = new URLSearchParams({ sha256: hash.toLowerCase(), size: "1e6" });
    expect(parseIdentify(query).ok).toBe(false);
  });

  it("rejects a version that is not semver", () => {
    expect(isSemver("1.0")).toBe(false);
    expect(isSemver("v1.0.0")).toBe(false);
    expect(isSemver("1.0.0-rc.1")).toBe(true);
    expect(isSemver("1.0.0+build.5")).toBe(true);
  });
});

describe("scrubbing", () => {
  const cases: ReadonlyArray<readonly [string, string]> = [
    ["/Users/alice/sigil/main.rs", "/Users/<user>/sigil/main.rs"],
    ["/home/bob/.local/share/sigil", "/home/<user>/.local/share/sigil"],
    ["C:\\Users\\Alice\\AppData\\Local\\sigil", "C:\\Users\\<user>\\AppData\\Local\\sigil"],
    ["76561198012345678", "<steamid64>"],
    ["alice@example.com", "<email>"],
    ["sk-ant-api03-abcdefghijklmnopqrstuvwxyz", "<key>"],
    ["ghp_abcdefghijklmnopqrstuvwxyz0123456789", "<key>"],
    ["Bearer abcdefghijklmnop", "Bearer <token>"],
    ["api_key=abcdef123456", "api_key=<redacted>"],
    ["3f2504e0-4f89-41d3-9a0c-0305e82c3301", "<uuid>"],
  ];

  for (const [input, expected] of cases) {
    it(`scrubs ${input.slice(0, 24)}`, () => {
      expect(scrub(input)).toBe(expected);
    });
  }

  it("leaves ordinary text alone", () => {
    expect(scrub("Error: pak01_dir.vpk was not found")).toBe("Error: pak01_dir.vpk was not found");
  });
});

describe("crash fingerprints", () => {
  it("removes addresses, line numbers and columns from frames", () => {
    const frames = fingerprintFrames(
      "Error: boom\n  at load (/Users/alice/x.vmat:12:3)\n  at run (0x7ff8ab12cd34)\n",
    );

    expect(frames).toEqual(["Error: boom", "at load (/Users/alice/x.vmat)", "at run (0x)"]);
  });

  it("keeps five frames at most", () => {
    const trace = Array.from({ length: 9 }, (_, index) => `  at frame${index}`).join("\n");
    expect(fingerprintFrames(trace)).toHaveLength(5);
  });

  it("is stable across a rebuild that moves code", async () => {
    const first = await crashFingerprint("Error: boom", "  at load (/Users/<user>/a.rs:12:3)\n  at run (0x1)");
    const second = await crashFingerprint("Error: boom", "  at load (/Users/<user>/a.rs:99:7)\n  at run (0x2)");
    expect(first).toBe(second);
  });

  it("separates different messages", async () => {
    const first = await crashFingerprint("Error: one", "  at run");
    const second = await crashFingerprint("Error: two", "  at run");
    expect(first).not.toBe(second);
  });
});

describe("support codes", () => {
  it("uses only unambiguous characters", () => {
    for (let i = 0; i < 200; i += 1) {
      const code = diagnosticsCode();
      expect(code).toMatch(CODE_PATTERN);
      expect(code).not.toMatch(/[01IO]/);
    }
  });
});

describe("hashing", () => {
  it("matches the known SHA-256 of abc", async () => {
    expect(await sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });
});

