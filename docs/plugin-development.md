# Plugin development

A Daily plugin is a folder with a manifest at `.daily-plugin/plugin.json` and one or more views that run in sandboxed iframes. Views call the host through `window.dailyPlugin`, typed by [`@daily/plugin-sdk`](../packages/plugin-sdk).

> **Trust model.** An enabled plugin can call every host API: read and write workspace files, fetch public URLs, invoke models, and run agents on the user's providers. There is no permission prompt. Users should install only plugins they trust.

## Layout

```text
my-plugin/
├── .daily-plugin/
│   ├── plugin.json        # manifest
│   └── dist/view.js       # built view entry, relative to .daily-plugin/
├── src/main.tsx           # dev entry served by your dev server
└── package.json
```

[`examples/local-writer-plugin`](../examples/local-writer-plugin) is a minimal plugin with a hand-written `view.js`.

## Manifest

```json
{
  "identifier": "acme.notes",
  "name": "Notes",
  "description": "A notes canvas for the workspace.",
  "version": "0.1.0",
  "author": "Acme",
  "contributes": {
    "views": [
      {
        "id": "notes.canvas",
        "title": "Notes",
        "location": "main",
        "runtime": "iframe",
        "entry": "dist/view.js",
        "devEntry": "src/main.tsx",
        "icon": "notebook-pen"
      },
      {
        "id": "notes.worker",
        "title": "Notes worker",
        "location": "background",
        "entry": "dist/worker.js"
      }
    ],
    "renderers": [],
    "agentTools": [],
    "eventViewParsers": []
  },
  "settingsSchema": {}
}
```

