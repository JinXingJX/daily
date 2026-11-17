import type { AgentAppConfig } from "@/types";
import { t } from "@/i18n";

export const APP_EVENT_NAME = "app://state";

export const PROVIDER_OPTIONS: Array<{ value: AgentAppConfig["providerKind"]; label: string }> = [
  { value: "codex_login", label: "Codex Login" },
  { value: "antigravity_login", label: "Antigravity Login" },
];

export const SLASH_COMMANDS = [
  {
    id: "/chart",
    get label() {
      return t("Insert chart instruction");
    },
    apply: () =>
      "Summarize the data and, if a chart helps, return a `daily-plugin` block with pluginId `builtin.chart`, renderer `chart`, and valid JSON payload.\n",
  },
  {
    id: "/weather",
    get label() {
      return t("Insert weather instruction");
    },
    apply: () =>
      "If a compact forecast helps, return a `daily-plugin` block with pluginId `builtin.weather`, renderer `weather`, and valid JSON payload.\n",
  },
  {
    id: "/new-chat",
    get label() {
      return t("New conversation");
    },
    apply: () => "",
  },
] as const;

/** Titlebar segment left of the title when the sidebar is collapsed: macOS traffic lights plus the toggle. */
export const TITLEBAR_COLLAPSED_LEAD = 118;
export const SIDEBAR_DEFAULT_WIDTH = 264;
export const SIDEBAR_MIN_WIDTH = 220;
export const SIDEBAR_MAX_WIDTH = 400;

export function clampSidebarWidth(width: number) {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width));
}
