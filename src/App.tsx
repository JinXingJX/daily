import { AlertTriangle, X } from "lucide-react";
import {
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  APP_EVENT_NAME,
  RAIL_WIDTH,
  SETTINGS_NAV_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  TITLEBAR_COLLAPSED_LEAD,
  clampSidebarWidth,
} from "@/app/constants";
import { useAgentDraft } from "@/app/useAgentDraft";
import { useSettingsDrafts } from "@/app/useSettingsDrafts";
import { useWorkspaceCommands } from "@/app/useWorkspaceCommands";
import { AppRail, Sidebar } from "@/features/workspace/Sidebar";
import { cn } from "@/lib/utils";
import {
  backendDesktop,
  invoke,
  onAppEvent,
  onBackendStatus,
  workspaceDesktop,
  type BackendStatus,
  type DesktopCommand,
  type WorkspaceInfo,
} from "@/platform/desktop";
import { coreRoute, routeIsCore, useAppStore, type AppRoute } from "./useAppStore";
import type { AppEventEnvelope, AppSnapshot, ComposerImage, ContextRootOption } from "./types";
import { ChatRoute } from "@/app/routes/ChatRoute";
import { SettingsRoute } from "@/app/routes/SettingsRoute";
import { WindowTitlebar } from "@/app/chrome/WindowTitlebar";
import { BackendBanner } from "@/app/chrome/BackendBanner";
import { StatusBar } from "@/app/chrome/StatusBar";
import { CommandPalette } from "@/app/chrome/CommandPalette";
import { BootstrapState } from "@/app/chrome/BootstrapState";
import { sameRoute } from "@/app/routes/sameRoute";
import { errorMessage } from "@/lib/errors";
import { readLocalPreference, writeLocalPreference } from "@/lib/preferences";
import type { SettingsSection } from "@/features/settings/SettingsPage";
import { displaySessionTitle } from "@/features/workspace/sessionTitle";
import { PluginViews } from "@/features/plugins/PluginViews";
import { forwardPluginEvents, pluginRuntime } from "@/features/plugins/runtime";
import { t } from "@/i18n";

