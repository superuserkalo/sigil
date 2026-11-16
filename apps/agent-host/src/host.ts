import {
  FencedError,
  FileStorage,
  ProviderError,
  ResumeError,
  RunError,
  Session,
  SessionBusyError,
  SessionCorruptionError,
  SessionFailedError,
  StorageError,
} from "tiny-agent-sdk";
import type { Agent, AgentEvent, PendingPause, Usage } from "tiny-agent-sdk";

const SESSION_ID = /^s_[a-zA-Z0-9_-]{1,100}$/;

export class HostError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "HostError";
    this.code = code;
  }
}

interface ActiveRun {
  controller: AbortController;
  runId: string | null;
  done: Promise<void>;
  finish: () => void;
}

interface ResultParts {
  text: string;
  pause?: PendingPause | null;
  usage?: Usage | null;
}

export interface HostOptions {
  agent: Agent;
  sessionDir: string;
  send: (value: unknown) => void;
  log: (line: string) => void;
  onShutdown: () => void;
}

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolvePromise: () => void = () => {};
  const promise = new Promise<void>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}

function isEnoent(error: unknown): boolean {
  return error instanceof Error && (error as { code?: unknown }).code === "ENOENT";
}

function truncated(value: string): string {
  return value.length <= 4000 ? value : `${value.slice(0, 4000)}...`;
}

function describeError(error: unknown): { code: string; message: string } {
  const message = truncated(error instanceof Error ? error.message : String(error));

  if (error instanceof HostError) return { code: error.code, message };
  if (error instanceof ResumeError) return { code: error.code, message };
  if (error instanceof SessionBusyError) return { code: "busy", message };
  if (error instanceof SessionFailedError) return { code: "session_failed", message };
  if (error instanceof SessionCorruptionError) return { code: "session_corrupt", message };
  if (error instanceof FencedError) return { code: "session_fenced", message };
  if (error instanceof StorageError) return { code: "storage_error", message };
  if (error instanceof ProviderError) return { code: "provider_error", message };
  if (error instanceof RunError) return { code: error.code, message };
  if (error instanceof Error && error.name === "AbortError") return { code: "aborted", message: "Run aborted." };
  if (isEnoent(error)) return { code: "session_not_found", message };
  return { code: "internal_error", message };
}

function isSessionFatal(code: string): boolean {
  return (
    code === "session_failed" ||
    code === "storage_error" ||
    code === "session_fenced" ||
    code === "session_corrupt"
  );
}

/**
 * One TinyAgent session at a time, one active run at a time. The host is the
 * only writer of its session directory.
 */
export class AgentHost {
  private readonly agent: Agent;
  private readonly storage: FileStorage;
  private readonly send: (value: unknown) => void;
  private readonly log: (line: string) => void;
  private readonly onShutdown: () => void;
  private current: Session | undefined;
  private active: ActiveRun | undefined;
  private switching = false;
  private closed = false;

  constructor(options: HostOptions) {
    this.agent = options.agent;
    this.storage = new FileStorage(options.sessionDir);
    this.send = options.send;
    this.log = options.log;
    this.onShutdown = options.onShutdown;
  }

  async handle(command: Record<string, unknown>): Promise<void> {
    const rawId = command.id;
    const id = typeof rawId === "string" ? rawId : null;
    const type = typeof command.type === "string" ? command.type : "unknown";

    try {
      if (rawId !== undefined && typeof rawId !== "string") {
        throw new HostError("invalid_command", "id must be a string.");
      }

      switch (type) {
        case "prompt":
          await this.prompt(id, command);
          return;
        case "resume":
          await this.resume(id, command);
          return;
        case "abort":
          this.abort(id);
          return;
        case "state":
          this.state(id);
          return;
        case "list_sessions":
          this.listSessions(id);
          return;
        case "new_session":
          await this.newSession(id);
          return;
        case "shutdown":
          await this.shutdownCommand(id);
          return;
        default:
          throw new HostError("invalid_command", `Unknown command type "${type}".`);
      }
    } catch (error) {
      const described = describeError(error);
      if (isSessionFatal(described.code)) await this.discard();
      this.send({ id, ok: false, error: described });
      this.log(`command ${type} failed: ${described.code}: ${described.message}`);
    }
  }

