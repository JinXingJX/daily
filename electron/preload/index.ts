import { contextBridge, ipcRenderer } from "electron";
import type {
  AppEventMessage,
  BackendStatus,
  DesktopCommand,
  InvokeResult,
  WorkspaceInfo,
} from "../shared/contracts";

const desktopApi = {
  invoke<T>(command: DesktopCommand, args?: unknown): Promise<InvokeResult<T>> {
    return ipcRenderer.invoke("daily:invoke", { command, args });
  },
  onAppEvent(listener: (message: AppEventMessage) => void): () => void {
    const handler = (_event: Electron.IpcRendererEvent, message: AppEventMessage) => listener(message);
    ipcRenderer.on("daily:app-event", handler);
    return () => ipcRenderer.removeListener("daily:app-event", handler);
  },
  onBackendStatus(listener: (status: BackendStatus) => void): () => void {
    const handler = (_event: Electron.IpcRendererEvent, status: BackendStatus) => listener(status);
    ipcRenderer.on("daily:backend-status", handler);
    return () => ipcRenderer.removeListener("daily:backend-status", handler);
  },
  workspace: {
    current(): Promise<WorkspaceInfo> {
      return ipcRenderer.invoke("daily:workspace-current");
    },
    open(): Promise<void> {
      return ipcRenderer.invoke("daily:workspace-open");
    },
    close(): Promise<void> {
      return ipcRenderer.invoke("daily:workspace-close");
    },
    setDirty(dirty: boolean): Promise<void> {
      return ipcRenderer.invoke("daily:workspace-dirty", dirty);
    },
  },
  backend: {
    retry(): Promise<void> {
      return ipcRenderer.invoke("daily:backend-retry");
    },
  },
} as const;

contextBridge.exposeInMainWorld("dailyDesktop", desktopApi);
