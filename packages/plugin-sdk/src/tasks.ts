import { dailyHost } from "./index";

export type TaskStatus = "queued" | "running" | "cancelling" | "waiting_input" | "completed" | "failed" | "cancelled" | "interrupted";
export type TaskSnapshot = {
  id: string; title: string; pluginId: string; kind: string; scope: string; requestKey: string;
  exclusiveKey: string | null; status: TaskStatus; attempt: number; cursor: number;
  progress: unknown; result: unknown; error: string | null; createdAt: number; updatedAt: number;
};
export type TaskSubmitInput = { kind: string; title?: string; scope?: string; requestKey: string; exclusiveKey?: string; input: unknown };
export type TaskClaim = { task: TaskSnapshot; input: unknown; checkpoint: unknown; resume: unknown; leaseToken: string };
export type TaskApi = {
  submit(input: TaskSubmitInput): Promise<TaskSnapshot>;
  get(taskId: string): Promise<TaskSnapshot>;
  list(filter?: { scope?: string; kind?: string }): Promise<{ tasks: TaskSnapshot[]; cursor: number }>;
  events(after: number): Promise<{ events: { seq: number; task: TaskSnapshot }[]; cursor: number; reset?: boolean }>;
  cancel(taskId: string): Promise<TaskSnapshot>;
  retry(taskId: string): Promise<TaskSnapshot>;
  resume(taskId: string, input: unknown): Promise<TaskSnapshot>;
  claim(kinds: string[]): Promise<TaskClaim | null>;
  heartbeat(input: { taskId: string; leaseToken: string }): Promise<TaskSnapshot>;
  update(input: { taskId: string; leaseToken: string; checkpoint?: unknown; progress?: unknown }): Promise<TaskSnapshot>;
  finish(input: { taskId: string; leaseToken: string; status: "completed" | "waiting_input" | "failed" | "cancelled"; result?: unknown; error?: string }): Promise<TaskSnapshot>;
  call(input: { taskId: string; leaseToken: string; operationKey: string; method: string; payload: unknown }): Promise<{ status: string; result: unknown; error?: string }>;
};

const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
export const taskIsActive = (task: TaskSnapshot) => ["queued", "running", "cancelling"].includes(task.status);

/** Unsubscribing or aborting stops observation; task cancellation is an explicit command. */
export async function waitForTask(taskId: string, options: { signal?: AbortSignal; onSnapshot?: (task: TaskSnapshot) => void } = {}): Promise<TaskSnapshot> {
  for (;;) {
    options.signal?.throwIfAborted();
    const task = await dailyHost().tasks.get(taskId);
    options.signal?.throwIfAborted();
    options.onSnapshot?.(task);
    if (!taskIsActive(task)) return task;
    await pause(500);
  }
}

/** Snapshot + ordered catch-up, so a page rebuild cannot miss changes during attachment. */
export function observeTasks(filter: { scope?: string; kind?: string }, onChange: (tasks: TaskSnapshot[]) => void, onError?: (error: unknown) => void): () => void {
  let stopped = false;
  void (async () => {
    let cursor = 0;
    const tasks = new Map<string, TaskSnapshot>();
    let initialized = false;
    while (!stopped) {
      try {
        if (!initialized) {
          const snapshot = await dailyHost().tasks.list(filter);
          if (stopped) return;
          for (const task of snapshot.tasks) tasks.set(task.id, task);
          cursor = snapshot.cursor;
          initialized = true;
          onChange([...tasks.values()]);
        }
        const next = await dailyHost().tasks.events(cursor);
        if (stopped) return;
        if (next.reset) { initialized = false; tasks.clear(); cursor = 0; continue; }
        let changed = false;
        for (const event of next.events) {
          const task = event.task;
          if ((filter.scope === undefined || task.scope === filter.scope) && (filter.kind === undefined || task.kind === filter.kind) && task.cursor > (tasks.get(task.id)?.cursor ?? -1)) {
            tasks.set(task.id, task); changed = true;
          }
        }
        cursor = next.cursor;
        if (changed) onChange([...tasks.values()]);
        if (next.events.length >= 100) continue;
      } catch (error) { if (!stopped) onError?.(error); }
      await pause(750);
    }
  })();
  return () => { stopped = true; };
}

export type TaskContext = {
  task: TaskSnapshot;
  input: unknown;
  checkpoint: unknown;
  resume: unknown;
  signal: AbortSignal;
  update(patch: { checkpoint?: unknown; progress?: unknown }): Promise<void>;
  /** An operation key is stable across attempts; an uncertain external outcome is never replayed. */
  call<T>(operationKey: string, method: string, payload: unknown): Promise<T>;
};
export type TaskOutcome = { status: "completed" | "waiting_input"; result: unknown };
export type TaskHandler = (context: TaskContext) => Promise<TaskOutcome>;

/** Claims and runs this plugin's queued tasks one at a time; start it from `activate`. */
export function runTaskHandlers(handlers: Record<string, TaskHandler>, onError?: (error: unknown) => void): () => void {
  let stopped = false;
  let active: AbortController | null = null;
  const host = dailyHost();
  void (async () => {
    while (!stopped) {
      try {
        const claim = await host.tasks.claim(Object.keys(handlers));
        if (stopped) return;
        if (!claim) { await pause(500); continue; }
        const controller = new AbortController(); active = controller;
        const auth = { taskId: claim.task.id, leaseToken: claim.leaseToken };
        let cancelled = false;
        let queue: Promise<unknown> = Promise.resolve();
        const context: TaskContext = {
          ...claim, signal: controller.signal,
          update(patch) {
            queue = queue.then(async () => {
              controller.signal.throwIfAborted();
              await host.request("tasks.update", { ...auth, ...patch });
              if ("checkpoint" in patch) context.checkpoint = patch.checkpoint;
            });
            return queue.then(() => undefined);
          },
          async call<T>(operationKey: string, method: string, payload: unknown): Promise<T> {
            for (;;) {
              controller.signal.throwIfAborted();
              const operation = await host.request("tasks.call", { ...auth, operationKey, method, payload }) as { status: string; result: T; error?: string };
              if (operation.status === "completed") return operation.result;
              if (operation.status !== "running") throw new Error(operation.error ?? "Task operation was interrupted.");
              await pause(350);
            }
          },
        };
        let heartbeating = false;
        const heartbeat = setInterval(() => {
          if (heartbeating) return;
          heartbeating = true;
          void host.request("tasks.heartbeat", auth).then((snapshot) => {
            if ((snapshot as TaskSnapshot).status === "cancelling") { cancelled = true; controller.abort(Object.assign(new Error("Task was cancelled."), { name: "TaskCancelledError" })); }
          }, (error) => { controller.abort(error); }).finally(() => { heartbeating = false; });
        }, 5_000);
        try {
          const outcome = await handlers[claim.task.kind](context);
          await queue;
          if (!stopped) await host.request("tasks.finish", { ...auth, ...outcome });
        } catch (error) {
          if (!stopped) await host.request("tasks.finish", { ...auth, status: cancelled ? "cancelled" : "failed", error: error instanceof Error ? error.message : String(error) }).catch(onError);
        } finally { clearInterval(heartbeat); active = null; }
      } catch (error) { if (!stopped) { onError?.(error); await pause(1_000); } }
    }
  })();
  return () => { stopped = true; active?.abort(); };
}
