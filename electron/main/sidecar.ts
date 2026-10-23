import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { EventEmitter } from "node:events";
import { realpathSync } from "node:fs";
import { createInterface } from "node:readline";
import { DesktopCommandError, type BackendStatus } from "../shared/contracts";

const PROTOCOL_VERSION = 1;
const MAX_FRAME_BYTES = 32 * 1024 * 1024;
const READY_TIMEOUT_MS = 15_000;
const REQUEST_TIMEOUT_MS = 30_000;
// Chat turns return as soon as they are queued and stream progress as events. These plugin
// methods resolve only when the model or agent task finishes, which can take many model
// requests; they get no client deadline. The sidecar bounds each model request, plugins can
// stop or cancel a run, and a sidecar exit rejects every pending request.
const LONG_RUNNING_PLUGIN_METHODS = new Set([
  "agent.run",
  "agent.task.resume",
  "agent.task.retry",
  "agent.task.start",
  "model.invoke",
]);

type ProtocolError = { code: string; message: string };
type IncomingFrame =
  | { kind: "ready"; protocolVersion: number; workspaceRoot: string }
  | { kind: "response"; id: string; ok: boolean; result?: unknown; error?: ProtocolError }
  | { kind: "event"; event: string; payload: unknown }
  | { kind: "fatal"; error: ProtocolError };

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timeout: NodeJS.Timeout | undefined;
  /** A plugin model/agent run that resolves only when the run ends. */
  longRunning: boolean;
};

export class SidecarClient extends EventEmitter {
  private child: ChildProcessWithoutNullStreams | null = null;
  private pending = new Map<string, PendingRequest>();
  private nextRequestId = 1;
  private ready = false;
  private stopping = false;
  private readonly executable: string;
  readonly workspacePath: string;

  constructor(executable: string, workspacePath: string) {
    super();
    this.executable = executable;
    this.workspacePath = realpathSync(workspacePath);
  }

  async start(): Promise<void> {
    if (this.child) return;
    this.stopping = false;
    this.emitStatus("starting", "Starting local agent runtime…", false);

    const child = spawn(this.executable, ["--project-root", this.workspacePath], {
      cwd: this.workspacePath,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child = child;

    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      const line = chunk.trim();
      if (line) console.error(`[daily-app-server:${this.workspacePath}] ${line}`);
    });

    const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
    lines.on("line", (line) => this.handleLine(line));

    child.once("exit", (code, signal) => {
      this.child = null;
      this.ready = false;
      const reason = `Local agent runtime exited${code == null ? "" : ` with code ${code}`}${signal ? ` (${signal})` : ""}.`;
      this.rejectPending(new Error(reason));
      this.emitStatus(this.stopping ? "stopped" : "failed", reason, !this.stopping);
      this.emit("exit", { expected: this.stopping, code, signal });
    });
    child.once("error", (error) => {
      this.emit("startup-error", error);
    });

    try {
      await new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => {
          cleanup();
          reject(new Error("Local agent runtime did not become ready in time."));
        }, READY_TIMEOUT_MS);
        const onReady = () => {
          cleanup();
          resolve();
        };
        const onError = (error: Error) => {
          cleanup();
          reject(error);
        };
        const cleanup = () => {
          clearTimeout(timeout);
          this.off("ready", onReady);
          this.off("startup-error", onError);
          this.off("fatal", onError);
        };
        this.once("ready", onReady);
        this.once("startup-error", onError);
        this.once("fatal", onError);
      });
    } catch (error) {
      this.forceStop();
      throw error;
    }
  }

  async invoke<T>(method: string, params: unknown = {}): Promise<T> {
    // Requests that arrive while the runtime is still starting (e.g. the renderer's initial snapshot)
    // wait for readiness instead of failing; the renderer may miss the "ready" status broadcast.
    if (this.child && !this.ready) await this.waitUntilReady();
    if (!this.child || !this.ready) {
      throw new Error("Local agent runtime is not ready.");
    }
    return this.send<T>(method, params);
  }

  private waitUntilReady(): Promise<void> {
    return new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timeout);
        this.off("ready", done);
        this.off("exit", done);
        this.off("fatal", done);
        this.off("startup-error", done);
        resolve();
      };
      const timeout = setTimeout(done, READY_TIMEOUT_MS);
      this.once("ready", done);
      this.once("exit", done);
      this.once("fatal", done);
      this.once("startup-error", done);
    });
  }

  private send<T>(method: string, params: unknown): Promise<T> {
    const id = String(this.nextRequestId++);
    const request = JSON.stringify({
      kind: "request",
      protocolVersion: PROTOCOL_VERSION,
      id,
      method,
      params,
    });
    if (Buffer.byteLength(request) > MAX_FRAME_BYTES) {
      return Promise.reject(new Error("Desktop command payload exceeds 32 MiB."));
    }
    return new Promise<T>((resolve, reject) => {
      const deadline = requestTimeout(method, params);
      const timeout = deadline === null ? undefined : setTimeout(() => {
        if (!this.pending.delete(id)) return;
        reject(new Error(`Desktop command ${method} timed out.`));
      }, deadline);
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout,
        longRunning: deadline === null,
      });
      this.child?.stdin.write(`${request}\n`, (error) => {
        if (!error) return;
        const pending = this.pending.get(id);
        if (pending) clearTimeout(pending.timeout);
        this.pending.delete(id);
        reject(error);
      });
    });
  }

  /** Plugin model and agent runs still in progress (each holds its request open until it ends). */
  activePluginRuns(): number {
    let count = 0;
    for (const request of this.pending.values()) if (request.longRunning) count += 1;
    return count;
  }

  async shutdown(): Promise<void> {
    if (!this.child) return;
    this.stopping = true;
    const child = this.child;
    try {
      if (this.ready) await this.invoke("shutdown");
    } catch {
      // The process may already be exiting.
    }
    if (!this.child) return;
    await Promise.race([
      new Promise<void>((resolve) => child.once("exit", () => resolve())),
      new Promise<void>((resolve) => setTimeout(resolve, 2_000)),
    ]);
    if (!this.child) return;
    if (this.child) child.kill("SIGTERM");
    await Promise.race([
      new Promise<void>((resolve) => child.once("exit", () => resolve())),
      new Promise<void>((resolve) => setTimeout(resolve, 1_000)),
    ]);
    if (this.child) child.kill("SIGKILL");
  }

  forceStop(): void {
    this.stopping = true;
    const child = this.child;
    if (!child) return;
    child.kill("SIGTERM");
    const killTimer = setTimeout(() => {
      if (this.child === child) child.kill("SIGKILL");
    }, 1_000);
    killTimer.unref();
  }

  private handleLine(line: string): void {
    if (Buffer.byteLength(line) > MAX_FRAME_BYTES) {
      this.emit("fatal", new Error("Local agent runtime emitted an oversized frame."));
      this.abort();
      return;
    }
    let frame: IncomingFrame;
    try {
      frame = parseIncomingFrame(line);
    } catch (error) {
      this.emit("fatal", new Error(`Invalid local runtime response: ${String(error)}`));
      this.abort();
      return;
    }

    if (frame.kind === "ready") {
      if (frame.protocolVersion !== PROTOCOL_VERSION) {
        this.emit("fatal", new Error("Local runtime protocol version is not supported."));
        this.abort();
        return;
      }
      if (frame.workspaceRoot !== this.workspacePath) {
        this.emit("fatal", new Error("Local runtime opened an unexpected workspace."));
        this.abort();
        return;
      }
      this.ready = true;
      this.emitStatus("ready", "Local agent runtime is ready.", false);
      this.emit("ready");
      return;
    }
    if (frame.kind === "event") {
      this.emit("event", { event: frame.event, payload: frame.payload });
      return;
    }
    if (frame.kind === "fatal") {
      this.emit("fatal", new Error(frame.error.message));
      this.abort();
      return;
    }
    if (frame.kind === "response") {
      const pending = this.pending.get(String(frame.id));
      if (!pending) return;
      this.pending.delete(String(frame.id));
      clearTimeout(pending.timeout);
      if (frame.ok) pending.resolve(frame.result);
      else pending.reject(new DesktopCommandError(frame.error ?? { code: "command_failed", message: "Desktop command failed." }));
    }
  }

  private rejectPending(error: Error): void {
    for (const request of this.pending.values()) {
      clearTimeout(request.timeout);
      request.reject(error);
    }
    this.pending.clear();
  }

  private abort(): void {
    this.stopping = false;
    const child = this.child;
    if (!child) return;
    child.kill("SIGTERM");
    const killTimer = setTimeout(() => {
      if (this.child === child) child.kill("SIGKILL");
    }, 1_000);
    killTimer.unref();
  }

  private emitStatus(
    state: BackendStatus["state"],
    message: string,
    canRetry: boolean,
  ): void {
    this.emit("status", { state, message, canRetry } satisfies BackendStatus);
  }
}

