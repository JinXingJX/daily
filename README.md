# Daily

[![CI](https://github.com/JinXingJX/daily/actions/workflows/ci.yml/badge.svg)](https://github.com/JinXingJX/daily/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[简体中文](README.zh-CN.md)

Daily is a local desktop workbench for running AI agents against a folder on your machine. It pairs an Electron + React interface with a Rust runtime that owns the agent loop, model providers, persistence, and a validated plugin host API.

<!-- Screenshot: add docs/images/workbench.png and reference it here. -->

## Features

- **Persistent tasks.** Each workspace keeps its chat sessions, turns, tool calls, and memory under `<workspace>/.daily/`. Several sessions can run at the same time.
- **Inspectable runs.** Every model request, tool call, and result is recorded as a step you can expand, with the tokens each turn used. Failed or stopped turns continue from their last checkpoint.
- **Bring your own model.** OpenAI-compatible APIs (OpenAI, local servers, gateways), Anthropic, and Gemini with an API key, all with native tool calling and streaming. Optional, unofficial adapters reuse a local Codex CLI or Antigravity sign-in.
- **Agents and hooks.** Configure agents with their own model, prompt, context folders, and plugins. Route a message with `@Agent Name`, and run other agents automatically before or after a turn.
- **Workspace tools.** Agents can list, read, and search files in the workspace (read-only; the private `.daily` folder is hidden). Several reads of one step run in parallel.
- **Standalone multi-agent SDK.** [`daily-agent-core`](crates/daily-agent-core) provides supervision, serial/parallel/DAG workflows, recursive agents, shared budgets and versioned checkpoints; [`daily-agent-providers`](crates/daily-agent-providers) supplies model protocols. Run `cargo run -p daily-agent-core --example framework` offline. The desktop retains the `daily_core::sdk` compatibility entry point.
- **Plugins.** Plugins add retained Shadow DOM views and activation-owned background work, inline renderers, and agent tools. They talk to the host through a typed SDK: storage, workspace files, network fetch, model calls, and structured agent tasks with resumable tool pauses.
- **English and Chinese UI.**

## Status

Daily is early software. macOS on Apple Silicon has been tested locally. CI covers Linux, macOS and Windows; Windows installer and desktop checks are tracked in GitHub Actions. Builds are unsigned and there are no automatic updates yet. Expect breaking changes to the on-disk format before 1.0.

## Quick start

Requirements:

- Windows 10/11 x64, or macOS 14+ on Apple Silicon
- Node.js 22.12 or newer (`.nvmrc` is included)
- A current stable Rust toolchain ([rustup](https://rustup.rs))
- Windows: Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK
- macOS: Xcode Command Line Tools (`xcode-select --install`)

```bash
git clone https://github.com/JinXingJX/daily.git
cd daily
npm ci
npm run dev
```

`npm run dev` builds the Rust sidecar in debug mode and starts Electron with live reload. On first launch Daily asks for a workspace folder; later launches restore the previous windows.

For the App, Core, Writer and Timeline together, follow the [Windows setup guide](docs/windows.md). Keep the local app folder named `daliy` when building the sibling plugins.

## Configure a model

Open **Settings → Model providers**.

### API key providers

Choose **Add API provider** and pick OpenAI compatible, Anthropic, or Gemini. Enter a model ID (for example `gpt-4.1`, `claude-sonnet-4-5`, or `gemini-2.5-pro`) and either paste an API key or leave it empty to read an environment variable:

| Provider | Default base URL | Environment variables |
| --- | --- | --- |
| OpenAI compatible | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| Anthropic | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| Gemini | `https://generativelanguage.googleapis.com/v1beta` | `GEMINI_API_KEY`, `GOOGLE_API_KEY` |

Change the base URL to use a compatible server such as Ollama (`http://localhost:11434/v1`) or a gateway. Keys entered in the UI are saved to `~/.daily/settings.json`, outside your workspaces. See [docs/settings.md](docs/settings.md) for the file format.

### Codex and Antigravity (unofficial)

Daily can also reuse the sign-in of a locally installed Codex CLI or Antigravity 2.x client. **These adapters are unofficial, not affiliated with OpenAI or Google, may violate those services' terms, and can stop working at any time.** Read [docs/subscription-adapters.md](docs/subscription-adapters.md) before enabling them; the Antigravity adapter also needs an OAuth client secret that Daily does not ship.

## Workspaces and data

| Path | Contents |
| --- | --- |
| `<workspace>/.daily/workspace.json` | Task tree and agents |
| `<workspace>/.daily/sessions/` | One file per chat session |
| `<workspace>/.daily/plugins.json` | Installed and enabled plugins |
| `<workspace>/.daily/settings.json` | Project settings (shareable) |
| `<workspace>/.daily/settings.local.json` | Local settings (do not commit) |
| `~/.daily/settings.json` | User settings, including provider keys |

Add `.daily/` to the workspace's `.gitignore` unless you intend to share it. Settings use **local > project > user** precedence; provider endpoints and keys are only read from user and local settings, so a shared repository cannot redirect your credentials.

## Plugins

Install a local plugin from **Settings → Plugins → Install local plugin** with the absolute path to the plugin folder (or its `.daily-plugin/plugin.json`). Optionally enter a `localhost` dev server URL for hot reload.

Enabled plugins can use every host API, so **install only plugins you trust**. Start with [docs/plugin-development.md](docs/plugin-development.md), the typed SDK in [`packages/plugin-sdk`](packages/plugin-sdk), and the minimal example in [`examples/local-writer-plugin`](examples/local-writer-plugin).

## Architecture

```text
React renderer ──contextBridge──▶ Electron main ──JSONL over stdio──▶ daily-app-server
trusted plugin modules ──direct API───┘                                   │
                                                                          ▼
                                     daily-desktop-runtime: desktop plugins, settings, sessions, persistence
                                         ├── daily-agent-core: agent SDK
                                         └── daily-agent-providers: model transports → agent SDK
```

Each workspace window has its own sidecar process. See [docs/architecture.md](docs/architecture.md) and [docs/protocol.md](docs/protocol.md).

## Development

| Command | What it does |
| --- | --- |
| `npm run dev` | Build the debug sidecar and run Electron with live reload |
| `npm run dev:web` | Run the renderer alone (desktop commands unavailable) |
| `npm test` | Rust tests, protocol check, renderer and Electron tests, production build |
| `npm run typecheck` | Type-check renderer, main, and preload |
| `npm run gen:protocol` | Regenerate TypeScript wire types, the IPC allowlist, and the SDK method list from Rust |
| `npm run package:agent-sdk` | Validate and package the standalone Rust SDK source |
| `cargo test --workspace` | Rust tests only |
| `cargo clippy --workspace --all-targets -- -D warnings` | Strict Rust lint |
| `npm run package:win` | Build an unsigned Windows x64 NSIS installer in `release/` |
| `npm run package:mac` | Build an unsigned Apple Silicon ZIP and DMG in `release/` |

For automated UI checks, development builds accept `DAILY_WORKSPACE=/absolute/path` (open that workspace) and `DAILY_CAPTURE_PATH=/tmp/shot.png` (save a screenshot after load).

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## Known limitations

- Builds are unsigned. Windows CI validates native builds and sidecar behavior; interactive desktop behavior still needs a Windows smoke test.
- Antigravity sign-in depends on macOS Keychain and is unavailable on Windows; API-key providers work across platforms.
- Agent system prompts and built-in model prompts are English.
- The plugin marketplace is a static demo list.
- Up to 4 sessions per workspace run at once.
- The built-in agent tools only read the workspace. There is no MCP client and no built-in tool that writes files or runs commands; plugins can add tools of their own.
- Token budgets for history and tool results are estimated, not counted with each provider's tokenizer.

## Disclaimer

Daily is provided as is, under the MIT license. You are responsible for how you use connected model providers and for complying with their terms. The Codex and Antigravity adapters are unofficial and unsupported by their vendors.

## License

[MIT](LICENSE) © 2026 JinXingJX
