# Architecture

Daily is three processes per workspace window: the Electron main process, a sandboxed React renderer, and a Rust sidecar (`daily-app-server`) that owns all agent, model, plugin, settings, and persistence behavior. Electron is the shell and transport; it never runs model or tool loops.

```text
React renderer (sandboxed, no Node.js)      plugin iframes (sandbox="allow-scripts")
        │ window.dailyDesktop (contextBridge)          │ window.dailyPlugin (postMessage)
        ▼                                              ▼
Electron main ── validates IPC sender frames, owns windows, supervises sidecars
        │ newline-delimited JSON over stdio (protocol v1, bounded frames)
        ▼
daily-app-server ── correlates requests/responses, forwards runtime events
        ▼
daily-core ── runtime commands · agent SDK · model adapters · plugins · settings · state
```

## Repository map

| Path | Responsibility |
| --- | --- |
| `electron/main/` | `workspaceManager.ts` (windows, sidecar lifecycle, close/quit checks), `sidecar.ts` (JSONL client), `ipc.ts` (fixed `daily:*` IPC surface), `menu.ts`, `security.ts`, `windowState.ts`, `i18n.ts` |
| `electron/preload/` | The narrow `window.dailyDesktop` API exposed through `contextBridge` |
| `electron/shared/` | IPC contracts; `commands.generated.ts` is generated from Rust |
| `src/` | Renderer. `App.tsx` shell, `app/` routes, chrome and hooks, `features/` (chat, settings, plugins, workspace), `i18n/` catalogs, `generated/protocol/` wire types from Rust |
| `packages/plugin-sdk/` | `@daily/plugin-sdk`: typed `window.dailyPlugin` API, the bridge the host injects (`bridge.ts`), the generated RPC method list, and runtime text localization |
| `crates/daily-app-server/` | Stdio protocol server |
| `crates/daily-core/` | `runtime/` (command table and handlers), `state/` (workspace, sessions, agents, persistence), `agent/` (SDK loop in `sdk/`, events, turns, hooks, workspace tools), `plugins/` (manifests, state, `rpc/` host API, agent sessions and pending runs), `model/` (OpenAI-compatible, Anthropic, Gemini, Codex, Antigravity), `settings/`, `net/` (HTTP clients, SSRF guard), `fs_utils/` (atomic writes, sandboxed paths) |

## Process model

- Each workspace window owns one sidecar, started with `--project-root <canonical path>`. Opening the same folder again focuses the existing window.
- The sidecar prints a `ready` frame with the protocol version and canonical root; Electron refuses a mismatch.
- An unexpected sidecar exit restarts it once; a second failure within a minute shows a retry banner.
- Closing a window or quitting asks for confirmation while a chat turn or a plugin model/agent run is in progress, or a plugin reports unsaved changes (`host.setDirty`).

## Generated contracts

Rust is the source of truth for the wire format. `npm run gen:protocol` runs the `ts`-feature exports in `daily-core` and writes:

- `src/generated/protocol/*.ts` — one file per serialized Rust type (via `ts-rs`);
- `electron/shared/commands.generated.ts` — the IPC allowlist, from the `desktop_commands!` table;
- `packages/plugin-sdk/src/methods.generated.ts` — plugin RPC methods, from `PLUGIN_RPC_METHODS`.

`npm run check:protocol` (part of `npm test`) fails when any of them is stale. A renderer test also checks that the SDK bridge exposes exactly the generated RPC methods.

## Agent runtime

`crates/daily-core/src/agent/sdk/` owns the model/tool loop: building requests, streaming, provider-native tool calls, tool execution and observations, async tool pauses and resumes, observation compaction, sub-agents, and final output. Callers describe a run with an `AgentTask` (prompts, context blocks, model, limits, tools, prior observations, sub-agent config). The SDK does not know about chat sessions, plugins, or UI.

### Tools

