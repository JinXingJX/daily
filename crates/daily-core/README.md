# Daily desktop compatibility crate

`daily-core` re-exports `DailyRuntime`, desktop commands and the legacy `sdk` API from `daily-desktop-runtime`. It remains available for existing desktop callers; the app-server uses `daily-desktop-runtime` directly.

New CLI/service integrations should depend on `daily-agent-core` and optionally `daily-agent-providers`, which do not require the desktop runtime, settings, SQLite or plugins. See [the framework guide](../daily-agent-core/README.md).
