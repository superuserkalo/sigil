/**
 * GameBanana's public API v11, read politely: small pages, a pause before every call, the
 * identifying user agent "Sigil", and the run stops on the first error. Only public submission
 * data is read; nothing about a player is ever sent here.
 *
 * URL shapes follow `rust/crates/deadlock-core/src/gamebanana.rs`.
 */

import { Budget, BudgetSpent, RemoteError, type Net } from "./net";

export const GB_API = "https://gamebanana.com/apiv11";
export const GB_GAME_ID = 20948;
export const GB_USER_AGENT = "Sigil (mod catalog tags; github.com/superuserkalo/sigil)";
/** Pause before each GameBanana call. */
export const GB_DELAY_MS = 1200;
export const LIST_PERPAGE = 20;

export type Shot = {
  readonly full: string;
  /** The 530-wide variant, with its size, when GameBanana made one. */
  readonly small: { readonly url: string; readonly width: number; readonly height: number } | null;
};

export type ListedMod = {
  readonly id: number;
  readonly title: string;
  /** The newest of added, modified and updated, in seconds. */
  readonly updated: number;
  readonly added: number;
  readonly category: string;
  readonly subcategory: string;
  readonly submitterId: number;
  /** GameBanana's own content-rating flag. */
  readonly rated: boolean;
  readonly postCount: number;
};

export type ModFile = {
  readonly id: number;
  readonly name: string;
  readonly size: number;
  readonly note: string;
  readonly added: number;
};

export type Profile = {
  readonly summary: string;
  readonly text: string;
  /** Links in the description to other GameBanana mods: id and link text. */
  readonly links: readonly { readonly id: number; readonly label: string }[];
  readonly files: readonly ModFile[];
  readonly shots: readonly Shot[];
  readonly updatesCount: number;
};

export type ModUpdate = {
  readonly title: string;
  readonly text: string;
  readonly changes: readonly string[];
  readonly added: number;
};

export type ModComment = {
  /** GameBanana's member id, used only to count distinct commenters; never stored or sent. */
  readonly poster: number;
  readonly added: number;
  readonly text: string;
};

type Json = Record<string, unknown>;

export class GameBanana {
  private calls = 0;

  constructor(
    private readonly net: Net,
    private readonly budget: Budget,
  ) {}

  async get(url: string): Promise<unknown> {
    if (!this.budget.take()) throw new BudgetSpent("subrequest budget spent");
    if (this.calls > 0) await this.net.sleep(GB_DELAY_MS);
    this.calls += 1;
    const response = await this.net.fetch(url, {
      headers: { "user-agent": GB_USER_AGENT, accept: "application/json" },
    });
    if (!response.ok) {
      await response.body?.cancel();
      throw new RemoteError("gamebanana", response.status);
    }
    return response.json();
  }

  /** Bytes of a public screenshot, capped. Same politeness as the API. */
  async image(url: string, maxBytes: number): Promise<Uint8Array | null> {
    if (!url.startsWith("https://images.gamebanana.com/")) return null;
    if (!this.budget.take()) throw new BudgetSpent("subrequest budget spent");
    if (this.calls > 0) await this.net.sleep(GB_DELAY_MS);
    this.calls += 1;
    const response = await this.net.fetch(url, { headers: { "user-agent": GB_USER_AGENT } });
    if (!response.ok) {
      await response.body?.cancel();
      throw new RemoteError("gamebanana", response.status);
    }
    const length = Number(response.headers.get("content-length") ?? "0");
    if (length > maxBytes) {
      await response.body?.cancel();
      return null;
    }
    const bytes = new Uint8Array(await response.arrayBuffer());
    return bytes.byteLength > maxBytes ? null : bytes;
  }

  async listMods(sort: "Generic_LatestUpdated" | "Generic_Newest", page: number): Promise<ListedMod[]> {
    const url =
      `${GB_API}/Mod/Index?_nPage=${page}&_nPerpage=${LIST_PERPAGE}&_sSort=${sort}` +
      `&_aFilters%5BGeneric_Game%5D=${GB_GAME_ID}`;
    return parseList(await this.get(url));
  }

  /** Null when the page is gone, trashed, withheld or private. */
  async profile(id: number): Promise<Profile | null> {
    try {
      const body = await this.get(`${GB_API}/Mod/${id}/ProfilePage`);
      if (isObject(body) && (body._bIsTrashed === true || body._bIsWithheld === true || body._bIsPrivate === true)) return null;
      return parseProfile(body);
    } catch (error) {
      if (error instanceof RemoteError && (error.status === 404 || error.status === 410)) return null;
      throw error;
    }
  }

