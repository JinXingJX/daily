# Settings

Daily reads three JSON files and merges them. Edit them from **Settings → Settings files** or with any editor; saving in the app reloads every open window.

| Scope | Path | Share it? |
| --- | --- | --- |
| User | `~/.daily/settings.json` | No — personal defaults and provider keys |
| Project | `<workspace>/.daily/settings.json` | Yes, if you want a team default |
| Local | `<workspace>/.daily/settings.local.json` | No |

**Precedence: local > project > user.** Later scopes override individual fields of earlier ones.

**Provider configurations come only from user and local settings.** `auth.configs` in project settings is ignored (the app shows a warning), so a cloned repository cannot change where your credentials are sent. A project may still pick a provider by id with `auth.selectedAuthConfigId`.

The user file is created from a template on first launch.

## Schema

```jsonc
{
  "agent": {
    "systemPrompt": "…",       // default system prompt for new agents
    "maxIterations": 4,         // model/tool rounds per turn, 1–6
    "dialogueWindow": 8,        // recent messages sent to the model, 1–24
    "memoryWindow": 16          // recent memory entries sent to the model, 1–48
  },
  "model": {
    "model": "gpt-4.1",         // default model of the selected provider
    "temperature": 0.2,         // 0–2
    "maxOutputTokens": 2048     // 128–8192
  },
  "auth": {
    "selectedAuthConfigId": "openai",
    "configs": [
      {
        "id": "openai",
        "name": "OpenAI",
        "providerKind": "open_ai_compatible",
        "baseUrl": "",                 // empty: provider default
        "models": ["gpt-4.1", "gpt-4.1-mini"],
        "defaultModel": "gpt-4.1",
        "apiKey": "sk-…",              // optional
        "apiKeyEnv": "OPENAI_API_KEY", // optional; takes precedence over apiKey
        "codexHome": null,
        "followCodex": false
      }
    ]
  }
}
```

Configurations with the same `id` in a higher scope replace the lower one; new ids are added.

## Provider kinds

| `providerKind` | Authentication | Default `baseUrl` | Key environment variables |
| --- | --- | --- | --- |
| `open_ai_compatible` | API key (optional for local servers) | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `anthropic_messages` | API key | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| `gemini_api` | API key | `https://generativelanguage.googleapis.com/v1beta` | `GEMINI_API_KEY`, `GOOGLE_API_KEY` |
| `codex_login` | Local Codex CLI sign-in (unofficial) | `https://chatgpt.com/backend-api/codex` | — |
| `antigravity_login` | Antigravity 2.x sign-in, macOS only (unofficial) | fixed | — |

For key-based providers the key is resolved in this order: the variable named by `apiKeyEnv`, `apiKey`, then the conventional variables above. When `models` is not empty, a selected model outside the list falls back to `defaultModel`.

For `codex_login`, `codexHome` points at a non-default Codex folder and `followCodex: true` reads the model list and default from it. See [subscription-adapters.md](subscription-adapters.md).

## Agents

Agents are stored in the workspace (`.daily/workspace.json`) and edited in **Settings → Agents**. Each agent picks a provider configuration and model, a temperature, a system prompt, context folders, mounted plugins, and before/after hooks. An agent's provider must exist in the merged settings; otherwise the default provider is used.

## Environment variables

| Variable | Effect |
| --- | --- |
| `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY` | Fallback API keys, see above |
| `DAILY_ANTIGRAVITY_CLIENT_SECRET`, `DAILY_ANTIGRAVITY_CLIENT_ID` | OAuth client used to refresh Antigravity tokens (see [subscription-adapters.md](subscription-adapters.md)) |
| `HTTP_PROXY`, `HTTPS_PROXY` | Proxy for model requests (plugin `network.fetch` never uses a proxy) |
| `CODEX_CA_CERTIFICATE`, `SSL_CERT_FILE` | Extra CA certificates for model requests |
| `DAILY_WORKSPACE` | Development builds only: open this workspace on launch |
| `DAILY_CAPTURE_PATH`, `DAILY_CAPTURE_DELAY_MS` | Development builds only: save a window screenshot after load |