- **Workspace tools** (`agent/workspace_tools.rs`): `workspace.list`, `workspace.read`, and `workspace.search` give chat agents read-only access to the workspace through the same sandbox as the plugin workspace API (no absolute paths, traversal, symlink escapes, or `.daily`). Output is size-bounded.
- **Dynamic tools**: plugins define tools for a single run. A `sync` tool returns a fixed observation; an `async` tool pauses the run. The SDK persists the pending state and returns its `runId`; the caller resumes with the result and the SDK injects one tool observation before the next model request.
- **Agents as tools**: a task can offer agents the model calls like tools (`subagent.<id>`, input `{ task }`). With `allowCreate` the model may define up to 8 of its own through `agent.create`. Sub-agents run one level deep with only the tools they are given; a sub-agent that hits an async tool pauses the whole task.

### Chat turns

- `run_session_turn` queues the turn and returns immediately; the turn runs on a background task and streams `turn_delta` events. Up to `MAX_CONCURRENT_TURNS` (4) sessions per workspace run at once; each session runs one turn at a time.
- Routing: an explicit `@Agent Name` mention in the prompt, then the agent of the most recent turn, the session's last or bound agent, then the first agent.
- Before and after hooks run other agents serially or in parallel; their output is attached to the turn.
- Durable events persist the session atomically; stream chunks only update memory and the UI. On restart, turns that were running are marked failed and can continue from their checkpoint.
- Stopping a turn aborts its task at the next await point and records it as stopped; it can continue later.

## Plugins

Plugins are installed per workspace (`.daily/plugins.json`). Built-in plugins render inline cards (chart, weather, hook reports); local plugins contribute iframe views (`main` or `background`), renderers, agent tool hints, and event view parsers.

- **Views** run in `sandbox="allow-scripts"` iframes with a nonce-based CSP. The host accepts messages only from the mounted frame with the matching plugin and view ids, and rate-bounds requests.
- **Host API**: every call is a plugin RPC dispatched by `plugins/rpc/mod.rs`. There is no per-capability permission system: an enabled plugin can call every method, so plugins are trusted code. The host still enforces enablement, path sandboxing, payload and collection limits, atomic writes, and the network guard.
- **Agent APIs**: `agent.run` (compact prompt/context run on a configured agent) and `agent.task.*` (structured tasks with system/user/final prompts, context blocks, dynamic tools, sub-agents, and limits). Tasks can be linked to a host-owned agent session (`agent.session.*`), which keeps conversation history, run records, and compaction summaries. Pending runs live in `.daily/plugin-agent-runs/` and `.daily/plugin-agent-task-runs/`, bound to the plugin that created them.
- **Storage**: `storage.*` keeps small JSON values per plugin with atomic compare-and-swap; `plugin.files.*` is a private folder under `.daily/plugin-files/<plugin>/`; `workspace.*` reads and writes workspace files outside `.daily`.
- **Network**: `network.fetch` allows only public HTTP(S) targets, re-resolves DNS on every redirect, ignores system proxies, rejects credentials in URLs and unsafe headers, strips credentials on cross-origin redirects, and caps responses at 2 MiB.
- **Development plugins** must be installed from an absolute path, and their dev server URL must be `localhost`, `127.0.0.1`, or `[::1]`, so a cloned repository cannot point the app at code it ships or hosts remotely.

See [plugin-development.md](plugin-development.md) for the manifest and SDK.

## Settings and providers

Settings merge user, project, and local scopes (local wins). Provider configurations (`auth.configs`: endpoint, key, Codex folder) are only read from user and local scopes. Each provider configuration resolves its own key from `apiKeyEnv`, `apiKey`, or the provider's conventional environment variable. See [settings.md](settings.md).

## Localization

The renderer uses English strings as message keys (`t("Retry")`) with a typed Chinese catalog in `src/i18n/zh-CN.ts`; changing the language remounts the app. Rust emits English for event views, runtime status, and errors; `localizeHostText` in the SDK translates the runtime's templates for the host and plugins. Electron menus and dialogs follow the system language (`electron/main/i18n.ts`).

## Security boundaries

- Renderers run with context isolation, sandboxing, no Node.js, no webviews, and a strict CSP. IPC accepts only the main frame of a registered window, and only allowlisted commands.
- Plugin frames are sandboxed; capabilities are host RPCs validated in Rust.
- Workspace and plugin paths reject absolute paths, traversal, symlink escapes, and `.daily`.
- State and settings writes are atomic; files, frames, and collections have explicit size limits; malformed workspace files are rejected before use.

The trust model is in [../SECURITY.md](../SECURITY.md).
