# Architecture

Daily uses an Electron main process, a sandboxed React UI renderer, a Rust sidecar (`daily-app-server`). Rust owns agent/model requests, task scheduling, settings, and persistence. Plugin modules share the app renderer; activation owns background work and lazily opened Shadow DOM views. The Electron main process owns windows and transport.

```text
React renderer + trusted plugin ES modules (Shadow DOM views, no Node.js)
        │ direct host API → window.dailyDesktop (contextBridge)
        ▼
Electron main ── validates IPC sender frames, owns windows, supervises sidecars
        │ newline-delimited JSON over stdio (protocol v1, bounded frames)
        ▼
daily-app-server ── correlates requests/responses, forwards runtime events
        ▼
daily-desktop-runtime ── desktop commands · plugins · settings · state · SDK compatibility facade
        ├── daily-agent-core ── model contract · loop · tools · transcript · compaction · sub-agents
        └── daily-agent-providers ── HTTP/native protocols ──▶ daily-agent-core
```

## Repository map

| Path | Responsibility |
| --- | --- |
| `electron/main/` | `workspaceManager.ts` (windows, sidecar lifecycle, close/quit checks), `sidecar.ts` (JSONL client), `ipc.ts` (fixed `daily:*` IPC surface), `menu.ts`, `security.ts`, `windowState.ts`, `i18n.ts` |
| `electron/preload/` | The narrow `window.dailyDesktop` API exposed through `contextBridge` |
| `electron/shared/` | IPC contracts; `commands.generated.ts` is generated from Rust |
| `src/` | Renderer. `App.tsx` shell, `app/` routes, chrome and hooks, `features/` (chat, settings, plugins, workspace), `i18n/` catalogs, `generated/protocol/` wire types from Rust |
| `packages/plugin-sdk/` | `@daily/plugin-sdk`: `definePlugin`, `dailyHost`, direct host API transport (`host.ts`), the generated RPC method list, and runtime text localization |
| `crates/daily-app-server/` | Stdio protocol server |
| `crates/daily-desktop-runtime/src/tasks.rs` | SQLite task queue, leases, checkpoints, ordered events and operation results |
| `crates/daily-agent-core/` | Standalone SDK: model interface, loop, async tools, transcripts, compaction, events, budgets, sub-agents and pause/resume state; no HTTP or desktop dependency |
| `crates/daily-core/` | Compatibility re-export of `daily-desktop-runtime`; no second execution engine |
| `crates/daily-agent-providers/` | Native model adapters and HTTP transports; `ProviderModel` binds them to the core contract; login adapters require the optional `login` feature |
| `crates/daily-desktop-runtime/` | Desktop `runtime/`, `state/`, `agent/` compatibility/turns/hooks/workspace tools, `plugins/`, desktop `model/` config/views, `settings/`, HTTP/SSRF and sandbox/persistence helpers |

## Process model

Each workspace has one app renderer and one Rust sidecar. Plugin modules are imported once, with `activate(host)` owning background work. Views mount on first open and stay mounted, hidden when inactive. Theme and visibility updates reach the existing view without a reload. Disabling or reloading a plugin runs cleanup and removes its views. There is no iframe or extra background BrowserWindow.

Plugins share the renderer's event loop and DOM access. Shadow DOM isolates styles only. They receive the typed host API directly; storage, model and task operations still use the existing Electron/Rust transport. Node integration remains disabled.

The task center reads workspace-owned snapshots. Task state and checkpoints live
in `.daily/tasks.sqlite3`; an interrupted attempt requires explicit continuation.
External operations are journaled before dispatch and their results survive the
loss of a worker. See [Durable plugin tasks](task-runtime.md).

- Each workspace window owns one sidecar, started with `--project-root <canonical path>`. Opening the same folder again focuses the existing window.
- The sidecar prints a `ready` frame with the protocol version and canonical root; Electron refuses a mismatch.
- An unexpected sidecar exit restarts it once; a second failure within a minute shows a retry banner.
- Closing a window or quitting asks for confirmation while a chat turn or a plugin model/agent run is in progress, or a plugin reports unsaved changes (`host.setDirty`).

## Generated contracts

Rust is the source of truth for the wire format. `npm run gen:protocol` runs the `ts`-feature exports in `daily-desktop-runtime`, including explicit shared-type exports in `protocol_exports.rs` (dependency test functions are not run), and writes:

- `src/generated/protocol/*.ts` — one file per serialized Rust type (via `ts-rs`);
- `electron/shared/commands.generated.ts` — the IPC allowlist, from the `desktop_commands!` table;
- `packages/plugin-sdk/src/methods.generated.ts` — plugin RPC methods, from `PLUGIN_RPC_METHODS`.

`npm run check:protocol` (part of `npm test`) fails when any of them is stale. A renderer test also checks that the SDK bridge exposes exactly the generated RPC methods.

## Agent runtime