  async updates(id: number, perpage: number): Promise<ModUpdate[]> {
    return parseUpdates(await this.get(`${GB_API}/Mod/${id}/Updates?_nPerpage=${perpage}&_nPage=1`));
  }

  async comments(id: number, perpage: number): Promise<ModComment[]> {
    return parseComments(await this.get(`${GB_API}/Mod/${id}/Posts?_nPerpage=${perpage}&_nPage=1`));
  }

  /** Hero names: the children of the "Skins" root category, read at run time. */
  async heroes(): Promise<string[]> {
    const roots = asArray(await this.get(`${GB_API}/Mod/Categories?_idGameRow=${GB_GAME_ID}&_sSort=a_to_z&_bShowEmpty=true`));
    const skins = roots.find((root) => str(root, "_sName").toLowerCase() === "skins");
    if (skins === undefined) return [];
    const children = asArray(
      await this.get(`${GB_API}/Mod/Categories?_idCategoryRow=${num(skins, "_idRow")}&_sSort=a_to_z&_bShowEmpty=true`),
    );
    return children.map((child) => decodeEntities(str(child, "_sName")).trim()).filter((name) => name.length > 0);
  }
}

export function parseList(body: unknown): ListedMod[] {
  const records = asArray(isObject(body) ? body._aRecords : body);
  return records
    .filter((record) => str(record, "_sModelName") === "Mod" || str(record, "_sModelName") === "")
    .map((record) => ({
      id: num(record, "_idRow"),
      title: decodeEntities(str(record, "_sName")).trim(),
      updated: Math.max(num(record, "_tsDateAdded"), num(record, "_tsDateModified"), num(record, "_tsDateUpdated")),
      added: num(record, "_tsDateAdded"),
      category: decodeEntities(str(obj(record, "_aRootCategory"), "_sName")),
      subcategory: decodeEntities(str(obj(record, "_aSubCategory"), "_sName")),
      submitterId: num(obj(record, "_aSubmitter"), "_idRow"),
      rated: record._bHasContentRatings === true,
      postCount: num(record, "_nPostCount"),
    }))
    .filter((mod) => mod.id > 0 && mod.title.length > 0);
}

export function parseProfile(body: unknown): Profile {
  const value = isObject(body) ? body : {};
  const html = str(value, "_sText");
  return {
    summary: decodeEntities(str(value, "_sDescription")).trim(),
    text: htmlToText(html),
    links: modLinks(html),
    files: asArray(value._aFiles)
      .filter((file) => file._bIsArchived !== true)
      .map((file) => ({
        id: num(file, "_idRow"),
        name: str(file, "_sFile"),
        size: num(file, "_nFilesize"),
        note: decodeEntities(str(file, "_sDescription")).trim(),
        added: num(file, "_tsDateAdded"),
      }))
      .filter((file) => file.id > 0),
    shots: shotsOf(obj(value, "_aPreviewMedia")),
    updatesCount: num(value, "_nUpdatesCount"),
  };
}

export function parseUpdates(body: unknown): ModUpdate[] {
  return asArray(isObject(body) ? body._aRecords : body).map((record) => ({
    title: decodeEntities(str(record, "_sName") || str(record, "_sTitle")).trim(),
    text: htmlToText(str(record, "_sText")),
    changes: asArray(record._aChangeLog)
      .map((line) => decodeEntities(`${str(line, "cat")}: ${str(line, "text")}`).trim())
      .filter((line) => line.length > 2),
    added: num(record, "_tsDateAdded"),
  }));
}

export function parseComments(body: unknown): ModComment[] {
  return asArray(isObject(body) ? body._aRecords : body)
    .map((record) => ({
      poster: num(obj(record, "_aPoster"), "_idRow"),
      added: num(record, "_tsDateAdded"),
      text: htmlToText(str(record, "_sText")),
    }))
    .filter((comment) => comment.text.length > 0);
}

function shotsOf(media: Json): Shot[] {
  return asArray(media._aImages)
    .filter((image) => str(image, "_sType") === "screenshot" || str(image, "_sType") === "")
    .map((image) => {
      const base = str(image, "_sBaseUrl").replace(/\/+$/, "");
      const file = str(image, "_sFile");
      const small = str(image, "_sFile530");
      return {
        full: `${base}/${file}`,
        small:
          small.length > 0
            ? { url: `${base}/${small}`, width: num(image, "_wFile530"), height: num(image, "_hFile530") }
            : null,
      };
    })
    .filter((shot) => !shot.full.endsWith("/"));
}

