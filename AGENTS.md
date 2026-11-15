# Repository Guidelines

## Project Structure & Module Organization

Daily is an Electron desktop app with a React/Vite renderer and a Rust sidecar:

- `electron/main/` owns windows, menus, lifecycle, workspace restoration, IPC validation, and sidecar supervision: `workspaceManager.ts` (windows, sidecars, close/quit checks), `sidecar.ts` (JSONL client), `ipc.ts`, `menu.ts`, `security.ts`, `windowState.ts`, `i18n.ts`; `index.ts` only bootstraps.
- `electron/preload/` exposes the narrow `window.dailyDesktop` bridge through `contextBridge`.
- `src/` contains the renderer. `App.tsx` owns the shell, `app/routes/` and `app/chrome/` hold route and window components, `useAppStore.ts` owns state with pure reducers in `app/turnDelta.ts`, `features/` contains feature surfaces (chat, settings sections, plugins, workspace), `components/ui/` contains primitives, `platform/desktop.ts` isolates desktop calls, `i18n/` holds the `t()` helper and the zh-CN catalog, and `styles/` holds the stylesheet sections imported by `index.css`.
- `packages/plugin-sdk/` is `@daily/plugin-sdk`: typed module API (`index.ts`), the direct host API transport (`host.ts`), the generated RPC method list, and `localizeHostText`.
- Generated from Rust by `npm run gen:protocol` (never edit by hand): `src/generated/protocol/`, `electron/shared/commands.generated.ts`, `packages/plugin-sdk/src/methods.generated.ts`.
- `crates/daily-app-server/` implements the bounded JSONL stdio protocol.
- `crates/daily-agent-core/` is the standalone multi-agent SDK: model contract, model/tool loop, tools, transcript, compaction, supervision, workflows, recursive delegation, shared budgets and versioned checkpoints. It must not depend on HTTP, providers, desktop state, plugins or a database.
- `crates/daily-agent-providers/` owns native model protocols and HTTP transports. `ProviderModel` implements the core model contract. Unofficial CLI/desktop login adapters require the optional `login` feature.
- `crates/daily-desktop-runtime/` is the desktop backend: `runtime/` (DailyRuntime and command handlers), `state/` (workspace and sessions), `agent/` (desktop SDK compatibility facade, events, turns/hooks and workspace tools), `plugins/` (manifests, host RPC, plugin sessions and pending runs), `model/` (desktop settings/view types and provider compatibility), `settings/`, `net/` (desktop HTTP + SSRF), `fs_utils/`, and `tasks.rs` (SQLite plugin task runtime). It explicitly enables provider login support.
- `crates/daily-core/` is a compatibility re-export of `daily-desktop-runtime`; app-server depends directly on the desktop runtime.
- `docs/` has the architecture, protocol, settings, plugin development, and subscription adapter docs. `examples/` has a minimal plugin.
- `build/`, `scripts/`, and the `build` section of `package.json` contain packaging assets and configuration. `out/`, `target/`, and `release/` are generated.

## Build, Test, and Development Commands

- `npm install`: install exact JavaScript dependencies from `package-lock.json`.
- `npm run dev`: build the debug sidecar and launch Electron with live renderer reload.
- `npm run dev:web`: run the renderer alone; desktop commands are unavailable.
- `npm run typecheck`: check renderer, Electron main, and preload TypeScript.
- `npm run gen:protocol`: regenerate TS wire types, the IPC command allowlist, and the SDK method list from Rust; `npm run check:protocol` fails if they are stale.
- `npm run test:renderer`: run renderer Vitest unit tests.
- `npm run build`: type-check and build all Electron bundles.
- `npm run test:electron`: build the sidecar and run Electron protocol integration tests.
- `npm test`: run Rust tests, the protocol check, renderer and Electron tests, and the production build.
- `cargo test --workspace`: run backend tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: require warning-free Rust.
- `cargo test -p daily-agent-core -p daily-agent-providers`: test the SDK without desktop/login feature unification.
- `npm run check:agent-sdk`: run a separate Cargo consumer using cached dependencies and verify SDK isolation.
- `npm run package:agent-sdk`: validate and package the two standalone SDK crates as a source archive.
- `npx tsc -p packages/plugin-sdk/tsconfig.json`: type-check the SDK.
- `npm run package:win`: build the native Windows x64 sidecar and unsigned NSIS installer.
- `npm run package:mac`: build the arm64 sidecar, bundles, ZIP, and unsigned DMG.

## Coding Style & Naming Conventions

Use strict TypeScript and React function components with two-space indentation. Use `PascalCase` for components/exported types, `useSomething` for hooks, and `camelCase` for local values. Prefer `@/` imports in renderer code, reuse `components/ui/`, and use `cn()` for class composition.

UI text is English passed to `t()` from `@/i18n`, with the Chinese translation in `src/i18n/zh-CN.ts` (a missing key fails type-checking). Rust user-visible text is English; runtime text the UI displays needs a template in `packages/plugin-sdk/src/hostText.ts`.

Rust follows `rustfmt` defaults. Add a desktop command by declaring it in `desktop_commands!` (runtime/mod.rs) and regenerating the protocol. Shared SDK/provider wire types use optional `ts` derives and are explicitly exported by `daily-desktop-runtime/src/protocol_exports.rs`; desktop types retain automatic exports. New plugin RPC methods go in `PLUGIN_RPC_METHODS` and the dispatcher, then `packages/plugin-sdk/src/host.ts` and `DailyPluginApi`. Keep commands thin. Put generic agent execution in `daily-agent-core`, model protocols in `daily-agent-providers`, and desktop validation, persistence and plugin behavior in `daily-desktop-runtime`.

## Testing Guidelines

Add Rust unit tests beside the changed module and Electron integration tests under `electron/**/*.test.ts`. Renderer tests run with the zh-CN locale (`src/test/setup.ts`). Renderer changes require `npm run typecheck` and `npm run build`; backend changes require `cargo test --workspace` and strict Clippy. Security fixes should include a regression test at the lowest boundary that enforces the invariant.

## Commit & Pull Request Guidelines

Prefer concise imperative commits with `feat:`, `fix:`, `refactor:`, `docs:`, or `chore:` when useful. Pull requests should state behavior changes, validation commands, linked issues, and UI screenshots or recordings where applicable.

## Security & Configuration Tips

Keep renderer `nodeIntegration` disabled, `contextIsolation` and sandboxing enabled, IPC main-frame-only, and plugin views in Shadow DOM. Plugin ES modules are trusted code in the app renderer; background work belongs to activation, with no iframe or extra BrowserWindow. Enabled plugins are trusted with the full host API (there is no permission system); the Rust host still validates every RPC. Preserve path canonicalization/symlink checks, size and collection limits, atomic persistence, SSRF/DNS-rebinding defenses, bounded protocol frames, the user/local-only rule for provider configurations, and the absolute-path and localhost-only rules for development plugins.

Do not commit `.daily/`, `*.local`, API keys, `out/`, `target/`, or `release/`. Provider keys belong in user or local settings or environment variables (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`); `CODEX_CA_CERTIFICATE` and `SSL_CERT_FILE` add CAs for model requests.
