import {
  ChartCandlestick,
  Clapperboard,
  ChevronsUpDown,
  CircleAlert,
  CirclePause,
  FolderOpen,
  LoaderCircle,
  MessageSquare,
  NotebookPen,
  Plug,
  Plus,
  Search,
  Settings,
  SlidersHorizontal,
} from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import type { StatusTone } from "@/app/chrome/runtimeHealth";
import { turnProgress, sessionRunState, type RunState } from "@/features/chat/turnProgress";
import { cn } from "@/lib/utils";
import { coreRoute, pluginViewRoute, routeIsCore, type AppRoute } from "@/useAppStore";
import type { AgentTurnView, PluginManifest, PluginViewContribution, RuntimeView, WorkspaceNode } from "@/types";
import { displaySessionTitle } from "@/features/workspace/sessionTitle";
import { t } from "@/i18n";

export type PluginMainView = {
  plugin: PluginManifest;
  view: PluginViewContribution;
};

type NavigationProps = {
  activeRoute: AppRoute;
  pluginViews: PluginMainView[];
  onRouteChange: (route: AppRoute) => void;
  onManagePlugins: () => void;
  onCreateSession: () => void;
  onCommandOpen: () => void;
};

function pluginViewActive(route: AppRoute, item: PluginMainView) {
  return route.kind === "pluginView" && route.pluginId === item.plugin.identifier && route.viewId === item.view.id;
}

/** The collapsed sidebar: the same destinations as icon buttons. */
export function AppRail(props: NavigationProps & { runtimeIsRunning: boolean }) {
  const chatActive = routeIsCore(props.activeRoute, "chat");
  const settingsActive = routeIsCore(props.activeRoute, "settings");

  return (
    <nav className="app-rail" aria-label={t("App")}>
      <RailButton label={`${t("New task")} (⌘N)`} onClick={props.onCreateSession}>
        <span className="new-task-mark">
          <Plus className="size-3.5" strokeWidth={2.25} />
        </span>
      </RailButton>
      <RailButton label={`${t("Search tasks and commands")} (⌘K)`} onClick={props.onCommandOpen}>
        <Search className="size-[17px]" />
      </RailButton>
      <div className="app-rail-divider" aria-hidden="true" />
      <RailButton
        label={props.runtimeIsRunning ? t("Chat (tasks running)") : t("Chat")}
        active={chatActive}
        onClick={() => props.onRouteChange(coreRoute("chat"))}
      >
        <MessageSquare className="size-[17px]" />
        {props.runtimeIsRunning ? <span className="app-rail-badge" aria-hidden="true" /> : null}
      </RailButton>
      {props.pluginViews.map((item) => {
        const Icon = pluginViewIcon(item.view);
        return (
          <RailButton
            key={`${item.plugin.identifier}:${item.view.id}`}
            label={item.view.title}
            active={pluginViewActive(props.activeRoute, item)}
            onClick={() => props.onRouteChange(pluginViewRoute(item.plugin.identifier, item.view.id))}
          >
            <Icon className="size-[17px]" />
          </RailButton>
        );
      })}
      <RailButton label={t("Manage plugins")} onClick={props.onManagePlugins}>
        <Plug className="size-4 text-[var(--text-muted)]" />
      </RailButton>
      <div className="flex-1" />
      <RailButton label={`${t("Settings")} (⌘,)`} active={settingsActive} onClick={() => props.onRouteChange(coreRoute("settings"))}>
        <SlidersHorizontal className="size-[17px]" />
      </RailButton>
    </nav>
  );
}