  /** Abort any run and release the journal lock. Safe to call more than once. */
  async shutdown(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    await this.stopRun();
    await this.closeCurrent();
  }

  private async prompt(id: string | null, command: Record<string, unknown>): Promise<void> {
    const text = command.text;
    if (typeof text !== "string" || text.trim().length === 0) {
      throw new HostError("invalid_command", "prompt needs a non-empty text string.");
    }
    const requested = this.requestedSession(command.sessionId);
    const run = this.beginRun();

    try {
      const session = await this.open(requested);
      this.refuseOpenRun(session);

      if (run.controller.signal.aborted) {
        throw new HostError("aborted", "Run aborted before it started.");
      }

      const result = await session.run(text, {
        signal: run.controller.signal,
        onEvent: (event) => this.recordEvent(run, event),
      });

      this.sendResult(id, session, result);
    } finally {
      this.finishRun(run);
    }
  }

  private async resume(id: string | null, command: Record<string, unknown>): Promise<void> {
    const requested = this.requestedSession(command.sessionId);
    const hasResult = Object.prototype.hasOwnProperty.call(command, "result");
    const hasError = Object.prototype.hasOwnProperty.call(command, "error");

    if (hasResult && hasError) {
      throw new HostError("invalid_command", "Send either result or error, not both.");
    }
    if (hasError && typeof command.error !== "string") {
      throw new HostError("invalid_command", "error must be a string.");
    }

    const run = this.beginRun();

    try {
      if (requested === undefined && this.current === undefined) {
        throw new HostError("nothing_to_resume", "The host has no open session to resume.");
      }

      const session = await this.open(requested);

      if (session.recoveryRequired) {
        throw new HostError(
          "recovery_required",
          "The session has an interrupted tool call that needs reconciliation. This host does not expose reconcile.",
        );
      }

      if (session.paused) {
        if (!hasResult && !hasError) {
          throw new HostError(
            "invalid_command",
            `The run is paused on ${session.paused.toolName}. Provide result or error.`,
          );
        }
      } else if (session.resumable) {
        if (hasResult || hasError) {
          throw new HostError(
            "invalid_command",
            "The session has no paused tool call, so result and error are not accepted.",
          );
        }
      } else {
        throw new HostError("nothing_to_resume", "The session has no interrupted or paused run.");
      }

      if (run.controller.signal.aborted) {
        throw new HostError("aborted", "Run aborted before it started.");
      }

      const result = await session.resume({
        signal: run.controller.signal,
        ...(hasResult ? { result: command.result } : {}),
        ...(hasError ? { error: command.error as string } : {}),
        onEvent: (event) => this.recordEvent(run, event),
      });

      this.sendResult(id, session, result);
    } finally {
      this.finishRun(run);
    }
  }

  private abort(id: string | null): void {
    const run = this.active;

    if (!run) {
      this.sendResult(id, this.current, { text: "", usage: this.current?.usage ?? null }, {
        aborted: false,
        runId: null,
      });
      return;
    }

    run.controller.abort();
    this.sendResult(id, this.current, { text: "", usage: this.current?.usage ?? null }, {
      aborted: true,
      runId: run.runId,
    });
  }

  private state(id: string | null): void {
    const session = this.current;
    const paused = session?.paused ?? null;
    const usage = session?.usage ?? null;

    const state = {
      status: this.active === undefined ? "idle" : "running",
      runId: this.active?.runId ?? null,
      sessionId: session?.id ?? null,
      messageCount: session?.history.length ?? 0,
      resumable: session?.resumable ?? false,
      paused,
      recoveryRequired: session?.recoveryRequired ?? false,
      usage,
    };

    this.send({
      id,
      ok: true,
      sessionId: session?.id ?? null,
      text: "",
      pause: paused,
      usage,
      state,
    });
  }

  private listSessions(id: string | null): void {
    const sessionIds = this.storage.list();
    const sessionId = this.current?.id ?? null;

    this.send({
      id,
      ok: true,
      sessionId,
      text: "",
      pause: null,
      usage: null,
      sessionIds,
    });
  }

