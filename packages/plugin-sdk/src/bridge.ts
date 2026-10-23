// The runtime half of the SDK: the host serializes `installDailyBridge` with
// `Function.prototype.toString()` and runs it inside each sandboxed plugin frame, where it
// defines `window.dailyPlugin`. It must stay self-contained — no imports and no references to
// anything outside the function body — because only its source text crosses into the frame.
// The typed view of the object it installs is `DailyPluginApi` in `index.ts`.

/* eslint-disable @typescript-eslint/no-explicit-any */
export function installDailyBridge(pluginId: string, viewId: string): void {
  type Pending = { resolve: (value: any) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> | undefined };
  let nextId = 1;
  const pending = new Map<string, Pending>();
  // Model and agent calls end when the run finishes (or is stopped); only short calls time out.
  const longRunning = new Set(["model.invoke", "agent.run", "agent.task.start", "agent.task.resume", "agent.task.retry"]);
  const shortTimeoutMs = 120_000;
  const listeners = new Map<string, Set<(payload: unknown) => void>>();

  function on(event: string, listener: (payload: unknown) => void): () => void {
    let set = listeners.get(event);
    if (!set) listeners.set(event, (set = new Set()));
    set.add(listener);
    return () => {
      set?.delete(listener);
    };
  }

  function request(method: string, payload?: unknown): Promise<any> {
    const id = String(nextId++);
    return new Promise((resolve, reject) => {
      const timer = longRunning.has(method)
        ? undefined
        : setTimeout(() => {
            if (pending.delete(id)) reject(new Error("Plugin host request timed out."));
          }, shortTimeoutMs);
      pending.set(id, { resolve, reject, timer });
      parent.postMessage(
        { source: "daily-plugin", id, pluginId, viewId, method, payload: payload === undefined ? {} : payload },
        "*",
      );
    });
  }

  window.addEventListener("message", (event: MessageEvent) => {
    const data = (event.data || {}) as Record<string, any>;
    if (event.source !== parent || data.source !== "daily-host") return;
    if (data.type === "event" && data.id === undefined) {
      listeners.get(data.event)?.forEach((listener) => {
        try {
          listener(data.payload);
        } catch (error) {
          console.error(error);
        }
      });
      return;
    }
    const callbacks = pending.get(data.id);
    if (!callbacks) return;
    pending.delete(data.id);
    clearTimeout(callbacks.timer);
    if (data.ok) {
      callbacks.resolve(data.payload);
    } else {
      const error = new Error(data.error || "Plugin host request failed.") as Error & { code?: string };
      if (typeof data.code === "string") error.code = data.code;
      callbacks.reject(error);
    }
  });

  (window as any).dailyPlugin = {
    pluginId,
    viewId,
    request,
    on,
    host: {
      getContext: () => request("host.getContext", {}),
      setDirty: (dirty: boolean) => request("host.setDirty", { dirty }),
    },
    storage: {
      get: (key: string) => request("storage.get", { key }).then((result) => result.value),
      set: (key: string, value: unknown) => request("storage.set", { key, value }),
      compareAndSet: (key: string, expected: unknown, value: unknown) =>
        request("storage.compareAndSwap", { key, expected, value }).then((result) => result.swapped),
      compareAndRemove: (key: string, expected: unknown) =>
        request("storage.compareAndSwap", { key, expected, remove: true }).then((result) => result.swapped),
      remove: (key: string) => request("storage.remove", { key }),
      list: () => request("storage.list", {}),
    },
    files: {
      readText: (path: string) => request("plugin.files.readText", { path }).then((result) => result.content),
      writeText: (path: string, content: string) => request("plugin.files.writeText", { path, content }),
      remove: (path: string) => request("plugin.files.remove", { path }),
      list: (path?: string) => request("plugin.files.list", { path }),
      ensureDir: (path: string) => request("plugin.files.ensureDir", { path }),
    },
    workspace: {
      readText: (path: string) => request("workspace.readText", { path }).then((result) => result.content),
      list: (path?: string) => request("workspace.list", { path }),
      writeText: (path: string, content: string) => request("workspace.writeText", { path, content }),
      ensureDir: (path: string) => request("workspace.ensureDir", { path }),
      removeFile: (path: string) => request("workspace.removeFile", { path }),
    },
    model: {
      invoke: (payload: unknown) => request("model.invoke", payload),
      list: () => request("model.list", {}),
    },
    network: {
      fetch: (url: string, options?: Record<string, unknown>) => request("network.fetch", { url, ...(options || {}) }),
    },
    agent: {
      list: () => request("agent.list", {}),
      run: (payload: unknown) => request("agent.run", payload),
      task: {
        start: (payload: unknown) => request("agent.task.start", payload),
        resume: (payload: unknown) => request("agent.task.resume", payload),
        retry: (payload: unknown) => request("agent.task.retry", payload),
        cancel: (runId: string) => request("agent.task.cancel", { runId }),
      },
      session: {
        create: (payload?: unknown) => request("agent.session.create", payload || {}).then((result) => result.session),
        list: (scope?: unknown) =>
          request("agent.session.list", scope === undefined ? {} : { scope }).then((result) => result.sessions),
        get: (sessionId: string) => request("agent.session.get", { sessionId }).then((result) => result.session),
        remove: (sessionId: string) => request("agent.session.delete", { sessionId }),
        appendMessages: (sessionId: string, messages: unknown) =>
          request("agent.session.appendMessages", { sessionId, messages }).then((result) => result.entries),
        updateEntry: (sessionId: string, entryId: string, patch?: Record<string, unknown>) =>
          request("agent.session.updateEntry", { sessionId, entryId, ...(patch || {}) }).then((result) => result.entry),
        stop: (sessionId: string) => request("agent.session.stop", { sessionId }),
        removeAgentTool: (sessionId: string, agentId: string) =>
          request("agent.session.removeAgentTool", { sessionId, agentId }),
      },
    },
  };
}

/** Source of a script that installs the bridge for one plugin view. */
export function bridgeScript(pluginId: string, viewId: string): string {
  return `(${installDailyBridge.toString()})(${JSON.stringify(pluginId)}, ${JSON.stringify(viewId)});`;
}
