import { useMemo, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { MessageSquare, MessageSquarePlus, Moon, PanelRight, Settings, Sun } from "lucide-react";
import { coreRoute, pluginViewRoute, type AppRoute } from "@/useAppStore";
import type { AppSnapshot, PluginManifest, WorkspaceView } from "@/types";
import { pluginViewIcon, type PluginMainView } from "@/features/workspace/Sidebar";
import type { DesktopCommand } from "@/platform/desktop";
import { displaySessionTitle } from "@/features/workspace/sessionTitle";
import { t } from "@/i18n";

type RunSnapshotCommand = <T,>(command: DesktopCommand, args?: T) => Promise<AppSnapshot>;

export type CommandGroup = "task" | "global";

export type CommandItem = {
  id: string;
  group: CommandGroup;
  /** The task that is open now; shown even before the user types a query. */
  current?: boolean;
  label: string;
  detail: string;
  icon: ReactNode;
  shortcut?: string;
  action: () => void;
};

export function useWorkspaceCommands({
  workspace,
  pluginMainViews,
  theme,
  runSnapshotCommand,
  setActiveRoute,
  setTheme,
  onToggleInspector,
}: {
  workspace: WorkspaceView | null;
  pluginMainViews: PluginMainView[];
  theme: "light" | "dark";
  runSnapshotCommand: RunSnapshotCommand;
  setActiveRoute: (route: AppRoute) => boolean;
  setTheme: Dispatch<SetStateAction<"light" | "dark">>;
  onToggleInspector: () => void;
}) {
  const currentParentId = useMemo(() => {
    if (!workspace) {
      return 1;
    }
    const activeSessionNode = workspace.nodes.find((node) => node.sessionId === workspace.activeSessionId);
    return activeSessionNode?.parentId ?? workspace.rootId;
  }, [workspace]);

  async function handleCreateSession(parentId = currentParentId): Promise<number | null> {
    if (!setActiveRoute(coreRoute("chat"))) return null;
    const nextSnapshot = await runSnapshotCommand("create_workspace_node", {
      input: {
        parentId,
        kind: "session",
        // Empty: the runtime applies its default title, which the first message replaces.
        name: "",
      },
    });
    const newestSessionNode = [...nextSnapshot.workspace.nodes]
      .reverse()
      .find((node) => node.kind === "session" && node.parentId === parentId);
    if (!newestSessionNode?.sessionId) return null;
    await runSnapshotCommand("open_session", { input: { sessionId: newestSessionNode.sessionId } });
    return newestSessionNode.sessionId;
  }

  async function handleOpenSession(sessionId: number) {
    if (!setActiveRoute(coreRoute("chat"))) return;
    await runSnapshotCommand("open_session", {
      input: { sessionId },
    });
  }

  async function handleInstallPlugin(pluginId: string) {
    await runSnapshotCommand("install_plugin", {
      input: { pluginId },
    });
  }

  async function handleInstallLocalPlugin(manifestPath: string, devUrl?: string) {
    const normalizedPath = manifestPath.trim();
    if (!normalizedPath) {
      throw new Error(t("Choose a local plugin folder or manifest file."));
    }
    await runSnapshotCommand("install_local_dev_plugin", {
      input: {
        manifestPath: normalizedPath,
        devUrl: devUrl?.trim() || null,
      },
    });
  }

  async function handleUninstallPlugin(plugin: PluginManifest) {
    if (!window.confirm(`Uninstall "${plugin.name}"?`)) {
      return;
    }
    await runSnapshotCommand("uninstall_plugin", {
      input: { pluginId: plugin.identifier },
    });
  }

  async function handleEnablePlugin(pluginId: string) {
    await runSnapshotCommand("enable_plugin", {
      input: { pluginId },
    });
  }

  async function handleDisablePlugin(pluginId: string) {
    await runSnapshotCommand("disable_plugin", {
      input: { pluginId },
    });
  }

  const commandItems = useMemo<CommandItem[]>(() => {
    const sessionCommandItems = (workspace?.nodes ?? [])
      .filter((node) => node.kind === "session" && node.sessionId)
      .slice(0, 50)
      .map<CommandItem>((node) => ({
        id: `session:${node.sessionId}`,
        group: "task",
        current: node.sessionId === workspace?.activeSessionId,
        label: displaySessionTitle(node.name),
        detail: node.sessionId === workspace?.activeSessionId ? t("Current task") : t("Open task"),
        icon: <MessageSquare className="size-4" />,
        action: () => void handleOpenSession(node.sessionId as number).catch(() => undefined),
      }));
    const pluginCommandItems = pluginMainViews.map<CommandItem>(({ plugin, view }) => {
      const Icon = pluginViewIcon(view);
      return {
        id: `open-plugin:${plugin.identifier}:${view.id}`,
        group: "global",
        label: t("Open {name}", { name: view.title }),
        detail: plugin.name,
        icon: <Icon className="size-4" />,
        action: () => setActiveRoute(pluginViewRoute(plugin.identifier, view.id)),
      };
    });
    return [
      {
        id: "new-chat",
        group: "global",
        label: t("New task"),
        detail: t("Create a task in this workspace"),
        icon: <MessageSquarePlus className="size-4" />,
        shortcut: "⌘N",
        action: () => void handleCreateSession(currentParentId).catch(() => undefined),
      },
      {
        id: "toggle-inspector",
        group: "global",
        label: t("Toggle inspector"),
        detail: t("Run steps, context and hooks"),
        icon: <PanelRight className="size-4" />,
        shortcut: "⌘I",
        action: () => {
          if (setActiveRoute(coreRoute("chat"))) onToggleInspector();
        },
      },
      ...pluginCommandItems,
      {
        id: "open-settings",
        group: "global",
        label: t("Settings"),
        detail: t("Agents, plugins, models and settings files"),
        icon: <Settings className="size-4" />,
        shortcut: "⌘,",
        action: () => setActiveRoute(coreRoute("settings")),
      },
      {
        id: "toggle-theme",
        group: "global",
        label: theme === "dark" ? t("Switch to light mode") : t("Switch to dark mode"),
        detail: t("Change the workspace theme"),
        icon: theme === "dark" ? <Sun className="size-4" /> : <Moon className="size-4" />,
        action: () => setTheme((value) => (value === "dark" ? "light" : "dark")),
      },
      ...sessionCommandItems,
    ];
  }, [currentParentId, onToggleInspector, pluginMainViews, setActiveRoute, setTheme, theme, workspace?.activeSessionId, workspace?.nodes]);

  return {
    currentParentId,
    commandItems,
    handleCreateSession,
    handleOpenSession,
    handleInstallPlugin,
    handleInstallLocalPlugin,
    handleUninstallPlugin,
    handleEnablePlugin,
    handleDisablePlugin,
  };
}