export function parseIncomingFrame(line: string): IncomingFrame {
  const value = JSON.parse(line) as unknown;
  if (!isRecord(value) || typeof value.kind !== "string") {
    throw new Error("Protocol frame must be an object with a kind.");
  }
  if (value.kind === "ready") {
    if (typeof value.protocolVersion !== "number" || typeof value.workspaceRoot !== "string") {
      throw new Error("Ready frame is malformed.");
    }
    return value as IncomingFrame;
  }
  if (value.kind === "event") {
    if (typeof value.event !== "string" || !("payload" in value)) {
      throw new Error("Event frame is malformed.");
    }
    return value as IncomingFrame;
  }
  if (value.kind === "fatal") {
    if (!isProtocolError(value.error)) throw new Error("Fatal frame is malformed.");
    return value as IncomingFrame;
  }
  if (value.kind === "response") {
    if ((typeof value.id !== "string" && typeof value.id !== "number") || typeof value.ok !== "boolean") {
      throw new Error("Response frame is malformed.");
    }
    if (!value.ok && !isProtocolError(value.error)) {
      throw new Error("Failed response frame is missing an error.");
    }
    return value as IncomingFrame;
  }
  throw new Error(`Unknown protocol frame kind ${value.kind}.`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isProtocolError(value: unknown): value is ProtocolError {
  return isRecord(value) && typeof value.code === "string" && typeof value.message === "string";
}

/** Milliseconds before a request is abandoned, or `null` for runs that only end on completion. */
export function requestTimeout(method: string, params: unknown): number | null {
  // The renderer sends plugin RPCs as `{ input: { method, ... } }`.
  const input = isRecord(params) && isRecord(params.input) ? params.input : params;
  if (method === "plugin_host_rpc" && isRecord(input) && LONG_RUNNING_PLUGIN_METHODS.has(String(input.method))) {
    return null;
  }
  return REQUEST_TIMEOUT_MS;
}
