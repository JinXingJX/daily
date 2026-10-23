import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID,
  DEFAULT_ANTIGRAVITY_BASE_URL,
  DEFAULT_CODEX_AUTH_CONFIG_ID,
  DEFAULT_CODEX_BASE_URL,
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_MODELS,
  parseModelList,
} from "@/app/modelDefaults";
import type {
  AntigravityAuthStatusView,
  AntigravityLoginStartView,
  AntigravityModelCatalog,
  AppSnapshot,
  AuthConfigView,
  CodexAuthStatusView,
  CodexLoginStartView,
  ProviderKind,
  SettingsScope,
  SettingsView,
} from "@/types";
import { pollAuthentication } from "@/app/authPolling";
import { invoke, type DesktopCommand } from "@/platform/desktop";
import { t } from "@/i18n";

type RunSnapshotCommand = <T,>(command: DesktopCommand, args?: T) => Promise<AppSnapshot>;

export function authConfigOptions(configs: AuthConfigView[]): AuthConfigView[] {
  const options = [...configs];
  if (!options.some((config) => config.providerKind === "codex_login")) {
    options.push({ id: DEFAULT_CODEX_AUTH_CONFIG_ID, name: "Codex", providerKind: "codex_login", baseUrl: DEFAULT_CODEX_BASE_URL,
      codexHome: null, models: [...DEFAULT_CODEX_MODELS], defaultModel: DEFAULT_CODEX_MODEL, followCodex: true });
  }
  if (!options.some((config) => config.providerKind === "antigravity_login")) {
    options.push({ id: DEFAULT_ANTIGRAVITY_AUTH_CONFIG_ID, name: "Antigravity", providerKind: "antigravity_login", baseUrl: DEFAULT_ANTIGRAVITY_BASE_URL,
      codexHome: null, models: [], defaultModel: "", followCodex: false });
  }
  return options;
}

/** Providers authenticated with an API key rather than a local client login. */
export function usesApiKey(kind: ProviderKind) {
  return kind === "open_ai_compatible" || kind === "anthropic_messages" || kind === "gemini_api";
}

export const API_PROVIDER_DEFAULTS: Record<"open_ai_compatible" | "anthropic_messages" | "gemini_api", { slug: string; name: string; keyEnv: string }> = {
  open_ai_compatible: { slug: "openai", name: "OpenAI compatible", keyEnv: "OPENAI_API_KEY" },
  anthropic_messages: { slug: "anthropic", name: "Anthropic", keyEnv: "ANTHROPIC_API_KEY" },
  gemini_api: { slug: "gemini", name: "Gemini", keyEnv: "GEMINI_API_KEY" },
};

