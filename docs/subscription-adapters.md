# Codex and Antigravity adapters (unofficial)

> **Disclaimer.** These adapters are unofficial. They are not affiliated with, endorsed by, or supported by OpenAI or Google. Using a consumer subscription through a third-party client may violate the provider's terms of service and could put your account at risk. The services can change at any time and break these adapters without notice. You use them at your own risk. If you need a supported setup, use an API key provider instead.

Both adapters are optional. Daily works with API keys alone (see [settings.md](settings.md)).

## Codex

The Codex adapter reuses the sign-in of the official [Codex CLI](https://github.com/openai/codex).

- **Setup:** install the Codex CLI and sign in (`codex login`), or click **Sign in** in **Settings → Model providers → Codex**, which starts the CLI's login.
- **Credentials:** Daily reads `auth.json` from the Codex folder (`~/.codex`, `CODEX_HOME`, or the folder set in the provider's `codexHome`) and refreshes tokens the way the CLI does. Tokens stay on your machine and are sent only to the configured Codex endpoint.
- **Models:** with **Follow Codex models** on, the model list and default come from the CLI's `models_cache.json` and `config.toml`.
- **Requests** identify the client as Codex CLI with a `Daily` suffix in the user agent.
- **Troubleshooting:** connection errors usually come from the network, proxy, or TLS setup. Daily honors `HTTP_PROXY`/`HTTPS_PROXY` and extra CAs from `CODEX_CA_CERTIFICATE` or `SSL_CERT_FILE`.

## Antigravity

The Antigravity adapter reuses the sign-in of the official Antigravity 2.x desktop client on macOS.

- **Requirements:** macOS, Antigravity 2.x, a personal Google account, and the OAuth client secret of the Antigravity desktop app's Google registration in `DAILY_ANTIGRAVITY_CLIENT_SECRET`. Daily does not ship that secret; without it, sign-in works until the current access token expires and refreshing then fails with an explanatory error. `DAILY_ANTIGRAVITY_CLIENT_ID` overrides the client id if it ever changes. Workspace (enterprise) accounts and image inputs are not supported.
- **Setup:** select the Antigravity provider, click **Sign in with Antigravity**, finish signing in to the official client, then click **Refresh** to load the model list and save.
- **Credentials:** Daily reads the current account from the macOS Keychain entry the client creates (macOS may ask you to allow access). Refreshed tokens are kept only in the sidecar's memory; Daily never writes them to settings and never modifies the client's own credentials.
- **Endpoint:** requests go only to the Antigravity gateway; the base URL cannot be changed.
- **Client identity:** token refresh presents the Antigravity desktop app's OAuth client (with the secret you supply), and requests send an Antigravity-style user agent. This is how the adapter can use the official sign-in, and is the main reason it is unofficial.
- **Live test** (requires a signed-in client):

  ```bash
  cargo test -p daily-core live_antigravity_catalog_and_models -- --ignored --nocapture
  ```

## Removing an adapter

Delete its entry from `auth.configs` in `~/.daily/settings.json` (or use **Delete** in the UI for API providers). Each agent keeps its own provider binding; agents bound to a removed provider fall back to the default provider.

## Code

- `crates/daily-core/src/model/codex/` — Codex auth, catalog, and streaming
- `crates/daily-core/src/model/antigravity/` — Keychain auth, catalog, and streaming
