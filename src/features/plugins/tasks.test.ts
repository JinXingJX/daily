import { afterEach, describe, expect, it, vi } from "vitest";
import { observeTasks, waitForTask, runTaskHandlers, setDailyHost, type DailyPluginApi, type TaskSnapshot } from "@daily/plugin-sdk";

function snapshot(status: TaskSnapshot["status"], cursor = 1): TaskSnapshot {
  return { id: "task-1", title: "Write", pluginId: "writer", kind: "write", scope: "book", requestKey: "one", exclusiveKey: null, status, attempt: 1, cursor, progress: null, result: null, error: null, createdAt: 1, updatedAt: 1 };
}
function host(api: Partial<DailyPluginApi>) { setDailyHost(api as DailyPluginApi); }
afterEach(() => { setDailyHost(null); vi.useRealTimers(); });

describe("durable task clients", () => {
  it("aborting a page observer never sends a cancellation command", async () => {
    const controller = new AbortController();
    const cancel = vi.fn();
    host({ tasks: { get: async () => { controller.abort(); return snapshot("running"); }, cancel } as unknown as DailyPluginApi["tasks"] });
    const observed = waitForTask("task-1", { signal: controller.signal });
    await expect(observed).rejects.toMatchObject({ name: "AbortError" });
    expect(cancel).not.toHaveBeenCalled();
  });

  it("attaches from the snapshot cursor and catches changes made while the page attached", async () => {
    const events = vi.fn().mockResolvedValueOnce({ events: [{ seq: 8, task: snapshot("completed", 8) }], cursor: 8 }).mockResolvedValue({ events: [], cursor: 8 });
    host({ tasks: { list: async () => ({ tasks: [snapshot("running", 7)], cursor: 7 }), events } as unknown as DailyPluginApi["tasks"] });
    const changed = vi.fn();
    const stop = observeTasks({ scope: "book" }, changed);
    await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(2));
    expect(events).toHaveBeenCalledWith(7);
    expect(changed.mock.calls[1][0][0].status).toBe("completed");
    stop();
  });

  it("persists worker progress before completing and leaves no page-owned execution", async () => {
    const writes: string[] = [];
    const request = vi.fn(async (method: string) => { writes.push(method); return snapshot("running"); });
    const claim = vi.fn().mockResolvedValueOnce({ task: snapshot("running"), input: { text: "input" }, checkpoint: null, resume: null, leaseToken: "lease" }).mockResolvedValue(null);
    host({ tasks: { claim } as unknown as DailyPluginApi["tasks"], request });
    const stop = runTaskHandlers({ write: async (context) => {
      await context.update({ checkpoint: { step: 1 }, progress: "working" });
      return { status: "completed", result: "done" };
    } });
    await vi.waitFor(() => expect(request).toHaveBeenCalledWith("tasks.finish", expect.objectContaining({ taskId: "task-1", leaseToken: "lease", status: "completed", result: "done" })));
    expect(writes).toEqual(["tasks.update", "tasks.finish"]);
    stop();
  });
});
