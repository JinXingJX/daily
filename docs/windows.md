# Windows: App, Core, Writer and Timeline

Daily uses three repositories. The Rust Core and model providers live in the
Daily workspace; they do not need a fourth checkout. Both plugins use the shared
SDK at `../daliy/packages/plugin-sdk`, so keep the directory names below.

## Requirements

- Windows 10/11 x64 and Git.
- Node.js 22.12 or later, including npm.
- Stable Rust installed with rustup, using the `x86_64-pc-windows-msvc` toolchain.
- Visual Studio Build Tools: **Desktop development with C++**, MSVC and a Windows SDK.

Restart PowerShell after installing the toolchains. You can verify them with
`node --version`, `npm --version`, `rustc --version` and `cargo --version`.

## Clone and build

Run these commands in PowerShell. If PowerShell blocks `npm.ps1`, use `npm.cmd`
instead of `npm` in the same commands.

```powershell
mkdir daily-workspace
cd daily-workspace
git clone https://github.com/JinXingJX/daily.git daliy
git clone https://github.com/JinXingJX/daily-writer-plugin.git
git clone https://github.com/JinXingJX/trading-timeline-plugin.git

cd daliy
npm ci
npm test
cd ../daily-writer-plugin
npm ci
npm test
npm run build
cd ../trading-timeline-plugin
npm ci
npm test
npm run build
cd ../daliy
npm run dev
```

The first build compiles Rust and SQLite and can take several minutes. Do not
copy `node_modules`, `target`, `out` or macOS application bundles to Windows.
Build native dependencies on the target machine.

## Install the plugins

Choose a workspace in Daily. In **Settings → Plugins → Install local plugin**,
enter the absolute path to each plugin folder, for example:

```text
C:\src\daily-workspace\daily-writer-plugin
C:\src\daily-workspace\trading-timeline-plugin
```

Use your actual checkout location. Leave Dev URL empty to load the production
bundles, then enable both plugins. The build output must remain in
`.daily-plugin/dist` beside each plugin's manifest.

For hot reload, run `npm run dev` in each plugin folder and use
`http://127.0.0.1:5174` for Writer or `http://127.0.0.1:5175` for Timeline.

## Models and data

Configure an API-key provider in **Settings → Model providers**. OpenAI compatible,
Anthropic and Gemini providers work across platforms. Model calls and image
generation use your provider account; CI tests use fixtures and local servers.

The optional Antigravity subscription adapter reads macOS Keychain and is not
available on Windows. A local Codex sign-in can be read from the configured
Codex home; see [subscription adapters](subscription-adapters.md).

User settings are under `%USERPROFILE%\.daily\settings.json`. Workspace state,
plugin data and manuscripts are under `<workspace>\.daily`. These paths are
private runtime data and must not be committed. Back them up before moving a
workspace between machines; copy the whole workspace, not just its source files.

## Windows installer

On Windows, run:

```powershell
npm run package:win
```

The NSIS installer is written to `release\Daily-<version>-x64.exe`. It includes
the native Rust sidecar at `resources\bin\daily-app-server.exe`; users of the
installer do not need Node.js or Rust. The plugins are installed separately as
described above.

The **CI** workflow also produces a `Daily-windows-x64` artifact after native
tests and packaging pass. Open the successful Actions run and download it.
Plugin workflows provide `Writer-plugin` and `Timeline-plugin` artifacts:
extract each into its own folder and install that folder's `plugin.json` in Daily.
Artifacts expire after seven days. They are unsigned builds, so Windows may
show an unknown-publisher prompt. No signing certificate is required to build.

## Verification scope

CI runs Rust tests and strict Clippy, protocol regeneration checks, renderer
tests, real sidecar handshakes in paths containing spaces and Chinese, TypeScript
checks and production builds on standard Linux, macOS and Windows runners. It
also validates the standalone Core SDK outside the desktop dependency graph.

Native CI establishes build and backend compatibility. Before using a release
for daily work, check on Windows that you can open a workspace, install both
plugins, create and reopen a Writer document, load Timeline, and switch views
while a background task is running. Live provider behavior depends on your
configuration and is not established by offline fixture tests.
