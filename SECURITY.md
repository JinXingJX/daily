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
- **Plugins and the app.** Plugins are trusted ES modules in the app renderer. They can access the app DOM and share its event loop; Shadow DOM provides style isolation only. Host RPC still validates paths, payloads and enablement, uses atomic writes and keeps provider credentials in Rust. Those checks protect host operations; they are not a sandbox between plugins and the app.

**Out of scope:**

- What a model decides to do with the read-only workspace tools or with a plugin's tools.
- Content a model or plugin returns. Review agent output before acting on it.
- The unofficial Codex and Antigravity adapters' standing with those services (see [docs/subscription-adapters.md](docs/subscription-adapters.md)).

## Hardening tips

- Add `.daily/` to the workspace's `.gitignore`; it contains session transcripts and local settings.
- Provider keys you enter in the app are stored in plain text in `~/.daily/settings.json` (or a workspace's `settings.local.json`). On Unix, Daily restricts these files to your account. On Windows, files inherit the folder ACL, so keep user settings in your private Windows profile and protect workspace-local settings. Anything running as you can read them. Prefer `apiKeyEnv`, which keeps the key out of the file.
- Keep Daily updated; builds are currently unsigned, so verify you downloaded them from this repository's Actions artifacts or releases.