| Field | Notes |
| --- | --- |
| `identifier` | 1–128 ASCII letters, digits, `.`, `_` or `-`. Must not clash with a built-in or marketplace id. |
| `views[].location` | `main` adds an entry to the app rail and fills the canvas; `background` stays mounted while the plugin is enabled, for timers and workers. |
| `views[].runtime` | Only `iframe` is supported. |
| `views[].entry` | Built script, relative to `.daily-plugin/`. Loaded when no dev URL is set. |
| `views[].devEntry` | Module path on your dev server, used when the plugin was installed with a dev URL. |
| `views[].icon` | A [lucide](https://lucide.dev/icons) icon name. |
| `renderers` | Inline renderers the agent can target from a reply (see below). |
| `agentTools` | Tool hints shown to agents that mount the plugin. |
| `eventViewParsers` | Custom renderers for agent event views, matched by event kind, tool id, and status. |

At most 20 views per plugin. Unknown fields are ignored.

## Install and develop

1. Run your dev server (for example Vite on `http://127.0.0.1:5174`).
2. In Daily, open **Settings → Plugins → Install local plugin**.
3. Enter the **absolute** path to the plugin folder or its `plugin.json`, and optionally the dev URL. Dev URLs must point at `localhost`, `127.0.0.1`, or `[::1]`.

With a dev URL the frame loads `<devUrl>/<devEntry>` with Vite's React Fast Refresh preamble; without one it loads the built `entry`. If the frame renders nothing within 15 seconds, or its script fails to load, the host shows an error with a retry button.

Frames run with `sandbox="allow-scripts"` and a CSP that allows only the host-injected scripts (and the dev server origin in development). They have no access to Node.js, Electron, the parent DOM, or arbitrary network origins; use `network.fetch` instead.

## Using the SDK

Add the SDK (it is not published to npm yet; depend on it by path):

```json
{ "dependencies": { "@daily/plugin-sdk": "file:../daily/packages/plugin-sdk" } }
```

```ts
import { dailyHost, errorMessage, hostErrorCode } from "@daily/plugin-sdk";

const host = dailyHost();
const context = await host.host.getContext();      // { plugin, view, agents, runtime }
await host.storage.set("draft", { text: "hello" });
const page = await host.network.fetch("https://example.com/feed.xml");
const { text } = await host.model.invoke({ prompt: "Summarize this page", system: "Be brief." });
```

Rejected calls throw an `Error` with a `code` matching the host's error codes (`invalid_params`, `not_found`, `payload_too_large`, `plugin_disabled`, `unavailable`, `command_failed`). Use `hostErrorCode(error)` to read it.

### API reference

| Area | Methods |
| --- | --- |
| `host` | `getContext()`, `setDirty(dirty)` — mark unsaved work so route changes, window close, and quit ask for confirmation |
| `storage` | `get`, `set`, `remove`, `list`, `compareAndSet(key, expected, value)`, `compareAndRemove(key, expected)` — small JSON values per plugin; compare-and-swap is atomic across views |
| `files` | `readText`, `writeText`, `remove`, `list`, `ensureDir` — a private folder at `.daily/plugin-files/<plugin>/` |
| `workspace` | `readText`, `writeText`, `list`, `ensureDir`, `removeFile` — workspace files, relative paths only, `.daily` is not accessible |
| `network` | `fetch(url, { timeoutMs, headers })` — public HTTP(S) only, 2 MiB response cap |
| `model` | `list()` — configured providers and models (no credentials); `invoke({ prompt, system, model, temperature, maxOutputTokens, reasoningEffort })` |
| `agent` | `list()`, `run(payload)`, `task.start/resume/retry/cancel`, `session.create/list/get/remove/appendMessages/updateEntry/stop/removeAgentTool` |
| events | `on("agent.progress", listener)`; `subscribeAgentProgress(streamId, onPatches)` and `applyAgentEventViewPatches(views, patches)` keep a live step list |

Request payloads are limited to 512 KiB, except file writes (16 MiB).

### Agent tasks

`agent.task.start` runs a structured task on the user's model and returns when it finishes or pauses on an async tool:

```ts
const response = await host.agent.task.start({
  systemPrompt: "You edit notes.",
  userPrompt: "Tidy the headings in this note.",
  context: [{ title: "Note", content: noteText }],
  tools: [
    {
      id: "notes.replace",
      name: "Replace note",
      description: "Propose replacement text for the user to review.",
      inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
      execution: { type: "async", pendingMessage: "Waiting for the user to review." },
    },
  ],
  streamId: "notes-run-1", // receive agent.progress events for this call
});

if (response.status === "tool_call") {
  // Show response.input to the user, then resume with their decision.
  await host.agent.task.resume({ runId: response.runId, ok: true, result: { accepted: true } });
}
```

- `sync` tools return a fixed `observation` immediately; `async` tools pause the run until you call `resume`.
- Pass `sessionId` to record the task in a host-owned session: later tasks replay its history and the host compacts it when it grows. `session.stop` ends a running task; `task.retry` continues it from its checkpoint.
- `subAgents: { agents, allowCreate }` lets the model call other agents as tools.
- `maxIterations` is capped by the host and reported back in the response.

### Inline renderers

Agents that mount your plugin can embed a fenced block in their reply:

````text
```daily-plugin
{ "pluginId": "acme.notes", "renderer": "note-card", "payload": { "title": "Plan" } }
```
````

Built-in plugins render these blocks directly; renderers from iframe plugins are declared in the manifest for discovery.

### Localized runtime text

Event view titles and runtime status come from the Rust runtime in English. Call `localizeHostText(text, "zh-CN")` to show them in Chinese.

## Adding a host API method

1. Add the method to `PLUGIN_RPC_METHODS` and the dispatcher in `crates/daily-core/src/plugins/rpc/mod.rs`, with validation in Rust.
2. Run `npm run gen:protocol` to update `packages/plugin-sdk/src/methods.generated.ts`.
3. Expose it in `packages/plugin-sdk/src/bridge.ts` and type it in `DailyPluginApi` (`index.ts`).
4. `npm test` checks that the bridge and the Rust method table match.
