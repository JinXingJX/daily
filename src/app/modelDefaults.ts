export const DEFAULT_CODEX_AUTH_CONFIG_ID = "codex-default";
export const DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID = "antigravity-default";
export const DEFAULT_ANTIGRAVITY_BASE_URL = "https://daily-cloudcode-pa.googleapis.com/v1internal";
export const DEFAULT_CODEX_BASE_URL = "https://chatgpt.com/backend-api/codex";
export const DEFAULT_CODEX_MODEL = "gpt-5.5";
export const DEFAULT_CODEX_MODELS = [
  "gpt-5.5",
  "gpt-5.5-low",
  "gpt-5.5-medium",
  "gpt-5.5-high",
  "gpt-5.5-xhigh",
  "gpt-5.4",
  "gpt-5.4-low",
  "gpt-5.4-medium",
  "gpt-5.4-high",
  "gpt-5.4-xhigh",
  "gpt-5.4-mini",
  "gpt-5.4-mini-low",
  "gpt-5.4-mini-medium",
  "gpt-5.4-mini-high",
  "gpt-5.4-mini-xhigh",
  "gpt-5.3-codex",
  "gpt-5.2",
  "codex-auto-review",
];

export function parseModelList(value: string, defaultModel: string) {
  const models = [defaultModel, ...value.split(/[\n,]/)]
    .map((model) => model.trim())
    .filter(Boolean);
  return Array.from(new Set(models.length ? models : DEFAULT_CODEX_MODELS));
}
