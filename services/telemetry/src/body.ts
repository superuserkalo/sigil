/**
 * Reading a request body without trusting its size.
 *
 * `request.text()` buffers whatever a client sends, so a broken or hostile client could make the
 * Worker hold a large body in memory before any limit applies. The reader below stops as soon as
 * the byte count passes the endpoint limit, using the declared `content-length` only as an early
 * hint.
 */

/** A body that is exactly `limit` bytes is accepted; one byte more is not. */
export type CappedBody =
  | { readonly ok: true; readonly text: string }
  | { readonly ok: false; readonly reason: "too_large" | "invalid_utf8" };

export type JsonBody =
  | { readonly ok: true; readonly value: unknown }
  | { readonly ok: false; readonly reason: "too_large" | "invalid_utf8" | "invalid_json" };

const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });

function declaredLength(request: Request): number | null {
  const header = request.headers.get("content-length");
  if (header === null) return null;
  if (!/^\d{1,10}$/.test(header)) return null;
  return Number(header);
}

export async function readCappedText(request: Request, limit: number): Promise<CappedBody> {
  const declared = declaredLength(request);
  if (declared !== null && declared > limit) {
    await request.body?.cancel();
    return { ok: false, reason: "too_large" };
  }

  const stream = request.body;
  if (stream === null) return { ok: true, text: "" };

  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;

  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (value === undefined) continue;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel();
      return { ok: false, reason: "too_large" };
    }
    chunks.push(value);
  }

  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }

  try {
    return { ok: true, text: decoder.decode(bytes) };
  } catch {
    return { ok: false, reason: "invalid_utf8" };
  }
}

export async function readJsonBody(request: Request, limit: number): Promise<JsonBody> {
  const body = await readCappedText(request, limit);
  if (!body.ok) return body;
  if (body.text.length === 0) return { ok: false, reason: "invalid_json" };
  try {
    const value: unknown = JSON.parse(body.text);
    return { ok: true, value };
  } catch {
    return { ok: false, reason: "invalid_json" };
  }
}
