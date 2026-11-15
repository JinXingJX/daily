import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { setLocaleForTests } from "@/i18n";
import { authConfigOptions } from "@/app/useSettingsDrafts";
import { SettingsPage } from "./SettingsPage";

function props(): ComponentProps<typeof SettingsPage> {
  const options = authConfigOptions([]);
  const noOp = () => {};
  return {
    workspace: { rootId: 1, activeSessionId: null, nodes: [] }, agentApps: [],
    plugins: { installed: [], enabled: [], marketplace: [], runtimeErrors: [], eventViewParsers: [] },
    settings: { precedenceNote: "", warnings: [], scopes: [], effective: {
      agent: { systemPrompt: "", maxIterations: 3, dialogueWindow: 4, memoryWindow: 4 },
      model: { authConfigId: options[0].id, authConfigs: options, providerKind: "codex_login", baseUrl: options[0].baseUrl,
        model: options[0].defaultModel, codexHome: null, temperature: 0.2, maxOutputTokens: 1024, apiKeySet: false },
    } },
    runtime: { isRunning: false, runningSessionIds: [], status: "Ready" },
    theme: "light", workspacePath: "/tmp/workspace", busyCommand: null, agentDraft: null,
    authConfigOptions: options, authConfigIdDraft: options[1].id, authProviderDraft: "antigravity_login",
    antigravityAuthStatus: { authenticated: false, email: null, appAvailable: true, appPath: "/Applications/Antigravity.app",
      expiresAt: null, error: "请先登录", errorCode: "not_logged_in" },
    authLoginPending: false, cancelAuthPolling: noOp, refreshAuthStatusOnOpen: noOp, codexAuthStatus: null,
    authNameDraft: "Antigravity", authBaseUrlDraft: options[1].baseUrl, authDefaultModelDraft: "",
    authModelsDraft: "", codexHomeDraft: "", codexFollowDraft: false,
    selectedScope: "user", scopeDrafts: { user: "", project: "", local: "" }, initialSection: "auth",
    setAgentDraft: noOp, setAuthConfigIdDraft: noOp, setAuthNameDraft: noOp, setAuthBaseUrlDraft: noOp,
    setAuthDefaultModelDraft: noOp, setAuthModelsDraft: noOp, setCodexHomeDraft: noOp, setCodexFollowDraft: noOp,
    setSelectedScope: noOp, setScopeDrafts: noOp, onThemeChange: noOp, onOpenWorkspace: noOp, onCreateAgent: noOp,
    onDeleteAgent: noOp, onSaveAgent: noOp, onRefreshAuth: noOp, onSaveAuth: noOp, onStartLogin: noOp,
    onScopeSave: noOp, onSelectAgent: noOp, onReloadPlugins: noOp, onInstallPlugin: noOp, onInstallLocalPlugin: noOp,
    onUninstallPlugin: noOp, onEnablePlugin: noOp, onDisablePlugin: noOp, onBackToApp: noOp,
    apiProvider: { apiKeyDraft: "", setApiKeyDraft: noOp, apiKeyEnvDraft: "", setApiKeyEnvDraft: noOp, canDelete: true, onAdd: noOp, onDelete: noOp },
  };
}

describe("authentication settings", () => {
  it("shows the native login source and an unavailable model selection before login", () => {
    const html = renderToStaticMarkup(createElement(SettingsPage, props()));
    expect(html).toContain("在 Antigravity 中登录");
    expect(html).toContain("macOS 系统钥匙串");
    expect(html).toContain("请先登录");
    expect(html).toContain("登录后刷新模型列表");
    expect(html).not.toContain("Codex 目录");
    expect(html).toContain("非官方实现");
  });

  it.each(["open_ai_compatible", "codex_api_key"] as const)("shows API key fields without login controls for %s", (kind) => {
    const fixture = props();
    fixture.authConfigOptions = [
      ...fixture.authConfigOptions,
      { id: "api", name: "API", providerKind: kind, baseUrl: "", codexHome: null, models: ["gpt-5.5"], defaultModel: "gpt-5.5", followCodex: false },
    ];
    fixture.authConfigIdDraft = "api";
    fixture.authProviderDraft = kind;
    const html = renderToStaticMarkup(createElement(SettingsPage, fixture));
    expect(html).toContain("OPENAI_API_KEY");
    expect(html).toContain("删除");
    expect(html).not.toContain("非官方实现");
    expect(html).not.toContain("Codex CLI");
    expect(html).not.toContain(">登录<");
    expect(html).toContain('type="password"');
    if (kind === "codex_api_key") {
      expect(html).toContain("Codex API (Responses)");
      expect(html).toContain("https://api.openai.com/v1");
    }
  });

  it("preserves the Codex settings panel when Codex is selected", () => {
    const fixture = props();
    fixture.authConfigIdDraft = fixture.authConfigOptions[0].id;
    fixture.authProviderDraft = "codex_login";
    const html = renderToStaticMarkup(createElement(SettingsPage, fixture));
    expect(html).toContain("Codex CLI");
    expect(html).toContain("Codex 目录");
    expect(html).not.toContain("在 Antigravity 中登录");
  });
});

describe("settings in English", () => {
  afterEach(() => setLocaleForTests("zh-CN"));

  it("renders the provider panel without Chinese text", () => {
    setLocaleForTests("en");
    const html = renderToStaticMarkup(createElement(SettingsPage, props()));
    expect(html).toContain("Sign in with Antigravity");
    expect(html).toContain("Model providers");
    expect(html).toContain("is unofficial and not affiliated with Google");
    // The fixture's simulated Antigravity error ("请先登录") is runtime data, not UI text.
    expect(html.replace("请先登录", "")).not.toMatch(/[\u4e00-\u9fff]/);
  });
});
