import { API_PROVIDER_DEFAULTS, usesApiKey } from "@/app/useSettingsDrafts";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type {
  AntigravityAuthStatusView,
  AuthConfigView,
  CodexAuthStatusView,
  ProviderKind
} from "@/types";
import {
  LogIn,
  RefreshCcw,
  Save,
  Trash2
} from "lucide-react";
import { useEffect } from "react";
import { Field, PROVIDER_LABELS, SettingRow, SettingsHeader } from "./settingsShared";
import { t } from "@/i18n";

/** Drafts and actions for key-based providers (OpenAI compatible, Anthropic, Gemini). */
export type ApiProviderProps = {
  apiKeyDraft: string;
  setApiKeyDraft: (value: string) => void;
  apiKeyEnvDraft: string;
  setApiKeyEnvDraft: (value: string) => void;
  canDelete: boolean;
  onAdd: (kind: keyof typeof API_PROVIDER_DEFAULTS) => void;
  onDelete: () => void;
};

export function AuthSettings({
  authConfigOptions, authConfigIdDraft, setAuthConfigIdDraft, authProviderDraft, antigravityAuthStatus, authLoginPending, cancelAuthPolling, refreshAuthStatusOnOpen,
  busyCommand,
  codexAuthStatus,
  authNameDraft,
  authBaseUrlDraft,
  authDefaultModelDraft,
  authModelsDraft,
  codexHomeDraft,
  codexFollowDraft,
  setAuthNameDraft,
  setAuthBaseUrlDraft,
  setAuthDefaultModelDraft,
  setAuthModelsDraft,
  setCodexHomeDraft,
  setCodexFollowDraft,
  onRefreshAuth,
  onSaveAuth,
  onStartLogin,
  apiProvider,
}: {
  busyCommand: string | null;
  authConfigOptions: AuthConfigView[];
  authConfigIdDraft: string;
  setAuthConfigIdDraft: (id: string) => void;
  authProviderDraft: ProviderKind;
  antigravityAuthStatus: AntigravityAuthStatusView | null;
  authLoginPending: boolean;
  cancelAuthPolling: () => void;
  refreshAuthStatusOnOpen: () => void;
  codexAuthStatus: CodexAuthStatusView | null;
  authNameDraft: string;
  authBaseUrlDraft: string;
  authDefaultModelDraft: string;
  authModelsDraft: string;
  codexHomeDraft: string;
  codexFollowDraft: boolean;
  setAuthNameDraft: (value: string) => void;
  setAuthBaseUrlDraft: (value: string) => void;
  setAuthDefaultModelDraft: (value: string) => void;
  setAuthModelsDraft: (value: string) => void;
  setCodexHomeDraft: (value: string) => void;
  setCodexFollowDraft: (value: boolean) => void;
  onRefreshAuth: () => void;
  onSaveAuth: () => void;
  onStartLogin: () => void;
  apiProvider: ApiProviderProps;
}) {
  useEffect(() => {
    refreshAuthStatusOnOpen();
    return cancelAuthPolling;
  }, [refreshAuthStatusOnOpen, cancelAuthPolling]);
  const isAntigravity = authProviderDraft === "antigravity_login";
  const isApiKey = usesApiKey(authProviderDraft);
  const apiDefaults = isApiKey ? API_PROVIDER_DEFAULTS[authProviderDraft as keyof typeof API_PROVIDER_DEFAULTS] : null;
  const busy = authLoginPending || busyCommand === "start_antigravity_login" || busyCommand === "save_auth_settings_content" || busyCommand === "get_antigravity_model_catalog" ||
    busyCommand === "delete_auth_settings_content" ||
    busyCommand === "start_codex_login" ||
    busyCommand === "save_codex_auth_settings_content";
  const statusReady = (isAntigravity ? antigravityAuthStatus : codexAuthStatus) !== null;
  const authenticated = (isAntigravity ? antigravityAuthStatus : codexAuthStatus)?.authenticated ?? false;
  const identity =
    (isAntigravity ? antigravityAuthStatus?.email : codexAuthStatus?.email ?? codexAuthStatus?.accountId) ??
    (statusReady ? authenticated ? t("Signed in") : t("Signed out") : t("Reading status…"));

  return (
    <section>
      <SettingsHeader
        title={t("Model providers")}
        detail={t("Use an API key (OpenAI compatible, Anthropic, Gemini) or reuse a local Codex / Antigravity sign-in. Saving makes it the default model.")}
        action={
          <div className="flex gap-2">
            <Select value="" onValueChange={(kind) => apiProvider.onAdd(kind as keyof typeof API_PROVIDER_DEFAULTS)}>
              <SelectTrigger className="w-44" aria-label={t("Add API provider")} disabled={busy}><SelectValue placeholder={t("Add API provider")} /></SelectTrigger>
              <SelectContent>
                {Object.entries(API_PROVIDER_DEFAULTS).map(([kind, defaults]) => <SelectItem key={kind} value={kind}>{defaults.name}</SelectItem>)}
              </SelectContent>
            </Select>
            {!isApiKey ? (
              <Button variant="secondary" disabled={busy} onClick={onRefreshAuth}>
                <RefreshCcw className="size-4" />
                {t("Refresh")}
              </Button>
            ) : null}
          </div>
        }
      />
      <div className="settings-group">
        <SettingRow title={t("Provider")} detail={isApiKey ? t("Authenticated with an API key.") : t("Pick a sign-in source, then sign in or refresh.")}>
          <Select value={authConfigIdDraft} onValueChange={setAuthConfigIdDraft}>
            <SelectTrigger className="w-56"><SelectValue /></SelectTrigger>
            <SelectContent>{authConfigOptions.map((config) => <SelectItem key={config.id} value={config.id}>{config.name} · {PROVIDER_LABELS[config.providerKind]}</SelectItem>)}</SelectContent>
          </Select>
        </SettingRow>
        {!isApiKey ? (
          <div role="note" className="border-y border-[var(--line)] bg-[var(--surface-muted)] px-4 py-3 text-xs leading-5 text-[var(--text-muted)]">
            {t("The {adapter} adapter is unofficial and not affiliated with {vendor}. It may violate their terms of service and can stop working at any time. Credentials stay on this machine; use at your own risk.", { adapter: isAntigravity ? "Antigravity" : "Codex", vendor: isAntigravity ? "Google" : "OpenAI" })}
          </div>
        ) : null}
        {isApiKey ? (
          <SettingRow title="API Key" detail={apiProvider.apiKeyEnvDraft ? t("Read from the environment variable {name} first", { name: apiProvider.apiKeyEnvDraft }) : apiProvider.apiKeyDraft ? t("Saved in user settings") : t("Falls back to the environment variable {name}", { name: apiDefaults?.keyEnv ?? "" })}>
            <span className="settings-state" data-tone={apiProvider.apiKeyDraft || apiProvider.apiKeyEnvDraft ? "success" : undefined}>
              {apiProvider.apiKeyDraft || apiProvider.apiKeyEnvDraft ? t("Configured") : t("Environment")}
            </span>
          </SettingRow>
        ) : null}
        {!isApiKey ? <SettingRow title={t("Status")} detail={identity}>
          <span className="settings-state" data-tone={authenticated ? "success" : authLoginPending ? "accent" : "danger"}>
            {authLoginPending ? t("Waiting for sign-in") : authenticated ? t("Signed in") : t("Sign-in required")}
          </span>
        </SettingRow> : null}
        {isApiKey ? null : isAntigravity ? (
          <>
            <SettingRow title={t("Antigravity client")} detail={antigravityAuthStatus?.appPath ?? (statusReady ? t("Official client not found") : t("Reading client info…"))}>
              <span className="settings-state" data-tone={antigravityAuthStatus?.appAvailable ? "success" : statusReady ? "danger" : undefined}>
                {statusReady ? antigravityAuthStatus?.appAvailable ? t("Available") : t("Missing") : t("Checking")}
              </span>
            </SettingRow>
            <SettingRow title={t("Sign-in source")} detail={t("macOS Keychain · Antigravity 2.x")}>
              <span className="text-xs text-[var(--text-muted)]">{t("Personal Google account")}</span>
            </SettingRow>
            {antigravityAuthStatus?.error ? <div role="status" className="border-y border-[var(--line)] px-4 py-3 text-sm text-[var(--text-muted)]">{antigravityAuthStatus.error}</div> : null}
          </>
        ) : (
          <>
            <SettingRow title="Codex CLI" detail={codexAuthStatus?.codexCliPath ?? t("codex command not found")}>
              <span className="settings-state" data-tone={codexAuthStatus?.codexCliAvailable ? "success" : "danger"}>
                {codexAuthStatus?.codexCliAvailable ? t("Available") : t("Missing")}
              </span>
            </SettingRow>
            <SettingRow title={t("Credentials file")} detail={codexAuthStatus?.authPath ?? t("Waiting for status")}>
              <span className="max-w-sm break-all text-right text-xs text-[var(--text-muted)]">
                {codexAuthStatus?.authMode ?? "codex"}
              </span>
            </SettingRow>
            {codexAuthStatus?.accountId ? (
              <SettingRow title={t("Account")} detail={codexAuthStatus.accountId}>
                <span className="text-sm text-[var(--text-muted)]">
                  {codexAuthStatus.planType ?? "ChatGPT"}
                </span>
              </SettingRow>
            ) : null}
            {codexAuthStatus?.error ? (
              <div className="border-y border-[var(--line)] bg-[var(--surface-muted)] px-4 py-3 text-sm leading-6 text-[var(--text-muted)]">
                {codexAuthStatus.error}
              </div>
            ) : null}
          </>
        )}
        <div className="settings-form border-t border-[var(--line)]">
          <div className="grid gap-4 md:grid-cols-2">
            <Field label={t("Name")}>
              <Input
                placeholder={apiDefaults?.name ?? (isAntigravity ? "Antigravity" : "Codex")}
                value={authNameDraft}
                onChange={(event) => setAuthNameDraft(event.currentTarget.value)}
              />
            </Field>
            <Field label={t("Default model")}>
              {isAntigravity ? (
                <Select value={authDefaultModelDraft || undefined} onValueChange={setAuthDefaultModelDraft} disabled={!authModelsDraft.trim()}>
                  <SelectTrigger><SelectValue placeholder={t("Sign in, then refresh the model list")} /></SelectTrigger>
                  <SelectContent>{authModelsDraft.split("\n").filter(Boolean).map((model) => <SelectItem key={model} value={model}>{model}</SelectItem>)}</SelectContent>
                </Select>
              ) : (
                <Input
                  placeholder={isApiKey ? t("Model ID") : "gpt-5.5"}
                  disabled={!isApiKey && codexFollowDraft}
                  value={authDefaultModelDraft}
                  onChange={(event) => setAuthDefaultModelDraft(event.currentTarget.value)}
                />
              )}
            </Field>
            {isApiKey ? (
              <>
                <Field label="Base URL">
                  <Input
                    placeholder={t("Leave empty for the official endpoint")}
                    value={authBaseUrlDraft}
                    onChange={(event) => setAuthBaseUrlDraft(event.currentTarget.value)}
                  />
                </Field>
                <Field label="API Key">
                  <Input
                    type="password"
                    autoComplete="off"
                    placeholder={t("Leave empty to read {name}", { name: apiDefaults?.keyEnv ?? "" })}
                    value={apiProvider.apiKeyDraft}
                    onChange={(event) => apiProvider.setApiKeyDraft(event.currentTarget.value)}
                  />
                </Field>
                <Field label={t("API key environment variable (optional)")}>
                  <Input
                    className="mono"
                    placeholder={apiDefaults?.keyEnv}
                    value={apiProvider.apiKeyEnvDraft}
                    onChange={(event) => apiProvider.setApiKeyEnvDraft(event.currentTarget.value)}
                  />
                </Field>
              </>
            ) : null}
            {!isAntigravity && !isApiKey ? (
              <>
                <Field label="Base URL">
                  <Input
                    placeholder="https://chatgpt.com/backend-api/codex"
                    value={authBaseUrlDraft}
                    onChange={(event) => setAuthBaseUrlDraft(event.currentTarget.value)}
                  />
                </Field>
                <Field label={t("Codex folder")}>
                  <Input
                    placeholder="~/.codex"
                    value={codexHomeDraft}
                    onChange={(event) => setCodexHomeDraft(event.currentTarget.value)}
                  />
                </Field>
              </>
            ) : null}
          </div>
          {!isAntigravity && !isApiKey ? <label className="mt-4 flex items-start gap-3 text-sm">
            <Checkbox
              className="mt-0.5"
              checked={codexFollowDraft}
              onCheckedChange={(checked) => setCodexFollowDraft(checked === true)}
            />
            <span className="min-w-0">
              <span className="block font-medium">{t("Follow Codex models")}</span>
              <span className="block text-[var(--text-muted)]">
                {t("Use the model list and default model from the Codex folder (models_cache.json and config.toml).")}
              </span>
            </span>
          </label> : null}
          <Field className="mt-4" label={t("Models")}>
            <Textarea
              className="min-h-28 resize-y font-mono text-sm"
              disabled={isAntigravity || (!isApiKey && codexFollowDraft)}
              placeholder={isApiKey ? t("One model ID per line") : undefined}
              value={authModelsDraft}
              onChange={(event) => setAuthModelsDraft(event.currentTarget.value)}
            />
          </Field>
          {isAntigravity && !authModelsDraft.trim() ? <p className="mt-3 text-sm text-[var(--text-muted)]">{t("Finish signing in to the official client, then click Refresh to load models.")}</p> : null}
          {isApiKey ? <p className="mb-0 mt-3 text-xs leading-5 text-[var(--text-muted)]">{t("API keys are saved in ~/.daily/settings.json, outside the workspace. You can also put them in the workspace's settings.local.json; provider configurations in project settings.json are ignored.")}</p> : null}
          <div className="mt-4 flex flex-wrap justify-end gap-2">
            {isApiKey && apiProvider.canDelete ? (
              <Button variant="ghost" className="text-[var(--danger)]" disabled={busy} onClick={apiProvider.onDelete}>
                <Trash2 className="size-4" />
                {t("Delete")}
              </Button>
            ) : null}
            <Button variant={isApiKey ? "default" : "secondary"} disabled={busy || ((isAntigravity || isApiKey) && !authDefaultModelDraft.trim())} onClick={onSaveAuth}>
              <Save className="size-4" />
              {t("Save")}
            </Button>
            {!isApiKey ? (
              <Button disabled={busy} onClick={onStartLogin}>
                <LogIn className="size-4" />
                {isAntigravity ? t("Sign in with Antigravity") : t("Sign in")}
              </Button>
            ) : null}
          </div>
        </div>
      </div>
    </section>
  );
}
