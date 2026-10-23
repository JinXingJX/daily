import { Button } from "@/components/ui/button";
import type {
  AgentAppConfig,
  RuntimeView,
  SettingsView,
  WorkspaceView
} from "@/types";
import {
  FolderCog,
  Moon,
  Sun
} from "lucide-react";
import { PROVIDER_LABELS, SettingRow, SettingsHeader } from "./settingsShared";
import { currentLocale, localePreference, setLocalePreference, t } from "@/i18n";
import { localizeHostText } from "@daily/plugin-sdk/hostText";

export function GeneralSettings({
  workspace,
  agentApps,
  settings,
  runtime,
  theme,
  workspacePath,
  enabledPluginCount,
  onThemeChange,
  onOpenWorkspace,
}: {
  workspace: WorkspaceView;
  agentApps: AgentAppConfig[];
  settings: SettingsView;
  runtime: RuntimeView;
  theme: "light" | "dark";
  workspacePath: string;
  enabledPluginCount: number;
  onThemeChange: (theme: "light" | "dark") => void;
  onOpenWorkspace: () => void;
}) {
  const sessionCount = workspace.nodes.filter((node) => node.kind === "session").length;

  return (
    <section>
      <SettingsHeader title={t("General")} detail={t("Options that affect the whole workspace.")} />
      <div className="settings-group">
        <SettingRow title={t("Language")} detail={t("Interface language. “System” follows macOS.")}>
          <div className="segmented" role="group" aria-label={t("Language")}>
            {([
              ["system", t("System")],
              ["zh-CN", "中文"],
              ["en", "English"],
            ] as const).map(([value, label]) => (
              <button
                key={value}
                type="button"
                className="segmented-item"
                aria-pressed={localePreference() === value}
                onClick={() => setLocalePreference(value)}
              >
                {label}
              </button>
            ))}
          </div>
        </SettingRow>
        <SettingRow title={t("Appearance")} detail={t("Choose the workspace theme.")}>
          <div className="segmented" role="group" aria-label={t("Theme")}>
            <button type="button" className="segmented-item" aria-pressed={theme === "light"} onClick={() => onThemeChange("light")}>
              <Sun className="size-3.5" />
              {t("Light")}
            </button>
            <button type="button" className="segmented-item" aria-pressed={theme === "dark"} onClick={() => onThemeChange("dark")}>
              <Moon className="size-3.5" />
              {t("Dark")}
            </button>
          </div>
        </SettingRow>
        <SettingRow title={t("Runtime")} detail={localizeHostText(runtime.status, currentLocale())}>
          <span className="settings-state" data-tone={runtime.isRunning ? "accent" : "success"}>
            {runtime.isRunning ? t("Running") : t("Ready")}
          </span>
        </SettingRow>
        <SettingRow title={t("Workspace")} detail={workspacePath}>
          <div className="flex items-center gap-3">
            <span className="text-xs text-[var(--text-muted)]">
              {t("{tasks} tasks · {agents} agents · {plugins} plugins", { tasks: sessionCount, agents: agentApps.length, plugins: enabledPluginCount })}
            </span>
            <Button variant="secondary" size="sm" onClick={onOpenWorkspace}>
              <FolderCog className="size-4" />
              {t("Open workspace")}
            </Button>
          </div>
        </SettingRow>
        <SettingRow title={t("Default model")} detail={settings.effective.model.model}>
          <span className="text-[13px] text-[var(--text-muted)]">
            {PROVIDER_LABELS[settings.effective.model.providerKind]}
          </span>
        </SettingRow>
      </div>
    </section>
  );
}
