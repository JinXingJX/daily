# Settings

Daily reads three JSON files and merges them. Edit them from **Settings → Settings files** or with any editor; saving in the app reloads every open window.

| Scope | Path | Share it? |
| --- | --- | --- |
| User | `~/.daily/settings.json` | No — personal defaults and provider keys |
| Project | `<workspace>/.daily/settings.json` | Yes, if you want a team default |
| Local | `<workspace>/.daily/settings.local.json` | No |

**Precedence: local > project > user.** Later scopes override individual fields of earlier ones.

User and local settings can hold provider keys in plain text, so on Unix Daily creates them readable by your account only (mode `0600`, and `0700` for `~/.daily`) and tightens older files on launch. On Windows, files inherit the directory ACL; store your user settings in your private Windows profile. Use `apiKeyEnv` to keep a key out of the file altogether.

**How much history the model sees.** Earlier messages of a session are replayed newest first while they fit a budget of about 32,000 tokens; the newest `dialogueWindow` messages are always included. What no longer fits is folded into a summary that the turn's model writes and the session keeps. Within a turn, tool results beyond about 48,000 tokens are summarized the same way, keeping the newest ones verbatim. Token counts are estimates.

**Provider configurations come only from user and local settings.** `auth.configs` in project settings is ignored (the app shows a warning), so a cloned repository cannot change where your credentials are sent. A project may still pick a provider by id with `auth.selectedAuthConfigId`.

The user file is created from a template on first launch.

## Schema

```jsonc
{
  "agent": {
    "systemPrompt": "…",       // default system prompt for new agents
    "maxIterations": 100,       // model requests per turn, 1–1000; the last one must answer
    "dialogueWindow": 8,        // newest messages always sent to the model, 1–24 (see below)
    "memoryWindow": 16          // recent memory entries sent to the model, 1–48
  },
  "model": {
    "model": "gpt-4.1",         // default model of the selected provider
    "temperature": 0.2,         // 0–2
    "maxOutputTokens": 32000    // 128–128000; lowered for you when the model allows less
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
| `codex_api_key` | API key, Responses API | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| `anthropic_messages` | API key | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| `gemini_api` | API key | `https://generativelanguage.googleapis.com/v1beta` | `GEMINI_API_KEY`, `GOOGLE_API_KEY` |
| `codex_login` | Local Codex CLI sign-in (unofficial) | `https://chatgpt.com/backend-api/codex` | — |
| `antigravity_login` | Antigravity 2.x sign-in, macOS only (unofficial) | fixed | — |

For key-based providers the key is resolved in this order: the variable named by `apiKeyEnv`, `apiKey`, then the conventional variables above. When `models` is not empty, a selected model outside the list falls back to `defaultModel`.

For `codex_login`, `codexHome` points at a non-default Codex folder and `followCodex: true` reads the model list and default from it. See [subscription-adapters.md](subscription-adapters.md).

For Codex with an API key, choose **Settings → Model providers → Add API provider → Codex API (Responses)**. Enter your key, default model and optional Base URL, then save. Requests use `POST /responses` with Bearer authentication and support streaming, function calls and image inputs. A Base URL may be the API root (for example `https://api.openai.com/v1`) or the full `/responses` URL. This mode does not require Codex CLI login or read its credentials; `codex_login` continues to use ChatGPT subscription tokens. API key usage is billed to your API account.

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
