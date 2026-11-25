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
export const LIST_PERPAGE = 50;

export type ListedMod = {
  readonly id: number;
  readonly title: string;
  /** The newest of added, modified and updated, in seconds. */
  readonly updated: number;
  readonly added: number;
  readonly category: string;
  readonly subcategory: string;
  readonly author: string;
  /** `_nLikeCount`: changes often, so it never enters the content hash. */
  readonly likes: number;
  /** `_bHasContentRatings`: the page carries content ratings. */
  readonly rated: boolean;
};

export type ModFile = {
  readonly id: number;
  readonly name: string;
  readonly size: number;
  readonly note: string;
};

/** The public text of one mod page, read in batches of up to 50 through `Mod/Multi`. */
export type ModText = {
  readonly id: number;
  readonly summary: string;
  readonly text: string;
  readonly files: readonly ModFile[];
};

type Json = Record<string, unknown>;

/** Ids per `Mod/Multi` request. */
export const MULTI_IDS = 50;

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

  async listMods(sort: "Generic_LatestUpdated" | "Generic_Newest", page: number): Promise<ListedMod[]> {
    const url =
      `${GB_API}/Mod/Index?_nPage=${page}&_nPerpage=${LIST_PERPAGE}&_sSort=${sort}` +
      `&_aFilters%5BGeneric_Game%5D=${GB_GAME_ID}`;
    return parseList(await this.get(url));
  }

  /**
   * Public text and files of up to 50 mods in one request. A mod that is missing from the answer,
   * or trashed, withheld or private, is in `gone`.
   */
  async texts(ids: readonly number[]): Promise<{ texts: ModText[]; gone: number[] }> {
    const chunk = ids.slice(0, MULTI_IDS);
    if (chunk.length === 0) return { texts: [], gone: [] };
    const url =
      `${GB_API}/Mod/Multi?_csvRowIds=${chunk.join(",")}` +
      "&_csvProperties=_idRow,_sDescription,_sText,_aFiles,_bIsTrashed,_bIsWithheld,_bIsPrivate";
    const rows = asArray(await this.get(url));
    const texts: ModText[] = [];
    for (const row of rows) {
      const id = num(row, "_idRow");
      if (!chunk.includes(id) || row._bIsTrashed === true || row._bIsWithheld === true || row._bIsPrivate === true) continue;
      texts.push(parseText(id, row));
    }
    const seen = new Set(texts.map((text) => text.id));
    return { texts, gone: chunk.filter((id) => !seen.has(id)) };
  }
}

export function parseList(body: unknown): ListedMod[] {
  const records = asArray(isObject(body) ? body._aRecords : body);
  return records
    .filter((record) => str(record, "_sModelName") === "Mod" || str(record, "_sModelName") === "")
    .map((record) => ({
      id: num(record, "_idRow"),
      title: decodeEntities(str(record, "_sName")).trim().slice(0, 200),
      updated: Math.max(num(record, "_tsDateAdded"), num(record, "_tsDateModified"), num(record, "_tsDateUpdated")),
      added: num(record, "_tsDateAdded"),
      category: decodeEntities(str(obj(record, "_aRootCategory"), "_sName")).slice(0, 80),
      subcategory: decodeEntities(str(obj(record, "_aSubCategory"), "_sName")).slice(0, 80),
      author: decodeEntities(str(obj(record, "_aSubmitter"), "_sName")).trim().slice(0, 60),
      likes: Math.max(0, Math.floor(num(record, "_nLikeCount"))),
      rated: record._bHasContentRatings === true,
    }))
    .filter((mod) => mod.id > 0 && mod.title.length > 0);
}

export function parseText(id: number, value: Json): ModText {
  return {
    id,
    summary: decodeEntities(str(value, "_sDescription")).trim().slice(0, 300),
    text: htmlToText(str(value, "_sText")).slice(0, 4000),
    files: asArray(value._aFiles)
      .filter((file) => file._bIsArchived !== true)
      .map((file) => ({
        id: num(file, "_idRow"),
        name: str(file, "_sFile").slice(0, 120),
        size: num(file, "_nFilesize"),
        note: decodeEntities(str(file, "_sDescription")).trim().slice(0, 200),
      }))
      .filter((file) => file.id > 0)
      .slice(0, 20),
  };
}

/** Untrusted HTML is cut to this many characters before any scanning. */
export const MAX_HTML = 20_000;

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