  private async newSession(id: string | null): Promise<void> {
    if (this.closed) {
      throw new HostError("session_closed", "The host is shutting down.");
    }
    if (this.switching) {
      throw new HostError("busy", "A session change is already in progress.");
    }

    this.switching = true;
    try {
      await this.stopRun();
      const previousSessionId = this.current?.id ?? null;
      await this.closeCurrent();
      const session = await this.create();
      this.send({
        id,
        ok: true,
        sessionId: session.id,
        text: "",
        pause: null,
        usage: null,
        previousSessionId,
      });
    } finally {
      this.switching = false;
    }
  }

  private async shutdownCommand(id: string | null): Promise<void> {
    const sessionId = this.current?.id ?? null;
    try {
      await this.shutdown();
      this.send({
        id,
        ok: true,
        sessionId,
        text: "",
        pause: null,
        usage: null,
        stopped: true,
      });
    } finally {
      this.onShutdown();
    }
  }

  private beginRun(): ActiveRun {
    if (this.closed) {
      throw new HostError("session_closed", "The host is shutting down.");
    }
    if (this.switching) {
      throw new HostError("busy", "A session change is in progress.");
    }
    if (this.active) {
      throw new HostError("busy", "A run is already active. Send abort first.");
    }

    const controller = new AbortController();
    const pending = deferred();
    const run: ActiveRun = {
      controller,
      runId: null,
      done: pending.promise,
      finish: pending.resolve,
    };
    this.active = run;
    return run;
  }

  private finishRun(run: ActiveRun): void {
    if (this.active === run) this.active = undefined;
    run.finish();
  }

  private recordEvent(run: ActiveRun, event: AgentEvent): void {
    if (event.type === "agent_start") run.runId = event.runId;
    this.send({ event });
  }

  private sendResult(
    id: string | null,
    session: Session | undefined,
    parts: ResultParts,
    extra: Record<string, unknown> = {},
  ): void {
    this.send({
      id,
      ok: true,
      sessionId: session?.id ?? null,
      text: parts.text,
      pause: parts.pause ?? null,
      usage: parts.usage ?? null,
      ...extra,
    });
  }

  private refuseOpenRun(session: Session): void {
    if (session.recoveryRequired) {
      throw new HostError(
        "recovery_required",
        "The session has an interrupted tool call that needs reconciliation. This host does not expose reconcile.",
      );
    }
    if (session.paused) {
      throw new HostError(
        "resume_required",
        `Session ${session.id} is paused on ${session.paused.toolName}. Send resume with a result or error.`,
      );
    }
    if (session.resumable) {
      throw new HostError(
        "resume_required",
        `Session ${session.id} has an interrupted run. Send resume or new_session.`,
      );
    }
  }

  private requestedSession(value: unknown): string | undefined {
    if (value === undefined) return undefined;
    if (typeof value !== "string" || !SESSION_ID.test(value)) {
      throw new HostError("invalid_command", "sessionId must look like s_ followed by letters, digits, _ or -.");
    }
    return value;
  }

  private async create(): Promise<Session> {
    const session = new Session(this.agent, { storage: this.storage });
    await session.ready;
    this.current = session;
    return session;
  }

  private async open(sessionId: string | undefined): Promise<Session> {
    if (this.current && (sessionId === undefined || this.current.id === sessionId)) {
      return this.current;
    }

    if (this.current) await this.closeCurrent();

    let session: Session;
    try {
      session = new Session(this.agent, {
        storage: this.storage,
        ...(sessionId === undefined ? {} : { sessionId, resume: true }),
      });
      await session.ready;
    } catch (error) {
      if (isEnoent(error)) {
        throw new HostError("session_not_found", `No stored session ${sessionId ?? ""}.`);
      }
      throw error;
    }

    this.current = session;
    return session;
  }

  private async closeCurrent(): Promise<void> {
    const session = this.current;
    this.current = undefined;
    if (!session) return;

    try {
      await session.close();
    } catch (error) {
      this.log(`could not close session ${session.id}: ${describeError(error).message}`);
    }
  }

  private async discard(): Promise<void> {
    const session = this.current;
    this.current = undefined;
    if (!session) return;

    try {
      await session.close();
    } catch {
      // The journal is already unusable in this process. Reopening is the way out.
    }
  }

  private async stopRun(): Promise<void> {
    const run = this.active;
    if (!run) return;
    run.controller.abort();
    await run.done;
    // The interrupted command answers with its own error envelope. Let that
    // settle before a session change or shutdown writes its result.
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
}
