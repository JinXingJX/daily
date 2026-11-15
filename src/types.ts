// Wire types are generated from the Rust sidecar; run `npm run gen:protocol` after
// changing a serialized Rust type. Only renderer-specific aliases and helpers live here.
import type {
  AgentHookMode,
  ModelAuthConfig,
  ModelProviderKind,
  PluginManifestView,
} from "./generated/protocol";

export type * from "./generated/protocol";

export type ProviderKind = ModelProviderKind;
export type HookMode = AgentHookMode;
export type AuthConfigView = ModelAuthConfig;
export type PluginManifest = PluginManifestView;

export type ComposerImage = {
  name: string;
  mimeType: string;
  dataBase64: string;
};

export function providerLabel(kind: ProviderKind) {
  switch (kind) {
    case "open_ai_compatible":
      return "OpenAI Compatible";
    case "anthropic_messages":
      return "Anthropic Messages";
    case "gemini_api":
      return "Gemini API";
    case "codex_api_key":
      return "Codex API (Responses)";
    case "codex_login":
      return "Codex Login";
    case "antigravity_login":
      return "Antigravity Login";
  }
}

export function formatTimestamp(timestamp: number) {
  if (!timestamp) {
    return "Never";
  }
  const value = new Date(timestamp * 1000);
  return value.toLocaleString();
}
