import {
  DesktopCommandError,
  type AppEventMessage,
  type BackendStatus,
  type DesktopCommand,
  type InvokeResult,
  type WorkspaceInfo,
} from "../../electron/shared/contracts";

export type DailyDesktopApi = {
  invoke<T>(command: DesktopCommand, args?: unknown): Promise<InvokeResult<T>>;
  onAppEvent(listener: (message: AppEventMessage) => void): () => void;
  onBackendStatus(listener: (status: BackendStatus) => void): () => void;
  workspace: {
    current(): Promise<WorkspaceInfo>;
    open(): Promise<void>;
    close(): Promise<void>;
    setDirty(dirty: boolean): Promise<void>;
  };
  backend: {
    retry(): Promise<void>;
  };
};

declare global {
  interface Window {
    dailyDesktop: DailyDesktopApi;
  }
}

function desktop(): DailyDesktopApi {
  if (!window.dailyDesktop) {
    throw new Error("Daily must be opened in the desktop application.");
  }
  return window.dailyDesktop;
}

/** Runs a sidecar command; failures reject with a {@link DesktopCommandError} carrying its code. */
export async function invoke<T>(command: DesktopCommand, args?: unknown): Promise<T> {
  const response = await desktop().invoke<T>(command, args);
  if (!response.ok) throw new DesktopCommandError(response.error);
  return response.result;
}

export function onAppEvent(listener: (message: AppEventMessage) => void): () => void {
  return desktop().onAppEvent(listener);
}

export function onBackendStatus(listener: (status: BackendStatus) => void): () => void {
  return desktop().onBackendStatus(listener);
}

export const workspaceDesktop = {
  current: () => desktop().workspace.current(),
  open: () => desktop().workspace.open(),
  close: () => desktop().workspace.close(),
  setDirty: (dirty: boolean) => desktop().workspace.setDirty(dirty),
};

export const backendDesktop = {
  retry: () => desktop().backend.retry(),
};

export { DesktopCommandError };
export type { AppEventMessage, BackendStatus, DesktopCommand, WorkspaceInfo };
