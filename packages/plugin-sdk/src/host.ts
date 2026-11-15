// The runtime half of the SDK: builds the host API object a plugin receives in `activate`.
// The Daily renderer calls it with a transport that forwards to the Rust host; plugin tests can
// pass a fake transport. The typed view of the object is `DailyPluginApi` in `index.ts`.
import type { DailyPluginApi } from "./index";

/** How the API object reaches its host. */
export type PluginHostTransport = {
  /** Sends one plugin RPC; rejects with an Error whose `code` is the host's error code. */
  request(method: string, payload: unknown): Promise<unknown>;
  /** Subscribes to a host event; returns an unsubscribe function. */
  on(event: string, listener: (payload: unknown) => void): () => void;
  /** Tells the host whether the plugin holds unsaved changes. */
  setDirty(dirty: boolean): void;
};

/* eslint-disable @typescript-eslint/no-explicit-any */
export function createPluginHost(pluginId: string, transport: PluginHostTransport): DailyPluginApi {
  const request = (method: string, payload?: unknown): Promise<any> =>
    transport.request(method, payload === undefined ? {} : payload);

  const api = {
    pluginId,
    request,
    on: transport.on,
    host: {
      getContext: () => request("host.getContext", {}),
      setDirty: (dirty: boolean) => {
        transport.setDirty(dirty);
        return Promise.resolve({});
      },
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
      readBase64: (path: string) => request("plugin.files.readBase64", { path }).then((result) => result.base64),
      writeBase64: (path: string, base64: string) => request("plugin.files.writeBase64", { path, base64 }),
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
      copyFromPluginFiles: (from: string, to: string) => request("workspace.copyFromPluginFiles", { from, to }),
    },
    model: {
      invoke: (payload: unknown) => request("model.invoke", payload),
      list: () => request("model.list", {}),
    },
    image: {
      status: () => request("image.status", {}),
      generate: (payload: unknown) => request("image.generate", payload),
    },
    network: {
      fetch: (url: string, options?: Record<string, unknown>) => request("network.fetch", { url, ...(options || {}) }),
    },
    tasks: {
      submit: (payload: unknown) => request("tasks.submit", payload),
      get: (taskId: string) => request("tasks.get", { taskId }),
      list: (filter?: unknown) => request("tasks.list", filter ?? {}),
      events: (after: number) => request("tasks.events", { after }),
      cancel: (taskId: string) => request("tasks.cancel", { taskId }),
      retry: (taskId: string) => request("tasks.retry", { taskId }),
      resume: (taskId: string, input: unknown) => request("tasks.resume", { taskId, input }),
      claim: (kinds: string[]) => request("tasks.claim", { kinds }),
      heartbeat: (payload: unknown) => request("tasks.heartbeat", payload),
      update: (payload: unknown) => request("tasks.update", payload),
      finish: (payload: unknown) => request("tasks.finish", payload),
      call: (payload: unknown) => request("tasks.call", payload),
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
  return api as unknown as DailyPluginApi;
}
