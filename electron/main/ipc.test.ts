import type { IpcMainInvokeEvent } from "electron";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { registerDesktopIpc, type DesktopIpcHost } from "./ipc";

const { handlers } = vi.hoisted(() => ({
  handlers: new Map<string, (event: IpcMainInvokeEvent, payload: unknown) => Promise<unknown>>(),
}));

vi.mock("electron", () => ({
  ipcMain: {
    handle: (channel: string, handler: (event: IpcMainInvokeEvent, payload: unknown) => Promise<unknown>) => {
      handlers.set(channel, handler);
    },
  },
}));

describe("desktop IPC command routing", () => {
  beforeEach(() => handlers.clear());

  it.each([
    "get_antigravity_auth_status",
    "start_antigravity_login",
    "get_antigravity_model_catalog",
    "refresh_antigravity_models",
    "save_auth_settings_content",
  ])("forwards %s through the authenticated workspace bridge", async (command) => {
    const workspace = { path: "/tmp/workspace" };
    const result = { authenticated: true };
    const host: DesktopIpcHost<typeof workspace> = {
      workspaceFor: vi.fn(() => workspace),
      workspaceInfo: vi.fn(() => ({ path: workspace.path, name: "workspace" })),
      invokeCommand: vi.fn().mockResolvedValue(result),
      chooseWorkspace: vi.fn(), retryBackend: vi.fn(), closeWorkspace: vi.fn(), setDirty: vi.fn(),
    };
    registerDesktopIpc(host);
    const event = {} as IpcMainInvokeEvent;
    const args = { input: { authConfigId: "antigravity-default" } };
    const response = await handlers.get("daily:invoke")!(event, { command, args });
    expect(response).toEqual({ ok: true, result });
    expect(host.workspaceFor).toHaveBeenCalledWith(event);
    expect(host.invokeCommand).toHaveBeenCalledWith(workspace, command, args);
    await expect(handlers.get("daily:invoke")!(event, { command: "shell.exec" })).resolves.toMatchObject({
      ok: false, error: { code: "command_not_allowed" },
    });
    expect(host.invokeCommand).toHaveBeenCalledTimes(1);
  });
});