/** Untrusted HTML is cut to this many characters before any scanning. */
export const MAX_HTML = 20_000;

/**
 * Links to other GameBanana mods, by a linear scan (no backtracking regex over untrusted HTML).
 * Each `<a` opens a window of at most 600 characters to its `</a>`.
 */
export function modLinks(raw: string): { id: number; label: string }[] {
  const html = raw.slice(0, MAX_HTML);
  const lower = html.toLowerCase();
  const links: { id: number; label: string }[] = [];
  let at = 0;
  // Remembered positions keep a run of unclosed "<a" linear.
  let tagEnd = -1;
  let close = -1;
  while (links.length < 8) {
    const open = lower.indexOf("<a", at);
    if (open < 0) break;
    at = open + 2;
    if (tagEnd <= open) tagEnd = lower.indexOf(">", open);
    if (tagEnd < 0) break;
    if (tagEnd - open > 600) continue;
    if (close <= tagEnd) close = lower.indexOf("</a>", tagEnd);
    if (close < 0) break;
    if (close - tagEnd > 600) continue;
    const tag = lower.slice(open, tagEnd);
    const marker = tag.indexOf("gamebanana.com/mods/");
    if (marker < 0) continue;
    let digits = "";
    for (let i = marker + 20; i < tag.length && digits.length < 10; i += 1) {
      const char = tag[i] ?? "";
      if (char < "0" || char > "9") break;
      digits += char;
    }
    const id = Number(digits);
    if (!(id > 0)) continue;
    const label = htmlToText(html.slice(tagEnd + 1, close)).slice(0, 80) || `mod ${id}`;
    if (!links.some((link) => link.id === id)) links.push({ id, label });
    at = close + 4;
  }
  return links;
}

const BREAK_TAGS = new Set(["br", "p", "li", "div", "h1", "h2", "h3", "h4", "h5", "h6"]);

/** Tags dropped by a linear scan, breaks kept as new lines, entities decoded, spaces collapsed. */
export function htmlToText(raw: string): string {
  const html = raw.slice(0, MAX_HTML);
  let out = "";
  let at = 0;
  // The next ">" is remembered, so a run of "<" without one stays linear.
  let close = -1;
  while (at < html.length) {
    const open = html.indexOf("<", at);
    if (open < 0) {
      out += html.slice(at);
      break;
    }
    out += html.slice(at, open);
    if (close <= open) close = html.indexOf(">", open);
    if (close < 0) {
      out += html.slice(open);
      break;
    }
    // A very long "tag" is text, not markup.
    if (close - open > 400) {
      out += "<";
      at = open + 1;
      continue;
    }
    const name = (html.slice(open + 1, close).replace(/^\//, "").split(/[\s/]/)[0] ?? "").toLowerCase();
    if (BREAK_TAGS.has(name)) out += "\n";
    at = close + 1;
  }
  return decodeEntities(out.replace(/\[\/?[a-z]{1,10}\]/gi, " "))
    .split("\n")
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => line.length > 0)
    .join("\n");
}

function codePoint(value: number): string {
  if (!Number.isInteger(value) || value < 0 || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return "\ufffd";
  return String.fromCodePoint(value);
}

export function decodeEntities(text: string): string {
  return text
    .replace(/&nbsp;/g, " ")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#0?39;|&apos;/g, "'")
    .replace(/&#[xX]([0-9a-fA-F]{1,8});/g, (_, hex: string) => codePoint(parseInt(hex, 16)))
    .replace(/&#(\d{1,10});/g, (_, code: string) => codePoint(Number(code)))
    .replace(/&amp;/g, "&");
}

function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asArray(value: unknown): Json[] {
  return Array.isArray(value) ? value.filter(isObject) : [];
}

function obj(value: Json, key: string): Json {
  const inner = value[key];
  return isObject(inner) ? inner : {};
}

function str(value: Json, key: string): string {
  const inner = value[key];
  return typeof inner === "string" ? inner : "";
}

function num(value: Json, key: string): number {
  const inner = value[key];
  const parsed = typeof inner === "number" ? inner : typeof inner === "string" ? Number(inner) : 0;
  return Number.isFinite(parsed) ? parsed : 0;
}
