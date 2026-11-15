# Changelog

All notable changes are recorded here. The project follows [Semantic Versioning](https://semver.org); before 1.0, minor versions may include breaking changes.

## [0.1.0] - Unreleased

First open-source release.

### Added

- Electron + React workbench with a Rust sidecar per workspace window.
- Persistent sessions with inspectable steps, stop and continue-from-checkpoint, and up to 4 concurrently running sessions per workspace.
- Model providers: OpenAI-compatible, Anthropic, and Gemini with API keys (from settings or environment variables), plus unofficial Codex and Antigravity sign-in adapters.
- Agents with their own model, prompt, context folders, mounted plugins, and before/after hooks; explicit `@Agent Name` routing.
- Read-only workspace tools for agents: list, read, and search files.
- Plugin system with sandboxed main and background views, inline renderers, and `@daily/plugin-sdk`: storage, private files, workspace files, guarded network fetch, model calls, structured agent tasks with async tool pauses, sub-agents, and host-owned agent sessions.
- English and Simplified Chinese interface.
- Documentation: architecture, protocol, settings, plugin development, and the subscription adapter disclaimer.

### Changed

- Tool rounds are sent back to every provider in its native tool-call format, with the reasoning it returned, instead of as text in the user message. The request prefix stays identical between steps, so provider caches hit.
- Anthropic and Gemini adapters call tools; OpenAI-compatible, Anthropic and Gemini adapters stream. The OpenAI-compatible adapter now sends the configured output limit, and the Anthropic adapter no longer sends sampling parameters current Claude models reject.
- Tools are async with timeouts and cancellation; read-only tools of one step run in parallel. A failed or unknown tool is reported to the model instead of failing the turn.
- Model errors are typed and transient ones are retried. Turns record token usage and a failure code; an answer cut off at the output limit is marked.
- Chat turns default to 100 steps (up to 1000) and 32,000 output tokens (up to 128,000, lowered automatically for models that allow less). Long sessions and long turns are summarized within token budgets.
- `daily_core::sdk` is a public library API with an example.

### Fixed

- The user's message was sent to the model twice per request, and the error text of a failed attempt was replayed as something the agent had said.

### Security
- User and local settings files, which can hold provider keys, are created readable by the owner only; existing files are tightened on launch.

- Provider configurations are read only from user and local settings, never from a workspace's project settings.
- Local development plugins require absolute manifest paths and `localhost` dev server URLs.
