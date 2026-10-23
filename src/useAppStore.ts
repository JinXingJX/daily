import { create } from "zustand";
import { applyTurnDeltaToSession } from "@/app/turnDelta";
import type {
  AgentAppConfig,
  AppEventEnvelope,
  AppSnapshot,
  ChatSessionView,
  PluginManifest,
  PluginSnapshot,
  RuntimeView,
  SettingsScope,
  SettingsView,
  TurnDelta,
  WorkspaceView,
} from "./types";

export type CorePage = "chat" | "settings";
export type AppRoute =
  | { kind: "core"; page: CorePage }
  | { kind: "pluginView"; pluginId: string; viewId: string };

type AppStore = {
  workspace: WorkspaceView | null;
  activeSession: ChatSessionView | null;
  agentApps: AgentAppConfig[];
  pluginManifests: PluginManifest[];
  plugins: PluginSnapshot | null;
  settings: SettingsView | null;
  runtime: RuntimeView | null;
  activeRoute: AppRoute;
  selectedAgentId: number | null;
  selectedScope: SettingsScope;
  commandPaletteOpen: boolean;
  setSnapshot: (snapshot: AppSnapshot) => void;
  applyAppEvent: (event: AppEventEnvelope) => void;
  setActiveRoute: (route: AppRoute) => void;
  setSelectedAgentId: (id: number | null) => void;
  setSelectedScope: (scope: SettingsScope) => void;
  setCommandPaletteOpen: (open: boolean) => void;
};

export const useAppStore = create<AppStore>((set, get) => ({
  workspace: null,
  activeSession: null,
  agentApps: [],
  pluginManifests: [],
  plugins: null,
  settings: null,
  runtime: null,
  activeRoute: coreRoute("chat"),
  selectedAgentId: null,
  selectedScope: "user",
  commandPaletteOpen: false,
  setSnapshot: (snapshot) => set(snapshotState(snapshot, get())),
  applyAppEvent: (event) => {
    if (event.kind === "snapshot") {
      set(snapshotState(event.snapshot, get()));
      return;
    }
    set((state) => deltaState(state, event.delta));
  },
  setActiveRoute: (activeRoute) => set({ activeRoute }),
  setSelectedAgentId: (selectedAgentId) => set({ selectedAgentId }),
  setSelectedScope: (selectedScope) => set({ selectedScope }),
  setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),
}));

export function coreRoute(page: CorePage): AppRoute {
  return { kind: "core", page };
}

export function pluginViewRoute(pluginId: string, viewId: string): AppRoute {
  return { kind: "pluginView", pluginId, viewId };
}

export function routeIsCore(route: AppRoute, page: CorePage) {
  return route.kind === "core" && route.page === page;
}

export function getCurrentSnapshot(): AppSnapshot | null {
  const state = useAppStore.getState();
  if (!state.workspace || !state.plugins || !state.settings || !state.runtime) {
    return null;
  }
  return {
    workspace: state.workspace,
    activeSession: state.activeSession,
    agentApps: state.agentApps,
    pluginManifests: state.pluginManifests,
    plugins: state.plugins,
    settings: state.settings,
    runtime: state.runtime,
  };
}

export function getSelectedAgentId() {
  return useAppStore.getState().selectedAgentId;
}

export function closePalette() {
  useAppStore.getState().setCommandPaletteOpen(false);
}

function snapshotState(snapshot: AppSnapshot, state: AppStore) {
  return {
    workspace: snapshot.workspace,
    activeSession: snapshot.activeSession,
    agentApps: snapshot.agentApps,
    pluginManifests: snapshot.pluginManifests,
    plugins: snapshot.plugins,
    settings: snapshot.settings,
    runtime: snapshot.runtime,
    selectedAgentId:
      state.selectedAgentId && snapshot.agentApps.some((agent) => agent.id === state.selectedAgentId)
        ? state.selectedAgentId
        : snapshot.activeSession?.agentAppId ?? snapshot.agentApps[0]?.id ?? null,
    selectedScope: snapshot.settings.scopes.some((scope) => scope.scope === state.selectedScope)
      ? state.selectedScope
      : "user",
  };
}

function deltaState(state: AppStore, delta: TurnDelta) {
  return {
    workspace:
      state.workspace && state.workspace.activeSessionId !== delta.activeSessionId
        ? {
            ...state.workspace,
            activeSessionId: delta.activeSessionId,
          }
        : state.workspace,
    activeSession: applyTurnDeltaToSession(state.activeSession, delta),
    runtime: delta.runtime,
  };
}