export function useSettingsDrafts({ settings, selectedScope, runSnapshotCommand, setCommandBusy, setError }: {
  settings: SettingsView | null;
  selectedScope: SettingsScope;
  runSnapshotCommand: RunSnapshotCommand;
  setCommandBusy: (command: string, busy: boolean) => void;
  setError: Dispatch<SetStateAction<string | null>>;
}) {
  const [scopeDrafts, setScopeDrafts] = useState<Record<SettingsScope, string>>({ user: "", project: "", local: "" });
  const [codexAuthStatus, setCodexAuthStatus] = useState<CodexAuthStatusView | null>(null);
  const [antigravityAuthStatus, setAntigravityAuthStatus] = useState<AntigravityAuthStatusView | null>(null);
  const [authConfigIdDraft, setAuthConfigIdDraft] = useState(DEFAULT_CODEX_AUTH_CONFIG_ID);
  const [authNameDraft, setAuthNameDraft] = useState("Codex");
  const [authBaseUrlDraft, setAuthBaseUrlDraft] = useState(DEFAULT_CODEX_BASE_URL);
  const [authDefaultModelDraft, setAuthDefaultModelDraft] = useState(DEFAULT_CODEX_MODEL);
  const [authModelsDraft, setAuthModelsDraft] = useState(DEFAULT_CODEX_MODELS.join("\n"));
  const [codexHomeDraft, setCodexHomeDraft] = useState("");
  const [codexFollowDraft, setCodexFollowDraft] = useState(true);
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [apiKeyEnvDraft, setApiKeyEnvDraft] = useState("");
  const [authLoginPending, setAuthLoginPending] = useState(false);
  const pollGeneration = useRef(0);
  const draftIdRef = useRef(authConfigIdDraft);
  draftIdRef.current = authConfigIdDraft;
  const cancelAuthPolling = useCallback(() => {
    pollGeneration.current += 1;
    setAuthLoginPending(false);
  }, []);

  const options = useMemo(() => authConfigOptions(settings?.effective.model.authConfigs ?? []), [settings?.effective.model.authConfigs]);
  const selectedAuthConfig = useMemo<AuthConfigView | null>(() => {
    const model = settings?.effective.model;
    return model?.authConfigs.find((config) => config.id === model.authConfigId) ?? model?.authConfigs[0] ?? null;
  }, [settings?.effective.model]);
  const draftAuthConfig = options.find((config) => config.id === authConfigIdDraft) ?? options[0];
  const isAntigravity = draftAuthConfig.providerKind === "antigravity_login";
  const isApiKey = usesApiKey(draftAuthConfig.providerKind);
  const draftSyncKey = JSON.stringify(draftAuthConfig);

  useEffect(() => {
    if (selectedAuthConfig) setAuthConfigIdDraft(selectedAuthConfig.id);
  }, [selectedAuthConfig?.id]);

  useEffect(() => {
    if (!settings) return;
    setScopeDrafts({
      user: settings.scopes.find((scope) => scope.scope === "user")?.content ?? "",
      project: settings.scopes.find((scope) => scope.scope === "project")?.content ?? "",
      local: settings.scopes.find((scope) => scope.scope === "local")?.content ?? "",
    });
  }, [settings?.scopes]);

  useEffect(() => {
    setAuthNameDraft(draftAuthConfig.name);
    setAuthBaseUrlDraft(draftAuthConfig.baseUrl);
    setAuthDefaultModelDraft(draftAuthConfig.defaultModel);
    setAuthModelsDraft(draftAuthConfig.models.join("\n"));
    setCodexHomeDraft(draftAuthConfig.codexHome ?? "");
    setCodexFollowDraft(draftAuthConfig.followCodex);
    setApiKeyDraft(draftAuthConfig.apiKey ?? "");
    setApiKeyEnvDraft(draftAuthConfig.apiKeyEnv ?? "");
    setCodexAuthStatus(null);
    setAntigravityAuthStatus(null);
  }, [draftSyncKey]);

  useEffect(() => { cancelAuthPolling(); }, [authConfigIdDraft, cancelAuthPolling]);
  useEffect(() => cancelAuthPolling, [cancelAuthPolling]);

  async function handleScopeSave() {
    await runSnapshotCommand("save_settings_file_content", { input: { scope: selectedScope, content: scopeDrafts[selectedScope] ?? "" } });
  }

  async function refreshStatus(id = authConfigIdDraft, codexHome = codexHomeDraft) {
    if (isApiKey) return null;
    if (isAntigravity) {
      const status = await invoke<AntigravityAuthStatusView>("get_antigravity_auth_status");
      if (draftIdRef.current === id) setAntigravityAuthStatus(status);
      return status;
    }
    const status = await invoke<CodexAuthStatusView>("get_codex_auth_status", { input: { codexHome } });
    if (draftIdRef.current === id) setCodexAuthStatus(status);
    return status;
  }

  async function refreshCatalog(id = authConfigIdDraft) {
    const catalog = await invoke<AntigravityModelCatalog>("get_antigravity_model_catalog");
    if (draftIdRef.current === id) {
      setAuthModelsDraft(catalog.models.join("\n"));
      setAuthDefaultModelDraft((current) => catalog.models.includes(current) ? current : catalog.defaultModel);
    }
    return catalog;
  }

  async function handleRefreshAuth() {
    setCommandBusy("get_antigravity_model_catalog", true);
    setError(null);
    try {
      const status = await refreshStatus();
      if (isAntigravity && status?.authenticated) {
        await refreshCatalog();
        await refreshStatus();
      }
      return status;
    } catch (error) {
      setError(error instanceof Error ? error.message : t("Could not read the sign-in status."));
      if (isAntigravity) await refreshStatus().catch(() => null);
      return null;
    } finally {
      setCommandBusy("get_antigravity_model_catalog", false);
    }
  }

  // Read only while the authentication panel is mounted, avoiding Keychain prompts on app launch.
  const refreshAuthStatusOnOpen = useCallback(() => {
    if (isApiKey) return;
    const generation = pollGeneration.current;
    const id = authConfigIdDraft;
    void refreshStatus(id, draftAuthConfig.codexHome ?? "").catch((error: unknown) => {
      if (pollGeneration.current === generation) setError(error instanceof Error ? error.message : t("Could not read the sign-in status."));
    });
  }, [authConfigIdDraft, isAntigravity, isApiKey, draftAuthConfig.codexHome]);

  function authSettingsInput(catalog?: AntigravityModelCatalog) {
    if (isApiKey) {
      const models = Array.from(new Set([authDefaultModelDraft, ...authModelsDraft.split(/[\n,]/)].map((m) => m.trim()).filter(Boolean)));
      return { config: {
        id: draftAuthConfig.id, name: authNameDraft, providerKind: draftAuthConfig.providerKind,
        baseUrl: authBaseUrlDraft, codexHome: null, defaultModel: authDefaultModelDraft.trim(), models, followCodex: false,
        apiKey: apiKeyDraft.trim(), apiKeyEnv: apiKeyEnvDraft.trim() || null,
      } };
    }
    const models = isAntigravity ? (catalog?.models ?? authModelsDraft.split("\n").map((m) => m.trim()).filter(Boolean)) : parseModelList(authModelsDraft, authDefaultModelDraft);
    const defaultModel = catalog && !catalog.models.includes(authDefaultModelDraft) ? catalog.defaultModel : authDefaultModelDraft;
    return { config: {
      id: draftAuthConfig.id, name: authNameDraft, providerKind: draftAuthConfig.providerKind,
      baseUrl: authBaseUrlDraft, codexHome: isAntigravity ? null : codexHomeDraft || null,
      defaultModel, models, followCodex: !isAntigravity && codexFollowDraft,
    } };
  }

  async function handleSaveAuthSettings() {
    if (isAntigravity && !authDefaultModelDraft) throw new Error(t("Sign in and refresh the Antigravity model list first."));
    await runSnapshotCommand("save_auth_settings_content", { input: authSettingsInput() });
    await refreshStatus();
  }

  /** Adds an unsaved-key provider entry without making it the default. */
  async function handleAddProvider(kind: keyof typeof API_PROVIDER_DEFAULTS) {
    const defaults = API_PROVIDER_DEFAULTS[kind];
    const taken = new Set(options.map((config) => config.id));
    let id = defaults.slug;
    for (let index = 2; taken.has(id); index += 1) id = `${defaults.slug}-${index}`;
    await runSnapshotCommand("save_auth_settings_content", { input: { select: false, config: {
      id, name: defaults.name, providerKind: kind, baseUrl: "", codexHome: null, models: [], defaultModel: "", followCodex: false,
    } } });
    setAuthConfigIdDraft(id);
  }

  async function handleDeleteProvider() {
    await runSnapshotCommand("delete_auth_settings_content", { input: { id: draftAuthConfig.id } });
  }

  async function handleStartLogin() {
    cancelAuthPolling();
    const generation = pollGeneration.current;
    const id = authConfigIdDraft;
    const command = isAntigravity ? "start_antigravity_login" : "start_codex_login";
    setCommandBusy(command, true);
    setError(null);
    try {
      if (isAntigravity) {
        const result = await invoke<AntigravityLoginStartView>(command);
        if (draftIdRef.current === id) setAntigravityAuthStatus(result.status);
        if (result.status.errorCode === "access_denied") return;
      } else {
        await runSnapshotCommand("save_auth_settings_content", { input: authSettingsInput() });
        const result = await invoke<CodexLoginStartView>("start_codex_login", { input: { codexHome: codexHomeDraft } });
        if (draftIdRef.current === id) setCodexAuthStatus(result.status);
      }
      if (draftIdRef.current !== id || pollGeneration.current !== generation) return;
      setAuthLoginPending(true);
      void pollAuthStatus(generation, id);
    } catch (error) {
      setError(error instanceof Error ? error.message : t("Could not start sign-in."));
    } finally {
      setCommandBusy(command, false);
    }
  }

  async function pollAuthStatus(generation: number, id: string) {
    try {
      const outcome = await pollAuthentication({
        isCurrent: () => pollGeneration.current === generation && draftIdRef.current === id,
        readStatus: async () => (await refreshStatus(id)) ?? { authenticated: false },
        onAuthenticated: async () => {
          if (isAntigravity) { await refreshCatalog(id); await refreshStatus(id); }
        },
      });
      if (outcome === "timeout") setError(t("Timed out waiting for sign-in. Finish signing in to the official client, then refresh."));
    } catch (error) {
      if (pollGeneration.current === generation) setError(error instanceof Error ? error.message : t("Could not refresh the sign-in status."));
    } finally {
      if (pollGeneration.current === generation) setAuthLoginPending(false);
    }
  }

  return {
    selectedAuthConfig, authConfigOptions: options, authConfigIdDraft, setAuthConfigIdDraft, authProviderDraft: draftAuthConfig.providerKind,
    antigravityAuthStatus, authLoginPending, cancelAuthPolling, refreshAuthStatusOnOpen,
    scopeDrafts, setScopeDrafts, codexAuthStatus, authNameDraft, authBaseUrlDraft, authDefaultModelDraft,
    authModelsDraft, codexHomeDraft, codexFollowDraft, setAuthNameDraft, setAuthBaseUrlDraft,
    setAuthDefaultModelDraft, setAuthModelsDraft, setCodexHomeDraft, setCodexFollowDraft,
    apiKeyDraft, setApiKeyDraft, apiKeyEnvDraft, setApiKeyEnvDraft,
    canDeleteProvider: (settings?.effective.model.authConfigs ?? []).some((config) => config.id === draftAuthConfig.id),
    handleRefreshAuth, handleSaveAuthSettings, handleStartLogin, handleScopeSave, handleAddProvider, handleDeleteProvider,
  };
}
