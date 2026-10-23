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

### Security

- Provider configurations are read only from user and local settings, never from a workspace's project settings.
- Local development plugins require absolute manifest paths and `localhost` dev server URLs.
