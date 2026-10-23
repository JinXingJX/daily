# Security policy

## Reporting a vulnerability

Please do not open a public issue. Report vulnerabilities privately through [GitHub security advisories](https://github.com/JinXingJX/daily/security/advisories/new). Include the affected version or commit, steps to reproduce, and the impact you observed. You should get a response within a week.

Only the latest release and `main` receive security fixes.

## Trust model

Daily runs AI agents and plugins on your machine with access to a workspace folder and your model provider credentials. Understanding what is and is not protected helps you use it safely.

**Trusted:**

- **Plugins you enable.** There is no per-capability permission system. An enabled plugin can read and write workspace files, fetch public URLs, invoke your models, and run agents on your providers. Install only plugins you trust.
- **Your user settings** (`~/.daily/settings.json`), which hold provider endpoints and keys.

**Not trusted, and defended against:**

- **Workspace contents.** Opening a folder you did not create (for example a cloned repository) must not leak your credentials or run code:
  - provider configurations in a workspace's project `settings.json` are ignored, so a repository cannot change where your keys go;
  - local development plugins require an absolute manifest path and a `localhost` dev server URL, so `.daily/plugins.json` from a repository cannot load code it ships or hosts remotely.
- **Plugin frames versus the app.** Plugin views run in sandboxed iframes with a strict CSP. They cannot reach Node.js, Electron IPC, or the app DOM, and every host call is validated in Rust: paths cannot escape the workspace or reach `.daily`, payloads and collections are size-limited, and writes are atomic.
- **Plugin network access.** `network.fetch` only reaches public HTTP(S) addresses, re-checks DNS on every redirect (no private, loopback, or link-local targets), ignores proxies, strips credentials on cross-origin redirects, and caps responses at 2 MiB.
- **The renderer.** Context isolation and sandboxing are on, Node.js integration is off, webviews are disabled, IPC is accepted only from the main frame of a registered window, and only allowlisted commands reach the sidecar.

**Out of scope:**

- What a model decides to do with the read-only workspace tools or with a plugin's tools.
- Content a model or plugin returns. Review agent output before acting on it.
- The unofficial Codex and Antigravity adapters' standing with those services (see [docs/subscription-adapters.md](docs/subscription-adapters.md)).

## Hardening tips

- Add `.daily/` to the workspace's `.gitignore`; it contains session transcripts and local settings.
- Prefer `apiKeyEnv` over storing keys in files you might share.
- Keep Daily updated; builds are currently unsigned, so verify you downloaded them from this repository's releases.