function RailButton({
  label,
  active,
  onClick,
  children,
}: {
  label: string;
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={cn("app-rail-button", active && "app-rail-button-active")}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      title={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

type SidebarProps = NavigationProps & {
  resizing: boolean;
  width: number;
  workspaceName: string;
  workspacePath: string;
  workspaceNodes: WorkspaceNode[];
  activeSessionId: number | null;
  activeAgentName: string | null;
  activeLatestTurn: AgentTurnView | null;
  runtime: RuntimeView | null;
  health: { tone: StatusTone; label: string };
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
  onResizeStart: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onResizeBy: (delta: number) => void;
  onOpenSession: (sessionId: number) => void;
  onOpenWorkspace: () => void;
};

export function Sidebar(props: SidebarProps) {
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const chatActive = routeIsCore(props.activeRoute, "chat");
  const nodesById = new Map(props.workspaceNodes.map((node) => [node.id, node]));
  const sessions = props.workspaceNodes.filter(
    (node) => node.kind === "session" && node.sessionId != null,
  );
  const busyCount = sessions.filter(
    (node) => sessionRunState(node.sessionId as number, props.runtime, props.activeSessionId, props.activeLatestTurn) === "running",
  ).length;

  useEffect(() => {
    if (!workspaceMenuOpen) return;
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) {
        setWorkspaceMenuOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setWorkspaceMenuOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [workspaceMenuOpen]);

  return (
    <aside className="app-sidebar relative flex min-h-0 flex-col overflow-hidden" aria-label={t("Tasks")}>
      <div className="flex shrink-0 flex-col gap-0.5 px-2 pb-2 pt-1">
        <button type="button" className="sidebar-row font-medium text-[var(--text-strong)]" onClick={props.onCreateSession}>
          <span className="new-task-mark">
            <Plus className="size-3.5" strokeWidth={2.25} />
          </span>
          <span className="min-w-0 flex-1 truncate">{t("New task")}</span>
          <kbd className="sidebar-row-kbd">⌘N</kbd>
        </button>
        <button type="button" className="sidebar-row" onClick={props.onCommandOpen}>
          <span className="sidebar-row-icon">
            <Search className="size-4" />
          </span>
          <span className="min-w-0 flex-1 truncate">{t("Search tasks and commands")}</span>
          <kbd className="sidebar-row-kbd">⌘K</kbd>
        </button>
        {props.pluginViews.length ? null : (
          <button type="button" className="sidebar-row" onClick={props.onManagePlugins}>
            <span className="sidebar-row-icon">
              <Plug className="size-4" />
            </span>
            <span className="min-w-0 flex-1 truncate">{t("Manage plugins")}</span>
          </button>
        )}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-2 pb-3 pt-1">
        {props.pluginViews.length ? (
          <section className="flex flex-col gap-0.5" aria-labelledby="sidebar-plugins">
            <h2 id="sidebar-plugins" className="sidebar-section-label">
              <span>{t("Plugins")}</span>
              <button
                type="button"
                className="sidebar-label-action"
                aria-label={t("Manage plugins")}
                title={t("Manage plugins")}
                onClick={props.onManagePlugins}
              >
                <Plus className="size-3.5" />
              </button>
            </h2>
            {props.pluginViews.map((item) => {
              const Icon = pluginViewIcon(item.view);
              const active = pluginViewActive(props.activeRoute, item);
              return (
                <button
                  key={`${item.plugin.identifier}:${item.view.id}`}
                  type="button"
                  className={cn("sidebar-row", active && "sidebar-row-active")}
                  aria-current={active ? "page" : undefined}
                  title={item.view.title}
                  onClick={() => props.onRouteChange(pluginViewRoute(item.plugin.identifier, item.view.id))}
                >
                  <span className="sidebar-row-icon">
                    <Icon className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1 truncate">{item.view.title}</span>
                </button>
              );
            })}
          </section>
        ) : null}

        <section className="flex flex-col gap-0.5" aria-labelledby="sidebar-tasks">
          <h2 id="sidebar-tasks" className="sidebar-section-label">
            <span>{t("Tasks")}</span>
            {sessions.length ? (
              <span className="font-normal">
                {busyCount ? t("{busy} running · {total}", { busy: busyCount, total: sessions.length }) : sessions.length}
              </span>
            ) : null}
          </h2>
          {sessions.map((node) => {
            const sessionId = node.sessionId as number;
            const current = sessionId === props.activeSessionId;
            const active = current && chatActive;
            const parent = node.parentId != null ? nodesById.get(node.parentId) : undefined;
            const nested = parent?.kind === "session";
            const state = sessionRunState(sessionId, props.runtime, props.activeSessionId, props.activeLatestTurn);
            const detail = sessionDetail(state, current, props.activeAgentName, props.activeLatestTurn);
            return (
              <button
                key={node.id}
                type="button"
                className={cn("sidebar-row sidebar-task", nested && "sidebar-task-nested", active && "sidebar-row-active")}
                aria-current={active ? "page" : undefined}
                title={displaySessionTitle(node.name)}
                onClick={() => props.onOpenSession(sessionId)}
              >
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">{displaySessionTitle(node.name)}</span>
                  {detail ? (
                    <span className={cn("truncate text-[12px] font-normal leading-4", sessionDetailTone(state))}>{detail}</span>
                  ) : null}
                </span>
                <SessionStateIcon state={state} />
              </button>
            );
          })}
          {!sessions.length ? (
            <button type="button" className="sidebar-empty" onClick={props.onCreateSession}>
              {t("No tasks yet. Create one to put a local agent to work.")}
            </button>
          ) : null}
        </section>
      </div>

      <div ref={menuRef} className="relative shrink-0 p-2">
        {workspaceMenuOpen ? (
          <div className="workspace-menu motion-reveal" role="menu" aria-label={t("Workspace")}>
            <div className="border-b border-[var(--line-soft)] px-3 py-2.5">
              <div className="truncate font-medium text-[var(--text-strong)]">{props.workspaceName}</div>
              <div className="mono mt-0.5 truncate text-[11px] text-[var(--text-muted)]" title={props.workspacePath}>
                {props.workspacePath}
              </div>
            </div>
            <div className="flex items-center justify-between gap-2 px-3 pb-1 pt-2.5">
              <span className="text-[13px] text-[var(--text-soft)]">{t("Appearance")}</span>
              <div className="segmented segmented-sm" role="group" aria-label={t("Theme")}>
                <button type="button" className="segmented-item" aria-pressed={props.theme === "light"} onClick={() => props.onThemeChange("light")}>
                  {t("Light")}
                </button>
                <button type="button" className="segmented-item" aria-pressed={props.theme === "dark"} onClick={() => props.onThemeChange("dark")}>
                  {t("Dark")}
                </button>
              </div>
            </div>
            <WorkspaceMenuButton
              onClick={() => {
                setWorkspaceMenuOpen(false);
                props.onOpenWorkspace();
              }}
            >
              <FolderOpen className="size-4 text-[var(--text-soft)]" />
              {t("Open another workspace…")}
            </WorkspaceMenuButton>
            <WorkspaceMenuButton
              onClick={() => {
                setWorkspaceMenuOpen(false);
                props.onRouteChange(coreRoute("settings"));
              }}
            >
              <Settings className="size-4 text-[var(--text-soft)]" />
              <span className="flex-1 text-left">{t("Settings")}</span>
              <kbd className="sidebar-row-kbd">⌘,</kbd>
            </WorkspaceMenuButton>
          </div>
        ) : null}
        <button
          type="button"
          className="sidebar-workspace-button"
          aria-expanded={workspaceMenuOpen}
          aria-haspopup="menu"
          onClick={() => setWorkspaceMenuOpen((value) => !value)}
        >
          <span className="workspace-mark">
            <span aria-hidden="true">{props.workspaceName.trim().charAt(0).toUpperCase() || "D"}</span>
            <span className="status-dot" data-tone={props.health.tone} role="img" aria-label={props.health.label} title={props.health.label} />
          </span>
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-medium">{props.workspaceName}</span>
            <span className="mono truncate text-[11px] leading-4 text-[var(--text-muted)]" title={props.workspacePath}>
              {abbreviateHome(props.workspacePath)}
            </span>
          </span>
          <ChevronsUpDown className="size-3.5 shrink-0 text-[var(--text-muted)]" />
        </button>
      </div>

      <div
        aria-label={t("Resize task panel")}
        aria-orientation="vertical"
        aria-valuemin={220}
        aria-valuemax={400}
        aria-valuenow={Math.round(props.width)}
        className={cn(
          "absolute inset-y-0 right-0 hidden w-2 cursor-col-resize items-center justify-center lg:flex",
          "after:h-10 after:w-px after:rounded-full after:bg-transparent after:transition-colors hover:after:bg-[var(--line-active)]",
          "focus-visible:outline-none focus-visible:shadow-[inset_-2px_0_var(--accent-strong)]",
          props.resizing && "after:bg-[var(--line-active)]",
        )}
        role="separator"
        tabIndex={0}
        onPointerDown={props.onResizeStart}
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          props.onResizeBy(event.key === "ArrowLeft" ? -16 : 16);
        }}
      />
    </aside>
  );
}

/** Idle tasks carry no icon; only a state that needs attention earns one. */
function SessionStateIcon({ state }: { state: RunState }) {
  switch (state) {
    case "running":
      return (
        <LoaderCircle
          className="size-3.5 shrink-0 animate-spin text-[var(--accent)] motion-reduce:animate-none"
          role="img"
          aria-label={t("Running")}
        />
      );
    case "waiting":
      return <CirclePause className="size-3.5 shrink-0 text-[var(--warning)]" role="img" aria-label={t("Waiting for confirmation")} />;
    case "failed":
      return <CircleAlert className="size-3.5 shrink-0 text-[var(--danger)]" role="img" aria-label={t("Failed")} />;
    default:
      return null;
  }
}

function sessionDetail(
  state: RunState,
  active: boolean,
  agentName: string | null,
  latestTurn: AgentTurnView | null,
) {
  if (state === "failed") return t("Failed · can continue from checkpoint");
  if (state === "waiting") return t("Waiting for confirmation in the plugin");
  if (state !== "running") return null;
  if (!active) return t("Running");
  const step = turnProgress(latestTurn, 0).currentStep;
  return [agentName, step ? t("Step {step}", { step }) : t("Running")].filter(Boolean).join(" · ");
}

function sessionDetailTone(state: RunState) {
  switch (state) {
    case "failed":
      return "text-[var(--danger)]";
    case "waiting":
      return "text-[var(--warning)]";
    default:
      return "text-[var(--text-muted)]";
  }
}

function abbreviateHome(path: string) {
  return path.replace(/^\/(?:Users|home)\/[^/]+/, "~");
}

function WorkspaceMenuButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" role="menuitem" className="workspace-menu-row" onClick={onClick}>
      {children}
    </button>
  );
}

export function pluginViewIcon(view: PluginViewContribution) {
  switch (view.icon) {
    case "chart-candlestick":
      return ChartCandlestick;
    case "notebook-pen":
      return NotebookPen;
    case "clapperboard":
      return Clapperboard;
    default:
      return Plug;
  }
}

export function pluginViewTone(view: PluginViewContribution) {
  switch (view.icon) {
    case "chart-candlestick":
      return "market";
    case "notebook-pen":
      return "writer";
    default:
      return "neutral";
  }
}
