import { describe, expect, it } from "vitest";
import { call, callStream, eventsBody, readJson } from "./helpers";

type ErrorBody = { readonly error: string };
type EventsResponse = { readonly accepted: number; readonly dropped: number };

function event(name: string): Record<string, unknown> {
  return { name, ts: Date.now() };
}

describe("client headers", () => {
  it("rejects a request without x-sigil-client", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([event("page:browse")]),
      headers: { "x-sigil-client": null },
    });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_client");
  });

  it("rejects a request with the wrong client marker", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([event("page:browse")]),
      headers: { "x-sigil-client": "2" },
    });

    expect(response.status).toBe(400);
  });

  it("rejects a missing version", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([event("page:browse")]),
      headers: { "x-sigil-version": null },
    });

    expect(response.status).toBe(400);
  });

  it("rejects a version that is not semver", async () => {
    for (const version of ["0.1", "v0.1.0", "0.1.0.0", ""]) {
      const response = await call("/v1/events", {
        body: eventsBody([event("page:browse")]),
        headers: { "x-sigil-version": version },
      });
      expect(response.status).toBe(400);
    }
  });

  it("accepts a pre-release version", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([event("page:browse")]),
      headers: { "x-sigil-version": "0.2.0-beta.3" },
    });

    expect(response.status).toBe(202);
  });

  it("requires the headers on identify too", async () => {
    const response = await call("/v1/identify?sha256=abc&size=1", {
      method: "GET",
      headers: { "x-sigil-client": null },
    });

    expect(response.status).toBe(400);
  });
});

describe("routing", () => {
  it("answers 404 for an unknown path", async () => {
    const response = await call("/v1/nope", { body: {} });

    expect(response.status).toBe(404);
    expect((await readJson<ErrorBody>(response)).error).toBe("not_found");
  });

  it("answers 404 for the root path", async () => {
    const response = await call("/", { method: "GET" });

    expect(response.status).toBe(404);
  });

  it("answers 405 with the allowed method", async () => {
    const response = await call("/v1/events", { method: "GET" });

    expect(response.status).toBe(405);
    expect(response.headers.get("allow")).toBe("POST");
    expect((await readJson<ErrorBody>(response)).error).toBe("method_not_allowed");
  });

  it("keeps responses out of caches", async () => {
    const response = await call("/v1/events", { body: eventsBody([event("page:browse")]) });

    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("x-content-type-options")).toBe("nosniff");
  });
});

describe("body limits", () => {
  it("rejects a malformed JSON body", async () => {
    const response = await call("/v1/events", { rawBody: "{" });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("invalid_json");
  });

  it("rejects an empty body", async () => {
    const response = await call("/v1/events", { rawBody: "" });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("invalid_json");
  });

  it("rejects an events body over 32 KiB before parsing", async () => {
    const response = await call("/v1/events", {
      rawBody: JSON.stringify(eventsBody([event("page:browse")], { padding: "x".repeat(33 * 1024) })),
    });

    expect(response.status).toBe(413);
    expect((await readJson<ErrorBody>(response)).error).toBe("body_too_large");
  });

  it("rejects a JSON value that is not an object", async () => {
    const response = await call("/v1/events", { rawBody: "[]" });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_body");
  });

  it("stops a streamed body at the cap without a content-length header", async () => {
    const encoder = new TextEncoder();
    const chunks = Array.from({ length: 64 }, () => encoder.encode("x".repeat(1024)));
    const response = await callStream("/v1/events", chunks);

    expect(response.status).toBe(413);
    expect((await readJson<ErrorBody>(response)).error).toBe("body_too_large");
  });

  it("rejects a body that is not valid UTF-8", async () => {
    const response = await callStream("/v1/events", [new Uint8Array([0x7b, 0xff, 0x7d])]);

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("invalid_utf8");
  });
});

describe("usage events", () => {
  it("accepts a known event and counts it", async () => {
    const response = await call("/v1/events", { body: eventsBody([event("page:browse")]) });

    expect(response.status).toBe(202);
    expect(await readJson<EventsResponse>(response)).toEqual({ accepted: 1, dropped: 0 });
  });

  it("drops event names outside the allowlist instead of storing them", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([
        event("page:browse"),
        event("page:definitely-not-a-screen"),
        event("install:done"),
        event("player_name=alice"),
      ]),
    });

    expect(response.status).toBe(202);
    expect(await readJson<EventsResponse>(response)).toEqual({ accepted: 2, dropped: 2 });
  });

  it("accepts exactly 50 events", async () => {
    const events = Array.from({ length: 50 }, () => event("page:browse"));
    const response = await call("/v1/events", { body: eventsBody(events) });

    expect(response.status).toBe(202);
    expect(await readJson<EventsResponse>(response)).toEqual({ accepted: 50, dropped: 0 });
  });

  it("rejects 51 events", async () => {
    const events = Array.from({ length: 51 }, () => event("page:browse"));
    const response = await call("/v1/events", { body: eventsBody(events) });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_event_count");
  });

  it("rejects an install id that is not a random uuid", async () => {
    for (const installId of ["alice-laptop", "76561198000000000", "3f2504e0-4f89-41d3-9a0c"]) {
      const response = await call("/v1/events", {
        body: eventsBody([event("page:browse")], { installId }),
      });
      expect(response.status).toBe(400);
      expect((await readJson<ErrorBody>(response)).error).toBe("bad_install_id");
    }
  });

  it("rejects an unknown operating system", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([event("page:browse")], { os: "Solaris" }),
    });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_os");
  });

  it("rejects an event without a timestamp", async () => {
    const response = await call("/v1/events", {
      body: eventsBody([{ name: "page:browse" }]),
    });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_event");
  });

  it("rejects an empty event list", async () => {
    const response = await call("/v1/events", { body: eventsBody([]) });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_event_count");
  });
});
