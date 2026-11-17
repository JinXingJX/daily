import { Input } from "@/components/ui/input";
import type { AgentFormState } from "@/features/agents/agentHooks";
import { cn } from "@/lib/utils";
import type {
  AgentAppConfig,
  AntigravityAuthStatusView,
  AuthConfigView,
  CodexAuthStatusView,
  ContextRootOption,
  PluginManifest,
  PluginSnapshot,
  ProviderKind,
  RuntimeView,
  SettingsScope,
  SettingsView,
  WorkspaceView
} from "@/types";
import { Search } from "lucide-react";
import { useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { AgentSettings } from "./AgentSettings";
import { AuthSettings, type ApiProviderProps } from "./AuthSettings";
import { ConfigFileSettings } from "./ConfigFileSettings";
import { GeneralSettings } from "./GeneralSettings";
import { PluginSettings } from "./PluginSettings";
import { t } from "@/i18n";

export type SettingsSection = "general" | "auth" | "agents" | "plugins" | "files";

type SettingsPageProps = {
  workspace: WorkspaceView;
  agentApps: AgentAppConfig[];
  plugins: PluginSnapshot;
  settings: SettingsView;
  runtime: RuntimeView;
  theme: "light" | "dark";
  workspacePath: string;
  busyCommand: string | null;
  agentDraft: AgentFormState | null;
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
  selectedScope: SettingsScope;
  scopeDrafts: Record<SettingsScope, string>;
  setAgentDraft: Dispatch<SetStateAction<AgentFormState | null>>;
  setAuthNameDraft: (value: string) => void;
  setAuthBaseUrlDraft: (value: string) => void;
  setAuthDefaultModelDraft: (value: string) => void;
  setAuthModelsDraft: (value: string) => void;
  setCodexHomeDraft: (value: string) => void;
  setCodexFollowDraft: (value: boolean) => void;
  setSelectedScope: (scope: SettingsScope) => void;
  setScopeDrafts: Dispatch<SetStateAction<Record<SettingsScope, string>>>;
  onThemeChange: (theme: "light" | "dark") => void;
  onOpenWorkspace: () => void;
  onCreateAgent: () => void;
  onDeleteAgent: () => void;
  onSaveAgent: () => void;
  onRefreshAuth: () => void;
  onSaveAuth: () => void;
  onStartLogin: () => void;
  apiProvider: ApiProviderProps;
  onScopeSave: () => void;
  onSelectAgent: (agentId: number | null) => void;
  onReloadPlugins: () => void;
  onInstallPlugin: (pluginId: string) => void;
  onInstallLocalPlugin: (manifestPath: string, devUrl?: string) => void;
  onUninstallPlugin: (plugin: PluginManifest) => void;
  onEnablePlugin: (pluginId: string) => void;
  onDisablePlugin: (pluginId: string) => void;
  onBackToApp: () => void;
  /** Lists project directories an agent can be scoped to; omitted outside the desktop app. */
  onLoadContextRoots?: () => Promise<ContextRootOption[]>;
  initialSection?: SettingsSection;
};

type SectionGroup = "workspace" | "build" | "advanced";

function sectionGroupLabel(group: SectionGroup) {
  return group === "workspace" ? t("Workspace") : group === "build" ? t("Build") : t("Advanced");
}

const SETTINGS_SECTIONS: Array<{
  id: SettingsSection;
  label: string;
  detail: string;
  group: SectionGroup;
}> = [
  { id: "general", get label() { return t("General"); }, detail: "theme runtime language general 外观 运行时 语言", group: "workspace" },
  { id: "auth", get label() { return t("Model providers"); }, detail: "OpenAI Anthropic Gemini Codex Antigravity api key auth login 登录", group: "workspace" },
  { id: "agents", get label() { return t("Agents"); }, detail: "agents model hooks context 模型 钩子 上下文", group: "build" },
  { id: "plugins", get label() { return t("Plugins"); }, detail: "plugins extensions 扩展", group: "build" },
  { id: "files", get label() { return t("Settings files"); }, detail: "user project local config files 用户 项目 本地", group: "advanced" },
];

const SECTION_GROUPS: SectionGroup[] = ["workspace", "build", "advanced"];

export function SettingsPage(props: SettingsPageProps) {
  const [activeSection, setActiveSection] = useState<SettingsSection>(props.initialSection ?? "general");
  const [query, setQuery] = useState("");
  const enabledPlugins = useMemo(
    () => props.plugins.installed.filter((plugin) => plugin.enabled),
    [props.plugins.installed],
  );

  useEffect(() => {
    if (props.initialSection) setActiveSection(props.initialSection);
  }, [props.initialSection]);

  const normalizedQuery = query.trim().toLowerCase();
  const visibleSections = SETTINGS_SECTIONS.filter((section) =>
    `${section.label} ${section.detail} ${sectionGroupLabel(section.group)}`.toLowerCase().includes(normalizedQuery),
  );
  const sectionCounts: Partial<Record<SettingsSection, number>> = {
    agents: props.agentApps.length,
    plugins: props.plugins.installed.length,
  };
  const agentEditor = activeSection === "agents";

  return (
    <div className="settings-page grid min-h-0 flex-1 overflow-hidden md:grid-cols-[15rem_minmax(0,1fr)]">
      <aside className="flex min-h-0 flex-col gap-4 border-b border-[var(--line-soft)] px-3 pb-3 pt-5 md:border-b-0 md:pl-5 md:pr-2">
        <h2 className="display-title px-2.5">{t("Settings")}</h2>
        <label className="relative block">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-[var(--text-muted)]" />
          <Input
            type="search"
            value={query}
            className="h-[34px] border-[var(--line)] bg-[var(--surface-strong)] pl-8 text-[13px]"
            placeholder={t("Search settings")}
            aria-label={t("Search settings")}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto" aria-label={t("Settings sections")}>
          {SECTION_GROUPS.map((group) => {
            const sections = visibleSections.filter((section) => section.group === group);
            if (!sections.length) return null;
            return (
              <div key={group} className="flex flex-col gap-px">
                <div className="sidebar-section-label">{sectionGroupLabel(group)}</div>
                {sections.map((section) => {
                  const count = sectionCounts[section.id];
                  return (
                    <button
                      key={section.id}
                      className={cn("settings-nav-item", activeSection === section.id && "settings-nav-item-active")}
                      type="button"
                      aria-current={activeSection === section.id ? "page" : undefined}
                      onClick={() => setActiveSection(section.id)}
                    >
                      <span className="min-w-0 flex-1 truncate text-left">{section.label}</span>
                      {count != null ? (
                        <span className="mono text-[11px] font-normal text-[var(--text-muted)]">{count}</span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            );
          })}
          {!visibleSections.length ? (
            <div className="px-3 py-8 text-center text-xs leading-5 text-[var(--text-muted)]">{t("No matching settings")}</div>
          ) : null}
        </nav>
      </aside>

      <main className={cn("min-h-0", agentEditor ? "flex overflow-hidden" : "overflow-auto px-6 pb-12 pt-7 lg:px-8")}>
        {agentEditor ? (
          <AgentSettings
            agentApps={props.agentApps}
            installedPlugins={props.plugins.installed}
            settings={props.settings}
            agentDraft={props.agentDraft}
            busy={props.busyCommand === "update_agent_app"}
            setAgentDraft={props.setAgentDraft}
            onCreateAgent={props.onCreateAgent}
            onDeleteAgent={props.onDeleteAgent}
            onSaveAgent={props.onSaveAgent}
            onSelectAgent={props.onSelectAgent}
            onLoadContextRoots={props.onLoadContextRoots}
          />
        ) : (
          <div className="w-full max-w-[46rem]">
            {activeSection === "general" ? (
              <GeneralSettings
                workspace={props.workspace}
                agentApps={props.agentApps}
                settings={props.settings}
                runtime={props.runtime}
                theme={props.theme}
                workspacePath={props.workspacePath}
                enabledPluginCount={enabledPlugins.length}
                onThemeChange={props.onThemeChange}
                onOpenWorkspace={props.onOpenWorkspace}
              />
            ) : null}
            {activeSection === "auth" ? (
              <AuthSettings
                busyCommand={props.busyCommand}
                authConfigOptions={props.authConfigOptions}
                authConfigIdDraft={props.authConfigIdDraft}
                setAuthConfigIdDraft={props.setAuthConfigIdDraft}
                authProviderDraft={props.authProviderDraft}
                antigravityAuthStatus={props.antigravityAuthStatus}
                authLoginPending={props.authLoginPending}
                cancelAuthPolling={props.cancelAuthPolling}
                refreshAuthStatusOnOpen={props.refreshAuthStatusOnOpen}
                codexAuthStatus={props.codexAuthStatus}
                authNameDraft={props.authNameDraft}
                authBaseUrlDraft={props.authBaseUrlDraft}
                authDefaultModelDraft={props.authDefaultModelDraft}
                authModelsDraft={props.authModelsDraft}
                codexHomeDraft={props.codexHomeDraft}
                codexFollowDraft={props.codexFollowDraft}
                setAuthNameDraft={props.setAuthNameDraft}
                setAuthBaseUrlDraft={props.setAuthBaseUrlDraft}
                setAuthDefaultModelDraft={props.setAuthDefaultModelDraft}
                setAuthModelsDraft={props.setAuthModelsDraft}
                setCodexHomeDraft={props.setCodexHomeDraft}
                setCodexFollowDraft={props.setCodexFollowDraft}
                onRefreshAuth={props.onRefreshAuth}
                onSaveAuth={props.onSaveAuth}
                onStartLogin={props.onStartLogin}
                apiProvider={props.apiProvider}
              />
            ) : null}
            {activeSection === "plugins" ? (
              <PluginSettings
                plugins={props.plugins}
                busyCommand={props.busyCommand}
                onReloadPlugins={props.onReloadPlugins}
                onInstallPlugin={props.onInstallPlugin}
                onInstallLocalPlugin={props.onInstallLocalPlugin}
                onUninstallPlugin={props.onUninstallPlugin}
                onEnablePlugin={props.onEnablePlugin}
                onDisablePlugin={props.onDisablePlugin}
              />
            ) : null}
            {activeSection === "files" ? (
              <ConfigFileSettings
                settings={props.settings}
                selectedScope={props.selectedScope}
                scopeDrafts={props.scopeDrafts}
                setSelectedScope={props.setSelectedScope}
                setScopeDrafts={props.setScopeDrafts}
                onScopeSave={props.onScopeSave}
              />
            ) : null}
          </div>
        )}
      </main>
    </div>
  );
}
