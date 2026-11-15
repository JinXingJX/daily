# Plugin development

A Daily plugin is a folder with a manifest at `.daily-plugin/plugin.json` and an ES module. The host imports one instance per plugin into its renderer, calls `activate(host)` once, and mounts views into Shadow DOM roots. Plugins use [`@daily/plugin-sdk`](../packages/plugin-sdk).

> **Trust model.** An enabled plugin can call every host API: read and write workspace files, fetch public URLs, invoke models, and run agents on the user's providers. There is no permission prompt. Users should install only plugins they trust.

## Layout

```text
my-plugin/
├── .daily-plugin/
│   ├── plugin.json        # manifest
│   └── dist/plugin.js     # built module entry, relative to .daily-plugin/
├── src/main.tsx           # dev entry served by your dev server
└── package.json
```

[`examples/local-writer-plugin`](../examples/local-writer-plugin) is a minimal plugin with a hand-written `view.js`.

## Manifest

```json
{
  "identifier": "local.notes",
  "name": "Notes",
  "description": "A local notes workbench.",
  "version": "0.1.0",
  "author": "Local Dev",
  "entry": "dist/plugin.js",
  "devEntry": "src/main.tsx",
  "activation": "onView",
  "contributes": {
    "views": [
      {
        "id": "notes.main",
        "title": "Notes",
        "icon": "notebook-pen"
      }
    ]
  }
}
```

