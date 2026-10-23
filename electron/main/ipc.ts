import { ipcMain, type IpcMainInvokeEvent } from "electron";
import {
  DesktopCommandError,
  isDesktopCommand,
  type DesktopCommand,
  type InvokeResult,
  type WorkspaceInfo,
} from "../shared/contracts";
import { errorMessage } from "./util";

/** What the IPC layer needs from the window manager, keyed by the calling window. */
export type DesktopIpcHost<Workspace> = {
  /** Resolves the workspace for a main-frame sender; throws for any other frame. */
  workspaceFor(event: IpcMainInvokeEvent): Workspace;
  workspaceInfo(workspace: Workspace): WorkspaceInfo;
  invokeCommand(workspace: Workspace, command: DesktopCommand, args: unknown): Promise<unknown>;
  chooseWorkspace(event: IpcMainInvokeEvent): Promise<void>;
  retryBackend(workspace: Workspace): Promise<void>;
  closeWorkspace(workspace: Workspace): Promise<void>;
  setDirty(workspace: Workspace, dirty: boolean): void;
};

/**
 * Registers the fixed `daily:*` IPC surface. Command results are wrapped in an
 * {@link InvokeResult} envelope because Electron drops custom Error fields.
 */
export function registerDesktopIpc<Workspace>(host: DesktopIpcHost<Workspace>): void {
  ipcMain.handle("daily:invoke", async (event, payload: { command?: unknown; args?: unknown }) => {
    if (!isDesktopCommand(payload?.command)) {
      return {
        ok: false,
        error: { code: "command_not_allowed", message: "Desktop command is not allowed." },
      } satisfies InvokeResult;
    }
    const command = payload.command;
    const workspace = host.workspaceFor(event);
    return host.invokeCommand(workspace, command, payload.args ?? {}).then(
      (result): InvokeResult => ({ ok: true, result }),
      (error: unknown): InvokeResult => ({
        ok: false,
        error: {
          code: error instanceof DesktopCommandError ? error.code : "desktop_error",
          message: errorMessage(error),
        },
      }),
    );
  });
  ipcMain.handle("daily:workspace-current", (event) => host.workspaceInfo(host.workspaceFor(event)));
  ipcMain.handle("daily:workspace-open", (event) => {
    host.workspaceFor(event);
    return host.chooseWorkspace(event);
  });
  ipcMain.handle("daily:backend-retry", (event) => host.retryBackend(host.workspaceFor(event)));
  ipcMain.handle("daily:workspace-close", (event) => host.closeWorkspace(host.workspaceFor(event)));
  ipcMain.handle("daily:workspace-dirty", (event, dirty: unknown) => {
    const workspace = host.workspaceFor(event);
    if (typeof dirty !== "boolean") throw new Error("Workspace dirty state must be a boolean.");
    host.setDirty(workspace, dirty);
  });
}
