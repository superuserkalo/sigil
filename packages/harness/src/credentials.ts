import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Credential, CredentialInfo, CredentialStore } from "@earendil-works/pi-ai";

export class JsonFileCredentialStore implements CredentialStore {
  readonly #path: string;
  #queue: Promise<void> = Promise.resolve();

  constructor(path: string) {
    this.#path = path;
  }

  async read(providerId: string): Promise<Credential | undefined> {
    const all = await this.#readAll();
    return all[providerId];
  }

  async list(): Promise<readonly CredentialInfo[]> {
    const all = await this.#readAll();
    return Object.entries(all).map(([providerId, credential]) => ({
      providerId,
      type: credential.type,
    }));
  }

  async modify(
    providerId: string,
    change: (current: Credential | undefined) => Promise<Credential | undefined>,
  ): Promise<Credential | undefined> {
    return this.#serialize(async () => {
      const all = await this.#readAll();
      const current = all[providerId];
      const next = await change(current);

      if (next === undefined) return current;

      all[providerId] = next;
      await this.#writeAll(all);
      return next;
    });
  }

  async delete(providerId: string): Promise<void> {
    await this.#serialize(async () => {
      const all = await this.#readAll();
      if (!Object.hasOwn(all, providerId)) return;

      delete all[providerId];
      await this.#writeAll(all);
    });
  }

  #serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.#queue.then(operation, operation);
    this.#queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  async #readAll(): Promise<Record<string, Credential>> {
    let text: string;

    try {
      text = await readFile(this.#path, "utf8");
    } catch (error) {
      if (isNotFound(error)) return {};
      throw error;
    }

    const parsed: unknown = JSON.parse(text);

    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error(`Credential store at ${this.#path} is not a JSON object`);
    }

    return parsed as Record<string, Credential>;
  }

  async #writeAll(all: Record<string, Credential>): Promise<void> {
    const directory = dirname(this.#path);
    await mkdir(directory, { recursive: true });

    const temporary = `${this.#path}.${process.pid}.tmp`;
    await writeFile(temporary, `${JSON.stringify(all, null, 2)}\n`, { mode: 0o600 });
    await rename(temporary, this.#path);

    if (process.platform !== "win32") await chmod(this.#path, 0o600);
  }
}

function isNotFound(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}