function App() {
  const workspace = useAppStore((state) => state.workspace);
  const agentApps = useAppStore((state) => state.agentApps);
  const pluginManifests = useAppStore((state) => state.pluginManifests);
  const plugins = useAppStore((state) => state.plugins);
  const settings = useAppStore((state) => state.settings);
  const activeRoute = useAppStore((state) => state.activeRoute);
  const selectedAgentId = useAppStore((state) => state.selectedAgentId);
  const selectedScope = useAppStore((state) => state.selectedScope);
  const runtime = useAppStore((state) => state.runtime);
  const activeLatestTurn = useAppStore((state) => state.activeSession?.turns[state.activeSession.turns.length - 1] ?? null);
  const activeSessionAgentId = useAppStore((state) => state.activeSession?.agentAppId ?? null);
  const commandPaletteOpen = useAppStore((state) => state.commandPaletteOpen);
  const setSnapshot = useAppStore((state) => state.setSnapshot);
  const applyAppEvent = useAppStore((state) => state.applyAppEvent);
  const setActiveRoute = useAppStore((state) => state.setActiveRoute);
  const setSelectedAgentId = useAppStore((state) => state.setSelectedAgentId);
  const setSelectedScope = useAppStore((state) => state.setSelectedScope);
  const setCommandPaletteOpen = useAppStore((state) => state.setCommandPaletteOpen);

  const [error, setError] = useState<string | null>(null);
  const [workspaceInfo, setWorkspaceInfo] = useState<WorkspaceInfo | null>(null);
  const [backendStatus, setBackendStatus] = useState<BackendStatus>({
    state: "starting",
    message: "Starting local agent runtime…",
    canRetry: false,
  });
  const [draft, setDraft] = useState("");
  const [composerImages, setComposerImages] = useState<ComposerImage[]>([]);
  const [busyCommands, setBusyCommands] = useState<Set<string>>(() => new Set());
  const [retryMessageId, setRetryMessageId] = useState<number | null>(null);
  const pluginsDirty = useSyncExternalStore(pluginRuntime.subscribe, pluginRuntime.isDirty);
  const [inspectorOpen, setInspectorOpen] = useState(() => {
    if (typeof window === "undefined") return false;
    return readLocalPreference("daily:inspector-open") === "true";
  });
  const [sidebarCollapsed, setSidebarCollapsed] = useState(
    () => typeof window !== "undefined" && readLocalPreference("daily:sidebar-collapsed") === "true",
  );
  const [settingsEntry, setSettingsEntry] = useState<SettingsSection>("general");
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    if (typeof window === "undefined") {
      return SIDEBAR_DEFAULT_WIDTH;
    }
    const savedWidth = Number(readLocalPreference("daily:sidebar-width"));
    return Number.isFinite(savedWidth) && savedWidth > 0
      ? clampSidebarWidth(savedWidth)
      : SIDEBAR_DEFAULT_WIDTH;
  });
  const shortcutsRef = useRef({ newTask: () => {}, openSettings: () => {} });
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    if (typeof window === "undefined") {
      return "dark";
    }
    const savedTheme = readLocalPreference("daily:theme");
    if (savedTheme === "light" || savedTheme === "dark") {
      return savedTheme;
    }
    return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  });

  useEffect(() => {
    let disposed = false;
    const removeAppEvent = onAppEvent((message) => {
      if (message.event !== APP_EVENT_NAME) return;
      const envelope = message.payload as AppEventEnvelope;
      startTransition(() => applyAppEvent(envelope));
    });
    const removeBackendStatus = onBackendStatus((status) => {
      if (!disposed) setBackendStatus(status);
    });

    async function bootstrap() {
      try {
        const [initialSnapshot, currentWorkspace] = await Promise.all([
          invoke<AppSnapshot>("get_app_snapshot"),
          workspaceDesktop.current(),
        ]);
        if (disposed) {
          return;
        }
        startTransition(() => setSnapshot(initialSnapshot));
        setWorkspaceInfo(currentWorkspace);
        setBackendStatus({ state: "ready", message: "Local agent runtime is ready.", canRetry: false });
      } catch (nextError) {
        if (!disposed) {
          setError(nextError instanceof Error ? nextError.message : "Failed to load the workspace.");
        }
      }
    }

    void bootstrap();

    return () => {
      disposed = true;
      removeAppEvent();
      removeBackendStatus();
    };
  }, [applyAppEvent, setSnapshot]);

  useEffect(() => {
    if (backendStatus.state !== "ready") return;
    void Promise.all([invoke<AppSnapshot>("get_app_snapshot"), workspaceDesktop.current()])
      .then(([snapshot, currentWorkspace]) => {
        startTransition(() => setSnapshot(snapshot));
        setWorkspaceInfo(currentWorkspace);
        setError(null);
      })
      .catch(() => undefined);
  }, [backendStatus.state]);

  useEffect(() => {
    if (sidebarCollapsed) {
      return;
    }
    writeLocalPreference("daily:sidebar-width", String(sidebarWidth));
  }, [sidebarCollapsed, sidebarWidth]);

  useEffect(() => {
    writeLocalPreference("daily:sidebar-collapsed", String(sidebarCollapsed));
  }, [sidebarCollapsed]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    writeLocalPreference("daily:theme", theme);
  }, [theme]);

  useEffect(() => {
    writeLocalPreference("daily:inspector-open", String(inspectorOpen));
  }, [inspectorOpen]);

  useEffect(() => {
    void workspaceDesktop.setDirty(pluginsDirty).catch(() => undefined);
  }, [pluginsDirty]);

  // Plugins run in this window: start the ones that activate at startup, stop the disabled ones.
  useEffect(() => {
    forwardPluginEvents();
    if (plugins) pluginRuntime.sync(plugins.installed);
  }, [plugins]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // A plugin view that handled the key keeps it.
      if (event.defaultPrevented) return;
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      if (mod && key === "k") {
        event.preventDefault();
        setCommandPaletteOpen(!useAppStore.getState().commandPaletteOpen);
        return;
      }
      if (event.key === "Escape") {
        setCommandPaletteOpen(false);
        return;
      }
      if (!mod || event.altKey || event.shiftKey) return;
      const shortcut = shortcutsRef.current;
      if (key === "n") {
        event.preventDefault();
        shortcut.newTask();
      } else if (key === "i" && routeIsCore(useAppStore.getState().activeRoute, "chat")) {
        event.preventDefault();
        setInspectorOpen((value) => !value);
      } else if (key === ",") {
        event.preventDefault();
        shortcut.openSettings();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [setCommandPaletteOpen]);

  const setCommandBusy = useCallback((command: string, busy: boolean) => {
    setBusyCommands((current) => {
      const next = new Set(current);
      if (busy) next.add(command);
      else next.delete(command);
      return next;
    });
  }, []);

  const runSnapshotCommand = async <T,>(command: DesktopCommand, args?: T) => {
    setCommandBusy(command, true);
    setError(null);
    try {
      const nextSnapshot = await invoke<AppSnapshot>(command, args as Record<string, unknown> | undefined);
      startTransition(() => setSnapshot(nextSnapshot));
      return nextSnapshot;
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : `Command ${command} failed.`);
      throw nextError;
    } finally {
      setCommandBusy(command, false);
    }
  };

  const busyCommand = Array.from(busyCommands).pop() ?? null;
  const toggleInspector = useCallback(() => setInspectorOpen((value) => !value), []);
  const loadContextRoots = useCallback(() => invoke<ContextRootOption[]>("list_context_roots"), []);
  const runUiAction = (action: Promise<unknown>) => {
    void action.catch((nextError) => setError(errorMessage(nextError)));
  };

  const pluginMainViews = useMemo(
    () =>
      (plugins?.installed ?? []).flatMap((plugin) =>
        plugin.enabled ? plugin.contributes.views.map((view) => ({ plugin, view })) : [],
      ),
    [plugins?.installed],
  );


  const navigateToRoute = useCallback((route: AppRoute) => {
    if (sameRoute(activeRoute, route)) return true;
    if (route.kind === "core" && route.page === "settings") setSettingsEntry("general");
    setActiveRoute(route);
    return true;
  }, [activeRoute, setActiveRoute]);

  const {
    selectedAuthConfig,
    authConfigOptions,
    authConfigIdDraft,
    setAuthConfigIdDraft,
    authProviderDraft,
    antigravityAuthStatus,
    authLoginPending,
    cancelAuthPolling,
    refreshAuthStatusOnOpen,
    scopeDrafts,
    setScopeDrafts,
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
    handleRefreshAuth,
    handleSaveAuthSettings,
    handleStartLogin,
    handleScopeSave,
    apiKeyDraft,
    setApiKeyDraft,
    apiKeyEnvDraft,
    setApiKeyEnvDraft,
    canDeleteProvider,
    handleAddProvider,
    handleDeleteProvider,
  } = useSettingsDrafts({
    settings,
    selectedScope,
    runSnapshotCommand,
    setCommandBusy,
    setError,
  });

  const {
    agentDraft,
    setAgentDraft,
    handleCreateAgent,
    handleSaveAgent,
    handleDeleteAgent,
  } = useAgentDraft({
    agentApps,
    pluginManifests,
    settings,
    selectedAgentId,
    selectedAuthConfig,
    runSnapshotCommand,
    setActiveRoute,
    setSelectedAgentId,
  });

  const {
    currentParentId,
    commandItems,
    handleCreateSession,
    handleOpenSession,
    handleInstallPlugin,
    handleInstallLocalPlugin,
    handleUninstallPlugin,
    handleEnablePlugin,
    handleDisablePlugin,
  } = useWorkspaceCommands({
    workspace,
    pluginMainViews,
    theme,
    runSnapshotCommand,
    setActiveRoute: navigateToRoute,
    setTheme,
    onToggleInspector: toggleInspector,
  });

  shortcutsRef.current = {
    newTask: () => runUiAction(handleCreateSession(currentParentId)),
    openSettings: () => {
      navigateToRoute(coreRoute("settings"));
    },
  };

  function handleSidebarResizeStart(event: ReactPointerEvent<HTMLDivElement>) {
    if (sidebarCollapsed) {
      return;
    }
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = sidebarWidth;
    setSidebarResizing(true);

    const handlePointerMove = (moveEvent: PointerEvent) => {
      setSidebarWidth(clampSidebarWidth(startWidth + moveEvent.clientX - startX));
    };
    const handlePointerUp = () => {
      setSidebarResizing(false);
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
  }

  if (!workspace || !settings || !plugins) {
    return (
      <BootstrapState
        error={error}
        backendStatus={backendStatus}
        onRetry={() => runUiAction(backendDesktop.retry())}
      />
    );
  }

  const activePluginView =
    activeRoute.kind === "pluginView"
      ? pluginMainViews.find(
          ({ plugin, view }) => plugin.identifier === activeRoute.pluginId && view.id === activeRoute.viewId,
        ) ?? null
      : null;
  const activeCorePage = activeRoute.kind === "core" ? activeRoute.page : null;
  const chatOpen = activeCorePage === "chat";
  const panelVisible = chatOpen && !sidebarCollapsed;
  const activeSessionNode =
    workspace.activeSessionId != null
      ? workspace.nodes.find((node) => node.sessionId === workspace.activeSessionId) ?? null
      : null;
  const leadWidth = panelVisible
    ? RAIL_WIDTH + sidebarWidth
    : activeCorePage === "settings"
      ? RAIL_WIDTH + SETTINGS_NAV_WIDTH
      : TITLEBAR_COLLAPSED_LEAD;
  const shellStyle = {
    "--panel-width": `${panelVisible ? sidebarWidth : 0}px`,
  } as CSSProperties;
  const activeAgentName =
    agentApps.find((agent) => agent.id === (activeLatestTurn?.agentAppId ?? activeSessionAgentId))?.name ?? null;
  const modelSettings = settings.effective.model;
  const defaultAuthConfig =
    modelSettings.authConfigs.find((config) => config.id === modelSettings.authConfigId) ?? modelSettings.authConfigs[0];
  const knownAuthStatus =
    defaultAuthConfig?.providerKind === "antigravity_login" ? antigravityAuthStatus : defaultAuthConfig?.providerKind === "codex_login" ? codexAuthStatus : null;
  const showRunTurn =
    chatOpen && activeLatestTurn && activeLatestTurn.status !== "completed" ? activeLatestTurn : null;

  return (
    <div
      className="app-shell flex h-[100dvh] min-h-[100dvh] max-h-[100dvh] flex-col overflow-hidden text-[var(--text)]"
      style={shellStyle}
    >
      <WindowTitlebar
        leadWidth={leadWidth}
        panelVisible={panelVisible}
        panelToggleAvailable={chatOpen}
        contextLabel={chatOpen ? workspaceInfo?.name ?? null : null}
        activeLabel={
          activePluginView?.view.title ??
          (chatOpen ? displaySessionTitle(activeSessionNode?.name) : t("Settings"))
        }
        runTurn={showRunTurn}
        inspectorAvailable={chatOpen}
        inspectorOpen={inspectorOpen}
        onCommandOpen={() => setCommandPaletteOpen(true)}
        onToggleInspector={() => setInspectorOpen((value) => !value)}
        onTogglePanel={() => setSidebarCollapsed((value) => !value)}
      />
      {backendStatus.state !== "ready" ? (
        <BackendBanner
          status={backendStatus}
          onRetry={() => runUiAction(backendDesktop.retry())}
        />
      ) : null}
      {error ? (
        <div className="border-b border-[var(--danger-line)] bg-[var(--danger-soft)] px-4 py-2" role="alert">
          <div className="flex items-start gap-3 text-[13px] text-[var(--text-strong)]">
            <AlertTriangle className="mt-0.5 size-4 shrink-0 text-[var(--danger)]" />
            <span className="min-w-0 flex-1 break-words">{error}</span>
            <button
              type="button"
              className="rounded-[var(--radius-control)] p-1 text-[var(--text-muted)] hover:bg-[var(--geist-gray-alpha-100)] hover:text-[var(--text-strong)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)]"
              aria-label={t("Dismiss error")}
              onClick={() => setError(null)}
            >
              <X className="size-4" />
            </button>
          </div>
        </div>
      ) : null}

      <div
        className={cn(
          "flex min-h-0 flex-1 overflow-hidden",
          sidebarResizing && "cursor-col-resize select-none",
        )}
      >
        <AppRail
          activeRoute={activeRoute}
          pluginViews={pluginMainViews}
          runtimeIsRunning={runtime?.isRunning ?? false}
          onRouteChange={navigateToRoute}
          onManagePlugins={() => {
            if (navigateToRoute(coreRoute("settings"))) setSettingsEntry("plugins");
          }}
        />
        {panelVisible ? (
          <div className="hidden min-h-0 shrink-0 md:flex" style={{ width: "var(--panel-width)" }}>
            <Sidebar
              resizing={sidebarResizing}
              width={sidebarWidth}
              workspaceName={workspaceInfo?.name ?? t("Workspace")}
              workspacePath={workspaceInfo?.path ?? t("Local workspace")}
              workspaceNodes={workspace.nodes}
              activeSessionId={workspace.activeSessionId}
              activeAgentName={activeAgentName}
              activeLatestTurn={activeLatestTurn}
              runtime={runtime}
              onCommandOpen={() => setCommandPaletteOpen(true)}
              onRouteChange={navigateToRoute}
              onResizeStart={handleSidebarResizeStart}
              onResizeBy={(delta) => setSidebarWidth((value) => clampSidebarWidth(value + delta))}
              onCreateSession={(parentId) => runUiAction(handleCreateSession(parentId ?? workspace.rootId))}
              onOpenSession={(sessionId) => runUiAction(handleOpenSession(sessionId))}
              onOpenWorkspace={() => runUiAction(workspaceDesktop.open())}
            />
          </div>
        ) : null}

        <main className="app-main flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <div className="flex min-h-0 flex-1 overflow-hidden">
            <PluginViews
              views={pluginMainViews}
              active={activePluginView ? { pluginId: activePluginView.plugin.identifier, viewId: activePluginView.view.id } : null}
              theme={theme}
            />
            {activeCorePage === "chat" ? (
              <ChatRoute
                draft={draft}
                composerImages={composerImages}
                busyCommand={busyCommand}
                retryMessageId={retryMessageId}
                setDraft={setDraft}
                setComposerImages={setComposerImages}
                setRetryMessageId={setRetryMessageId}
                runSnapshotCommand={runSnapshotCommand}
                onError={setError}
                onCreateSession={() => handleCreateSession(currentParentId)}
                onOpenSettings={(target) => {
                  if (navigateToRoute(coreRoute("settings"))) setSettingsEntry(target);
                }}
                inspectorOpen={inspectorOpen}
                onCloseInspector={() => setInspectorOpen(false)}
              />
            ) : activePluginView ? null : (
              <SettingsRoute
                initialSection={settingsEntry}
                onBackToApp={() => navigateToRoute(coreRoute("chat"))}
                theme={theme}
                workspacePath={workspaceInfo?.path ?? t("Local workspace")}
                busyCommand={busyCommand}
                agentDraft={agentDraft}
                authConfigOptions={authConfigOptions}
                authConfigIdDraft={authConfigIdDraft}
                setAuthConfigIdDraft={setAuthConfigIdDraft}
                authProviderDraft={authProviderDraft}
                antigravityAuthStatus={antigravityAuthStatus}
                authLoginPending={authLoginPending}
                cancelAuthPolling={cancelAuthPolling}
                refreshAuthStatusOnOpen={refreshAuthStatusOnOpen}
                codexAuthStatus={codexAuthStatus}
                authNameDraft={authNameDraft}
                authBaseUrlDraft={authBaseUrlDraft}
                authDefaultModelDraft={authDefaultModelDraft}
                authModelsDraft={authModelsDraft}
                codexHomeDraft={codexHomeDraft}
                codexFollowDraft={codexFollowDraft}
                selectedScope={selectedScope}
                scopeDrafts={scopeDrafts}
                setAgentDraft={setAgentDraft}
                setAuthNameDraft={setAuthNameDraft}
                setAuthBaseUrlDraft={setAuthBaseUrlDraft}
                setAuthDefaultModelDraft={setAuthDefaultModelDraft}
                setAuthModelsDraft={setAuthModelsDraft}
                setCodexHomeDraft={setCodexHomeDraft}
                setCodexFollowDraft={setCodexFollowDraft}
                setSelectedScope={setSelectedScope}
                setScopeDrafts={setScopeDrafts}
                onThemeChange={setTheme}
                onOpenWorkspace={() => runUiAction(workspaceDesktop.open())}
                onCreateAgent={() => runUiAction(handleCreateAgent())}
                onDeleteAgent={() => runUiAction(handleDeleteAgent())}
                onSaveAgent={() => runUiAction(handleSaveAgent())}
                onRefreshAuth={() => runUiAction(handleRefreshAuth())}
                onSaveAuth={() => runUiAction(handleSaveAuthSettings())}
                onStartLogin={() => runUiAction(handleStartLogin())}
                apiProvider={{
                  apiKeyDraft,
                  setApiKeyDraft,
                  apiKeyEnvDraft,
                  setApiKeyEnvDraft,
                  canDelete: canDeleteProvider,
                  onAdd: (kind) => runUiAction(handleAddProvider(kind)),
                  onDelete: () => runUiAction(handleDeleteProvider()),
                }}
                onScopeSave={() => runUiAction(handleScopeSave())}
                onSelectAgent={setSelectedAgentId}
                onReloadPlugins={() =>
                  runUiAction(
                    runSnapshotCommand("reload_plugins").then(async (snapshot) => {
                      await pluginRuntime.reloadChanged();
                      pluginRuntime.sync(snapshot.plugins.installed);
                    }),
                  )
                }
                onInstallPlugin={(pluginId) => runUiAction(handleInstallPlugin(pluginId))}
                onInstallLocalPlugin={(manifestPath, devUrl) =>
                  runUiAction(handleInstallLocalPlugin(manifestPath, devUrl))
                }
                onUninstallPlugin={(plugin) => runUiAction(handleUninstallPlugin(plugin))}
                onEnablePlugin={(pluginId) => runUiAction(handleEnablePlugin(pluginId))}
                onDisablePlugin={(pluginId) => runUiAction(handleDisablePlugin(pluginId))}
                onLoadContextRoots={loadContextRoots}
              />
            )}
          </div>
        </main>
      </div>
      <StatusBar
        backendStatus={backendStatus}
        runtimeStatus={runtime?.isRunning ? runtime.status : null}
        runtimeIsRunning={runtime?.isRunning ?? false}
        authLabel={defaultAuthConfig ? `${defaultAuthConfig.name} · ${modelSettings.model}` : t("No model configured")}
        authTone={knownAuthStatus ? (knownAuthStatus.authenticated ? "success" : "warning") : "neutral"}
        enabledPluginCount={plugins.installed.filter((plugin) => plugin.enabled).length}
      />
      <CommandPalette
        open={commandPaletteOpen}
        items={commandItems}
        onClose={() => setCommandPaletteOpen(false)}
      />
    </div>
  );
}


export default App;