| Field | Notes |
| --- | --- |
| `identifier` | 1–128 ASCII letters, digits, `.`, `_` or `-`. Must not clash with a built-in or marketplace id. |
| `entry` | ES module with a default plugin definition, relative to `.daily-plugin/`. |
| `activation` | `onView` (default) loads on first open; `startup` also runs without an open view. Return cleanup from `activate` to stop timers and task handlers. |
| `devEntry` | Module path on your dev server, used when the plugin was installed with a dev URL. |
| `views[].icon` | A [lucide](https://lucide.dev/icons) icon name. |
| `renderers` | Inline renderers the agent can target from a reply (see below). |
| `agentTools` | Tool hints shown to agents that mount the plugin. |
| `eventViewParsers` | Custom renderers for agent event views, matched by event kind, tool id, and status. |

At most 20 views per plugin. Legacy per-view `runtime`, `location`, and `entry` fields are rejected. Existing iframe plugins must migrate to the module contract.

## Install and develop

1. Run your dev server (for example Vite on `http://127.0.0.1:5174`).
2. In Daily, open **Settings → Plugins → Install local plugin**.
3. Enter the **absolute** path to the plugin folder or its `plugin.json`, and optionally the dev URL. Dev URLs must point at `localhost`, `127.0.0.1`, or `[::1]`.

With a dev URL Daily imports `<devUrl>/<devEntry>` after installing the Vite React refresh preamble. Built plugins load from a `daily-plugin:` folder origin; relative chunks and assets work normally. Build ES modules (`formats: ["es"]`), bundle dependencies, and return a default `definePlugin` definition. The host displays import and mount failures with a retry button.

Views mount on first use and stay mounted while hidden. Use `view.onVisibilityChange` to pause UI polling and animations, `view.onThemeChange` for imperative theme updates, and the live `view.visible`/`view.theme` values. Styles added through `view.addStyles(css)` are shared as constructed stylesheets. Use `:host([data-daily-theme="dark"])`, `cqw`/`cqh`, and `@container daily-view` for theme and view-relative layout. Scope keyboard listeners to the supplied container. Async mounts can stop when `view.signal` is aborted and must return their cleanup.

Plugins are trusted code in the app renderer: Shadow DOM separates styles, not permissions or crashes. Heavy CPU work should yield or use a Web Worker. Node integration remains disabled; the host API provides persistent files, model calls and tasks. Disabling, removing or reloading a plugin unmounts its views and calls its activation cleanup. Changing routes or themes does neither. Rebuilding the entry and using Reload plugins loads the new module revision.

## Using the SDK

Add the SDK (it is not published to npm yet; depend on it by path):

```json
{ "dependencies": { "@daily/plugin-sdk": "file:../daily/packages/plugin-sdk" } }
```

After `activate(host)` has run, any part of the plugin can access the same host:

```ts
import { dailyHost, errorMessage, hostErrorCode } from "@daily/plugin-sdk";

const host = dailyHost();
const context = await host.host.getContext();      // { plugin }
await host.storage.set("draft", { text: "hello" });
const page = await host.network.fetch("https://example.com/feed.xml");
const { text } = await host.model.invoke({ prompt: "Summarize this page", system: "Be brief." });
```

Rejected calls throw an `Error` with a `code` matching the host's error codes (`invalid_params`, `not_found`, `payload_too_large`, `plugin_disabled`, `unavailable`, `command_failed`). Use `hostErrorCode(error)` to read it.

### API reference

| Area | Methods |
| --- | --- |
| `host` | `getContext()`, `setDirty(dirty)` — mark unsaved work so window close and quit ask for confirmation |
| `storage` | `get`, `set`, `remove`, `list`, `compareAndSet(key, expected, value)`, `compareAndRemove(key, expected)` — small JSON values per plugin; compare-and-swap is atomic across views |
| `files` | `readText`, `writeText`, `readBase64`, `writeBase64`, `remove`, `list`, `ensureDir` — a private folder at `.daily/plugin-files/<plugin>/` |
| `workspace` | `readText`, `writeText`, `list`, `ensureDir`, `removeFile`, `copyFromPluginFiles(from, to)` — workspace files, relative paths only, `.daily` is not accessible |
| `image` | `status()`; `generate({ prompt, model, size, quality, n, references, outputDir, ... })` — GPT Image 2.6 on the user's OpenAI key. The key never reaches the plugin; images are saved under `outputDir` and their paths returned. Billed per call and never retried by the host |
| `network` | `fetch(url, { timeoutMs, headers })` — public HTTP(S) only, 2 MiB response cap |
| `model` | `list()` — configured providers and models (no credentials); `invoke({ prompt, system, model, temperature, maxOutputTokens, reasoningEffort, images })` — `images` are up to 4 plugin-private paths, for providers whose `acceptsImages` is true. Resolves to `{ text, stopReason, usage }`: `stopReason` is `"max_tokens"` when the text was cut off, and `usage` holds token counts when the provider reports them |
| `agent` | `list()`, `run(payload)`, `task.start/resume/retry/cancel`, `session.create/list/get/remove/appendMessages/updateEntry/stop/removeAgentTool` |
| events | `on("agent.progress", listener)`; `subscribeAgentProgress(streamId, onPatches)` and `applyAgentEventViewPatches(views, patches)` keep a live step list |

Request payloads are limited to 512 KiB, except file writes (16 MiB). `files.readBase64` returns at most 20 MiB.

The image key comes from an OpenAI provider in Daily settings whose endpoint is `https://api.openai.com/v1`, then `OPENAI_API_KEY`, then `~/.config/gpt-image-2-6/credentials.env` (mode 0600; `XDG_CONFIG_HOME`, `IMAGE26_CONFIG_DIR` and `%APPDATA%` are honored). Requests use Daily's normal network path unless that folder's `config.json` sets `proxyEnabled` with an HTTP(S) `proxy`.

### Agent tasks

For work that outlives a page, submit a durable task from the main view and put
the complete workflow (including result persistence) in a background handler:

```ts
import { dailyHost, observeTasks, runTaskHandlers } from "@daily/plugin-sdk";

// Main view: save this request key with the pending user action if it may be resent.
await dailyHost().tasks.submit({
  kind: "notes.tidy", scope: noteId, requestKey: crypto.randomUUID(),
  input: { noteId, text: noteText },
});
const stopObserving = observeTasks({ scope: noteId }, renderTasks);
// Page cleanup only removes observation.
// stopObserving();

// Background view: register once for this view's lifecycle.
const stopWorker = runTaskHandlers({
  "notes.tidy": async (context) => {
    const { noteId, text } = context.input as { noteId: string; text: string };
    const result = await context.call<{ text: string }>("tidy-model", "model.invoke", {
      system: "Tidy note headings.", prompt: text,
    });
    context.signal.throwIfAborted();
    // Replace by stable note id: finalization may be repeated after interruption.
    await dailyHost().files.writeText(`notes/${noteId}.md`, result.text);
    return { status: "completed", result };
  },
});
```

`tasks.get(id)` reads the full result; list/event snapshots omit large completed
results. `observeTasks` attaches from a snapshot cursor and catches up without
losing events. `tasks.cancel(id)` is explicit; aborting an observer never cancels
execution. Save a checkpoint with `context.update({checkpoint, progress})` at
safe boundaries. `tasks.retry(id)` continues an interrupted/failed attempt;
`tasks.resume(id, input)` answers a task whose handler returned `waiting_input`.

Operation keys must identify the same request across attempts. Reusing a key with
a different payload is rejected. If the sidecar exits while an external request
is in flight, its outcome is unknown and the journal blocks automatic replay.
Image operations are never repeated by continuing the same task. Full lifecycle,
limits and failure semantics are in [Durable plugin tasks](task-runtime.md).

The lower-level agent API below executes a single segment; call it from a
background workflow when a sequence of tool calls must survive page changes.

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
- `subAgents: { agents, allowCreate }` selects root agents offered as tools. `definitions` adds their model binding, delegates and generation parameters; `modelBindings` selects existing host provider/model configurations without exposing credentials.
- `allowHandoff` enables `agent.handoff` to a declared delegate. A definition must opt in separately before its child can hand off or dynamically create agents.
- `runLimits` shares budgets across the entire delegation tree. The desktop caps concurrency at four, depth at eight, model requests at 1024 and tool calls at 4096. Absolute deadlines survive retries. Reported-token limits stop subsequent calls after observed usage reaches the limit.
- `maxIterations` is capped by the host and reported back in the response.

For example, a reviewer can use a separately configured model and delegate to an editor:

```ts
const reviewer = { id: "reviewer", name: "Reviewer", instructions: "Review the draft.", tools: ["notes.replace"] };
await host.agent.task.start({
  systemPrompt: "Coordinate the review.", userPrompt: draft,
  tools: noteTools,
  subAgents: { agents: [reviewer] },
  definitions: [
    { spec: reviewer, modelKey: "review", delegates: ["editor"] },
    { spec: { id: "editor", name: "Editor", instructions: "Edit the draft.", tools: ["notes.replace"] }, modelKey: "edit" },
  ],
  modelBindings: {
    review: { authConfigId: reviewerProviderId, model: reviewerModel },
    edit: { authConfigId: editorProviderId, model: editorModel },
  },
  runLimits: { maxModelRequests: 24, maxToolCalls: 32, maxConcurrency: 2, maxDepth: 3 },
});
```

The host preserves recursive pauses and completed tool results in its SDK checkpoint store. Retrying a failed model request is an explicit user action. An external tool whose outcome is uncertain must be resolved, rather than dispatched again automatically. The standalone Rust framework additionally exposes workflow scheduling and supervision; see [daily-agent-core](../crates/daily-agent-core/README.md).

### Inline renderers

Agents that mount your plugin can embed a fenced block in their reply:

````text
```daily-plugin
{ "pluginId": "acme.notes", "renderer": "note-card", "payload": { "title": "Plan" } }
```
````

Built-in plugins render these blocks directly; renderers from module plugins are declared in the manifest for discovery.

### Localized runtime text

Event view titles and runtime status come from the Rust runtime in English. Call `localizeHostText(text, "zh-CN")` to show them in Chinese.

## Adding a host API method

1. Add the method to `PLUGIN_RPC_METHODS` and the dispatcher in `crates/daily-desktop-runtime/src/plugins/rpc/mod.rs`, with validation in Rust.
2. Run `npm run gen:protocol` to update `packages/plugin-sdk/src/methods.generated.ts`.
3. Expose it in `packages/plugin-sdk/src/host.ts` and type it in `DailyPluginApi` (`index.ts`).
4. `npm test` checks that the bridge and the Rust method table match.