`crates/daily-agent-core/` owns the model/tool loop: building requests, streaming, native tool-call transcripts, async tool execution, pauses/resumes, compaction, sub-agents and final output. Callers describe an `AgentTask` and supply an `Arc<dyn Model>`. The model contract carries no HTTP client, credentials, provider enum, desktop session or turn ids. See the [SDK README](../crates/daily-agent-core/README.md) and `cargo run -p daily-agent-core --example multi_agent` for a fully offline example.

`daily-agent-providers::ProviderModel` supplies built-in protocols through this interface. Resolved provider configuration is separate from desktop authentication settings and views. Default provider builds omit CLI/desktop login modules; Daily enables the `login` feature explicitly. Proxy, CA and network validation policies remain owned by the host.

`daily_desktop_runtime::sdk` translates desktop task/model/event entry points into the same core loop. Chat and plugin callers use it; `daily_core::sdk` remains a compatibility re-export. Settings, history selection, hook prompts, plugin identities and `.daily` persistence belong to `daily-desktop-runtime`.

The core supplies `RunSupervisor`, serial/parallel/DAG workflows, independently bound recursive agents and handoffs. Descendants share cancellation, model/tool concurrency, absolute deadlines and aggregate request/tool/token budgets. Hooks use owned futures and the same turn budget. Versioned checkpoints use compare-and-swap revisions; they preserve the pending model round, completed tool cursor and recursive frames. The desktop adapter stores them in `.daily/agent-runs.sqlite3`; session and plugin state carry a run reference instead of copying transcripts/images. Failed runs reload current credentials. Completed results are reused; uncertain tools require a result or an explicit safe replay decision. Workflow state is also durable, so a completed child is recovered even if the parent graph write was interrupted. SDK event replay is bounded and process-local; durable plugin task events use their separate queue database.

The SDK CI job tests default features on Linux, macOS and Windows. `npm run package:agent-sdk` validates a two-crate source archive; `npm run check:agent-sdk` also compiles and runs an external Cargo consumer, checks its resolved dependency graph, and ensures the desktop and login features are absent.

- **Transcript.** Each model response that calls tools is kept as a round: the model's text, its calls, any opaque reasoning the provider returned, and one result per call. Adapters send rounds back in the provider's own format (`tool` messages, `tool_use`/`tool_result` blocks, `functionCall`/`functionResponse` parts, Responses `function_call` items), reasoning included. The request prefix does not change between steps — the system prompt and tool list are fixed for a run and per-step notes are appended — so provider caches keep hitting.
- **Tools** are async: a call can await, has an optional timeout, and is cancelled when the run stops. Calls of one response to tools marked side-effect free run in parallel. A failed or unknown tool is answered with an error result the model can react to.
- **Budgets.** A run has a step limit and shares aggregate limits across descendants; on the last step tools stay declared but may not be called. Tool results beyond a token estimate are folded into a model-written summary, never splitting a round.
- **Errors and usage.** Model failures are typed (`ModelError`: auth, rate limit, overload, context overflow, …); transient ones are retried with backoff until output starts streaming. Every response reports token usage and why it stopped, so a cut-off answer is distinguishable from a finished one.

### Tools

- **Workspace tools** (`agent/workspace_tools.rs`): `workspace.list`, `workspace.read`, and `workspace.search` give chat agents read-only access to the workspace through the same sandbox as the plugin workspace API (no absolute paths, traversal, symlink escapes, or `.daily`). Output is size-bounded; they run on the blocking thread pool, in parallel, with a timeout.
- **Dynamic tools**: plugins define tools for a single run. A `sync` tool returns a fixed observation; an `async` tool pauses the run. The SDK persists the pending state and returns its `runId`; the caller resumes with the result and the SDK injects one tool observation before the next model request.
- **Agents as tools**: a task can offer agents the model calls like tools (`subagent.<id>`, input `{ task }`). With `allowCreate` the model may define up to 8 of its own through `agent.create`. Delegates may recurse within the shared depth limit, select a model binding and hand off execution. They receive only the allowed parent tool subset; an async tool pauses the complete delegation stack.

### Chat turns

- `run_session_turn` queues the turn and returns immediately; the turn runs on a background task and streams `turn_delta` events. Up to `MAX_CONCURRENT_TURNS` (4) sessions per workspace run at once; each session runs one turn at a time.
- Routing: an explicit `@Agent Name` mention in the prompt, then the agent of the most recent turn, the session's last or bound agent, then the first agent.
- Before and after hooks run other agents serially or in parallel; their output is attached to the turn.
- Durable events persist the session atomically; stream chunks only update memory and the UI. On restart, turns that were running are marked failed and can continue from their checkpoint.
- Stopping a turn aborts its task at the next await point and records it as stopped; it can continue later.
- History: earlier messages are replayed newest first within a token budget; older ones are folded into a summary saved with the session. The running turn's own messages are not replayed.

## Plugins

Plugins are installed per workspace (`.daily/plugins.json`). Built-in plugins render inline cards (chart, weather, hook reports); local plugins contribute module views, renderers, agent tool hints, and event view parsers.

- **Views** mount into Shadow DOM in the app renderer. A plugin has one activation and any number of lazily opened, retained views. Background handlers live in activation; visibility callbacks pause UI-only work.
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
