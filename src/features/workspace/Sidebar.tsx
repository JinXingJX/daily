import {
  ChartCandlestick,
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
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
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

type AppRailProps = {
  activeRoute: AppRoute;
  pluginViews: PluginMainView[];
  runtimeIsRunning: boolean;
  onRouteChange: (route: AppRoute) => void;
  onManagePlugins: () => void;
};

export function AppRail(props: AppRailProps) {
  const mainPluginViews = props.pluginViews.filter(({ view }) => view.location === "main");
  const chatActive = routeIsCore(props.activeRoute, "chat");
  const settingsActive = routeIsCore(props.activeRoute, "settings");

  return (
    <nav className="app-rail" aria-label={t("App")}>
      <RailButton
        label={props.runtimeIsRunning ? t("Chat (tasks running)") : t("Chat")}
        active={chatActive}
        onClick={() => props.onRouteChange(coreRoute("chat"))}
      >
        <MessageSquare className="size-[18px]" />
        {props.runtimeIsRunning ? <span className="app-rail-badge" aria-hidden="true" /> : null}
      </RailButton>
      {mainPluginViews.map(({ plugin, view }) => {
        const tone = pluginViewTone(view);
        const Icon = pluginViewIcon(view);
        const active =
          props.activeRoute.kind === "pluginView" &&
          props.activeRoute.pluginId === plugin.identifier &&
          props.activeRoute.viewId === view.id;
        return (
          <RailButton
            key={`${plugin.identifier}:${view.id}`}
            label={view.title}
            active={active}
            style={{ "--rail-tone": `var(--tone-${tone})` } as CSSProperties}
            onClick={() => props.onRouteChange(pluginViewRoute(plugin.identifier, view.id))}
          >
            <Icon className="size-[18px]" />
          </RailButton>
        );
      })}
      <div className="app-rail-divider" aria-hidden="true" />
      <RailButton label={t("Manage plugins")} onClick={props.onManagePlugins}>
        <Plus className="size-4 text-[var(--text-muted)]" />
      </RailButton>
      <div className="flex-1" />
      <RailButton label={t("Settings")} active={settingsActive} onClick={() => props.onRouteChange(coreRoute("settings"))}>
        <SlidersHorizontal className="size-[17px]" />
      </RailButton>
    </nav>
  );
}

function RailButton({
  label,
  active,
  style,
  onClick,
  children,
}: {
  label: string;
  active?: boolean;
  style?: CSSProperties;
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
      style={style}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

type SidebarProps = {
  resizing: boolean;
  width: number;
  workspaceName: string;
  workspacePath: string;
  workspaceNodes: WorkspaceNode[];
  activeSessionId: number | null;
  activeAgentName: string | null;
  activeLatestTurn: AgentTurnView | null;
  runtime: RuntimeView | null;
  onCommandOpen: () => void;
  onRouteChange: (route: AppRoute) => void;
  onResizeStart: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onResizeBy: (delta: number) => void;
  onCreateSession: (parentId?: number) => void;
  onOpenSession: (sessionId: number) => void;
  onOpenWorkspace: () => void;
};

export function Sidebar(props: SidebarProps) {
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
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
      <div className="flex shrink-0 gap-1.5 px-3 pb-2 pt-3">
        <button type="button" className="sidebar-new-task" onClick={() => props.onCreateSession()}>
          <Plus className="size-[15px]" />
          <span className="flex-1 text-left">{t("New task")}</span>
          <kbd className="text-[11px] font-normal text-[var(--text-muted)]">⌘N</kbd>
        </button>
        <button
          type="button"
          className="sidebar-square-button"
          aria-label={`${t("Search tasks and commands")} (⌘K)`}
          title={`${t("Search tasks and commands")} (⌘K)`}
          onClick={props.onCommandOpen}
        >
          <Search className="size-[15px]" />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3 pt-1">
        <h2 className="sidebar-section-label m-0">
          <span>{t("Tasks")}</span>
          {sessions.length ? (
            <span className="mono">{busyCount ? t("{busy} running · {total}", { busy: busyCount, total: sessions.length }) : sessions.length}</span>
          ) : null}
        </h2>
        <div className="flex flex-col gap-px">
          {sessions.map((node) => {
            const sessionId = node.sessionId as number;
            const active = sessionId === props.activeSessionId;
            const parent = node.parentId != null ? nodesById.get(node.parentId) : undefined;
            const nested = parent?.kind === "session";
            const state = sessionRunState(sessionId, props.runtime, props.activeSessionId, props.activeLatestTurn);
            const detail = sessionDetail(state, active, props.activeAgentName, props.activeLatestTurn);
            return (
              <button
                key={node.id}
                type="button"
                className={cn("sidebar-row", nested && "sidebar-row-nested", active && "sidebar-row-active")}
                aria-current={active ? "page" : undefined}
                title={displaySessionTitle(node.name)}
                onClick={() => props.onOpenSession(sessionId)}
              >
                <SessionStateIcon state={state} active={active} />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className={cn("truncate", active && "font-medium")}>{displaySessionTitle(node.name)}</span>
                  {detail ? (
                    <span className={cn("truncate text-[11px]", sessionDetailTone(state))}>{detail}</span>
                  ) : null}
                </span>
              </button>
            );
          })}
        </div>
        {!sessions.length ? (
          <button
            type="button"
            className="mx-1 mt-1 w-[calc(100%-0.5rem)] rounded-[var(--radius-control)] border border-dashed border-[var(--line-strong)] px-3 py-4 text-left text-xs leading-5 text-[var(--text-muted)] hover:border-[var(--line-active)] hover:text-[var(--text-strong)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)]"
            onClick={() => props.onCreateSession()}
          >
            {t("No tasks yet. Create one to put a local agent to work.")}
          </button>
        ) : null}
      </div>

      <div ref={menuRef} className="relative shrink-0 p-2">
        {workspaceMenuOpen ? (
          <div className="workspace-menu motion-reveal" role="menu" aria-label={t("Workspace")}>
            <div className="border-b border-[var(--line)] px-3 py-2.5">
              <div className="truncate text-[13px] font-medium text-[var(--text-strong)]">{props.workspaceName}</div>
              <div className="mono mt-0.5 truncate text-[11px] text-[var(--text-muted)]" title={props.workspacePath}>
                {props.workspacePath}
              </div>
            </div>
            <WorkspaceMenuButton
              onClick={() => {
                setWorkspaceMenuOpen(false);
                props.onOpenWorkspace();
              }}
            >
              <FolderOpen className="size-4" />
              {t("Open another workspace…")}
            </WorkspaceMenuButton>
            <WorkspaceMenuButton
              onClick={() => {
                setWorkspaceMenuOpen(false);
                props.onRouteChange(coreRoute("settings"));
              }}
            >
              <Settings className="size-4" />
              {t("Workspace settings")}
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
          <span className="workspace-mark" aria-hidden="true">
            {props.workspaceName.trim().charAt(0).toLowerCase() || "d"}
          </span>
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-medium">{props.workspaceName}</span>
            <span className="mono truncate text-[11px] text-[var(--text-muted)]" title={props.workspacePath}>
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
          "focus-visible:outline-none focus-visible:shadow-[inset_-2px_0_var(--accent)]",
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

function SessionStateIcon({ state, active }: { state: RunState; active: boolean }) {
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
      return (
        <span className="grid size-3.5 shrink-0 place-items-center" aria-hidden="true">
          <span className={cn("size-1.5 rounded-full", active ? "bg-[var(--text-strong)]" : "bg-[var(--line-active)]")} />
        </span>
      );
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
