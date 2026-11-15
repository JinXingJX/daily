import { readFile, stat, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import {
  app,
  BrowserWindow,
  dialog,
  shell,
  session,
  type BrowserWindowConstructorOptions,
  type IpcMainInvokeEvent,
} from "electron";
import {
  DesktopCommandError,
  type AppEventMessage,
  type BackendStatus,
  type DesktopCommand,
} from "../shared/contracts";
import { registerDesktopIpc } from "./ipc";
import { installApplicationMenu } from "./menu";
import { isSafeDevRendererUrl, isSafeExternalUrl, lockDownSessionPermissions, secureWebPreferences } from "./security";
import { SidecarClient, sidecarExecutableName } from "./sidecar";
import { canonicalDirectory, errorMessage, isDirectory } from "./util";
import {
  emptyWindowState,
  loadWindowState,
  visibleSavedBounds,
  withRecentWorkspace,
  writeWindowState,
  type SavedWindow,
  type WindowState,
} from "./windowState";
import { t } from "./i18n";
import { PluginModules, pluginContentType } from "./pluginModules";

const RESTART_RESET_MS = 60_000;
const SETTINGS_COMMANDS = new Set<DesktopCommand>([
  "save_settings_file_content",
  "save_codex_auth_settings_content",
  "save_auth_settings_content",
  "delete_auth_settings_content",
  "refresh_antigravity_models",
]);

/** One open workspace window and the sidecar process serving it. */
type WorkspaceRecord = {
  workspacePath: string;
  window: BrowserWindow;
  client: SidecarClient | null;
  status: BackendStatus;
  restartUsed: boolean;
  restartResetTimer: NodeJS.Timeout | null;
  forceClosing: boolean;
  closeInProgress: boolean;
  dirty: boolean;
};

export class WorkspaceWindowManager {
  private readonly records = new Map<string, WorkspaceRecord>();
  private state: WindowState = emptyWindowState();
  private saveTimer: NodeJS.Timeout | null = null;
  private stateSaveQueue: Promise<void> = Promise.resolve();
  private settingsQueue: Promise<unknown> = Promise.resolve();
  private quitting = false;
  private readonly pluginModules = new PluginModules();

  async initialize(): Promise<void> {
    this.state = await loadWindowState();
    lockDownSessionPermissions();
    this.servePluginModules();
    registerDesktopIpc<WorkspaceRecord>({
      workspaceFor: (event) => this.recordForEvent(event),
      workspaceInfo: (record) => ({ path: record.workspacePath, name: basename(record.workspacePath) }),
      invokeCommand: (record, command, args) => this.invokeCommand(record, command, args),
      chooseWorkspace: (event) => this.chooseWorkspace(BrowserWindow.fromWebContents(event.sender) ?? undefined),
      retryBackend: (record) => this.retrySidecar(record),
      closeWorkspace: (record) => this.confirmAndClose(record),
      setDirty: (record, dirty) => {
        record.dirty = dirty;
      },
      pluginModuleUrl: async (record, pluginId) => {
        const module = await this.invokeCommand(record, "plugin_module", { input: { pluginId } }) as {
          root: string;
          entry: string;
          revision: string;
        };
        return this.pluginModules.moduleUrl(module.root, module.entry, module.revision);
      },
    });
    this.installMenu();

    const developmentWorkspace = !app.isPackaged
      ? process.env.DAILY_WORKSPACE?.trim()
      : undefined;
    if (developmentWorkspace) {
      await this.openWorkspace(developmentWorkspace);
      return;
    }

    const saved = [...this.state.windows].sort((left, right) =>
      left.workspacePath === this.state.lastActive ? 1 : right.workspacePath === this.state.lastActive ? -1 : 0,
    );
    for (const entry of saved) {
      if (await isDirectory(entry.workspacePath)) {
        try {
          await this.openWorkspace(entry.workspacePath, entry);
        } catch (error) {
          console.error(`Failed to restore workspace ${entry.workspacePath}`, error);
        }
      }
    }
    if (this.records.size === 0) await this.chooseWorkspace();
  }

  async chooseWorkspace(owner?: BrowserWindow): Promise<void> {
    const options: Electron.OpenDialogOptions = {
      title: t("Open Daily Workspace"),
      buttonLabel: t("Open Workspace"),
      properties: ["openDirectory", "createDirectory"],
    };
    const parent = owner ?? BrowserWindow.getFocusedWindow();
    const result = parent
      ? await dialog.showOpenDialog(parent, options)
      : await dialog.showOpenDialog(options);
    if (result.canceled || result.filePaths.length === 0) {
      if (this.records.size === 0) app.quit();
      return;
    }
    await this.openWorkspace(result.filePaths[0]);
  }

  async openWorkspace(path: string, saved?: SavedWindow): Promise<void> {
    const workspacePath = await canonicalDirectory(path);
    const existing = this.records.get(workspacePath);
    if (existing) {
      if (existing.window.isMinimized()) existing.window.restore();
      existing.window.show();
      existing.window.focus();
      return;
    }

    const restoredBounds = saved ? visibleSavedBounds(saved.bounds) : null;
    const options: BrowserWindowConstructorOptions = {
      width: restoredBounds?.width ?? 1440,
      height: restoredBounds?.height ?? 960,
      x: restoredBounds?.x,
      y: restoredBounds?.y,
      minWidth: 1180,
      minHeight: 760,
      show: false,
      title: `${basename(workspacePath)} · Daily`,
      backgroundColor: "#121313",
      titleBarStyle: "hiddenInset",
      trafficLightPosition: { x: 16, y: 15 },
      webPreferences: secureWebPreferences(join(__dirname, "../preload/index.js")),
    };
    const window = new BrowserWindow(options);
    if (saved?.maximized) window.maximize();

    const record: WorkspaceRecord = {
      workspacePath,
      window,
      client: null,
      status: status("starting", "Starting local agent runtime…"),
      restartUsed: false,
      restartResetTimer: null,
      forceClosing: false,
      closeInProgress: false,
      dirty: false,
    };
    this.records.set(workspacePath, record);
    this.touchRecent(workspacePath);
    this.attachWindowHandlers(record);

    try {
      await this.startSidecar(record, false);
    } catch (error) {
      this.setStatus(record, "failed", errorMessage(error), true);
    }
    window.webContents.once("did-finish-load", () => {
      window.webContents.send("daily:backend-status", record.status);
      if (!app.isPackaged && process.env.DAILY_CAPTURE_PATH) {
        setTimeout(() => {
          void window.webContents
            .capturePage()
            .then((image) => writeFile(process.env.DAILY_CAPTURE_PATH as string, image.toPNG()))
            .catch((error) => console.error("Failed to capture Daily window", error));
        }, Number(process.env.DAILY_CAPTURE_DELAY_MS) || 2_000);
      }
    });
    window.once("ready-to-show", () => window.show());
    await this.loadRenderer(window);
    this.scheduleStateSave();
    this.installMenu();
  }

  async requestQuit(): Promise<void> {
    if (this.quitting) return;
    const running = await this.runningRecords();
    const dirty = [...this.records.values()].filter((record) => record.dirty);
    if (running.length > 0 || dirty.length > 0) {
      const message = dirty.length > 0
        ? t("Workspaces with unsaved plugin changes: {count}.", { count: dirty.length })
        : t("Workspaces with an active task: {count}.", { count: running.length });
      const detail = running.length > 0 && dirty.length > 0
        ? t("Workspaces that also have active tasks: {count}. Quitting discards the drafts and stops the local runtimes.", { count: running.length })
        : dirty.length > 0
          ? t("Quitting discards those drafts.")
          : t("Quitting stops the active tasks and closes their local runtimes.");
      const result = await dialog.showMessageBox({
        type: "warning",
        buttons: [t("Cancel"), t("Quit Daily")],
        defaultId: 0,
        cancelId: 0,
        title: dirty.length > 0 ? t("Unsaved plugin changes") : t("Tasks are still running"),
        message,
        detail,
      });
      if (result.response === 0) return;
    }
    this.quitting = true;
    await this.persistState();
    await Promise.all([...this.records.values()].map((record) => record.client?.shutdown()));
    for (const record of this.records.values()) {
      record.forceClosing = true;
      record.window.destroy();
    }
    app.quit();
  }

  private async startSidecar(record: WorkspaceRecord, restarting: boolean): Promise<void> {
    this.setStatus(
      record,
      restarting ? "restarting" : "starting",
      restarting ? "Restarting local agent runtime…" : "Starting local agent runtime…",
      false,
    );
    const client = new SidecarClient(sidecarExecutable(), record.workspacePath);
    record.client = client;
    client.on("event", (message: AppEventMessage) => {
      if (!record.window.isDestroyed()) record.window.webContents.send("daily:app-event", message);
    });
    client.on("status", (nextStatus: BackendStatus) => {
      if (record.client === client) this.setStatus(record, nextStatus.state, nextStatus.message, nextStatus.canRetry);
    });
    client.on("exit", ({ expected }: { expected: boolean }) => {
      if (!expected && record.client === client && !record.forceClosing) void this.handleUnexpectedExit(record);
    });
    await client.start();
    this.setStatus(record, "ready", "Local agent runtime is ready.", false);
    if (record.restartResetTimer) clearTimeout(record.restartResetTimer);
    record.restartResetTimer = setTimeout(() => {
      record.restartUsed = false;
    }, RESTART_RESET_MS);
  }

  private async handleUnexpectedExit(record: WorkspaceRecord): Promise<void> {
    if (record.restartUsed) {
      this.setStatus(record, "failed", "The local agent runtime stopped again.", true);
      return;
    }
    record.restartUsed = true;
    try {
      await this.startSidecar(record, true);
    } catch (error) {
      this.setStatus(record, "failed", errorMessage(error), true);
    }
  }

  private async retrySidecar(record: WorkspaceRecord): Promise<void> {
    const previous = record.client;
    record.client = null;
    await previous?.shutdown();
    record.restartUsed = true;
    try {
      await this.startSidecar(record, true);
    } catch (error) {
      this.setStatus(record, "failed", errorMessage(error), true);
      throw error;
    }
  }

  private attachWindowHandlers(record: WorkspaceRecord): void {
    const { window } = record;
    window.on("focus", () => {
      this.state.lastActive = record.workspacePath;
      this.touchRecent(record.workspacePath);
      this.scheduleStateSave();
    });
    window.on("resize", () => this.scheduleStateSave());
    window.on("move", () => this.scheduleStateSave());
    window.on("maximize", () => this.scheduleStateSave());
    window.on("unmaximize", () => this.scheduleStateSave());
    window.on("close", (event) => {
      if (record.forceClosing || this.quitting) return;
      event.preventDefault();
      void this.confirmAndClose(record).catch((error) => {
        dialog.showErrorBox(t("Could not close workspace"), errorMessage(error));
      });
    });
    window.on("closed", () => {
      record.restartResetTimer && clearTimeout(record.restartResetTimer);
      this.records.delete(record.workspacePath);
      this.scheduleStateSave();
      this.installMenu();
    });
    window.webContents.setWindowOpenHandler(({ url }) => {
      if (isSafeExternalUrl(url)) {
        void shell.openExternal(url).catch((error) => {
          dialog.showErrorBox(t("Could not open link"), errorMessage(error));
        });
      }
      return { action: "deny" };
    });
    window.webContents.on("will-navigate", (event, url) => {
      if (url !== window.webContents.getURL()) event.preventDefault();
    });
  }

  private async confirmAndClose(record: WorkspaceRecord): Promise<void> {
    if (record.closeInProgress || record.forceClosing) return;
    record.closeInProgress = true;
    try {
      const running = await this.isRunning(record);
      if (running || record.dirty) {
        const result = await dialog.showMessageBox(record.window, {
          type: "warning",
          buttons: [t("Keep Open"), record.dirty ? t("Discard and Close") : t("Stop and Close")],
          defaultId: 0,
          cancelId: 0,
          title: record.dirty ? t("Unsaved plugin changes") : t("A task is still running"),
          message: record.dirty
            ? t("Close this workspace and discard its unsaved plugin changes?")
            : t("Close this workspace and stop its active task?"),
          detail: running && record.dirty ? t("The active task will also be stopped.") : undefined,
        });
        if (result.response === 0) return;
      }
      record.forceClosing = true;
      this.records.delete(record.workspacePath);
      try {
        await this.persistState();
        await record.client?.shutdown();
      } catch (error) {
        this.records.set(record.workspacePath, record);
        record.forceClosing = false;
        throw error;
      }
      record.window.destroy();
    } finally {
      record.closeInProgress = false;
    }
  }

  /** Runs a sidecar command; settings writes are serialized and fanned out to other workspaces. */
  private invokeCommand(record: WorkspaceRecord, command: DesktopCommand, args: unknown): Promise<unknown> {
    const execute = async () => {
      if (!record.client) {
        throw new DesktopCommandError({ code: "unavailable", message: "Local agent runtime is not available." });
      }
      const result = await record.client.invoke(command, args);
      if (SETTINGS_COMMANDS.has(command)) await this.reloadOtherSettings(record);
      return result;
    };
    if (!SETTINGS_COMMANDS.has(command)) return execute();
    const queued = this.settingsQueue.then(execute, execute);
    this.settingsQueue = queued.catch(() => undefined);
    return queued;
  }

  private async reloadOtherSettings(source: WorkspaceRecord): Promise<void> {
    await Promise.all(
      [...this.records.values()]
        .filter((record) => record !== source && record.status.state === "ready")
        .map((record) => record.client?.invoke("reload_settings").catch(() => undefined)),
    );
  }

  private recordForSender(webContentsId: number): WorkspaceRecord {
    const record = [...this.records.values()].find((entry) => entry.window.webContents.id === webContentsId);
    if (!record) throw new Error("Workspace window is not registered.");
    return record;
  }

  private recordForEvent(event: IpcMainInvokeEvent): WorkspaceRecord {
    if (!event.senderFrame || event.senderFrame !== event.sender.mainFrame) {
      throw new Error("Desktop IPC is only available to the main application frame.");
    }
    return this.recordForSender(event.sender.id);
  }

  private async runningRecords(): Promise<WorkspaceRecord[]> {
    const flags = await Promise.all([...this.records.values()].map(async (record) => [record, await this.isRunning(record)] as const));
    return flags.filter(([, running]) => running).map(([record]) => record);
  }

  private async isRunning(record: WorkspaceRecord): Promise<boolean> {
    if ((record.client?.activePluginRuns() ?? 0) > 0) return true;
    try {
      const tasks = await record.client?.invoke<{ isRunning: boolean }>("get_task_runtime_status");
      if (tasks?.isRunning) return true;
      const snapshot = await record.client?.invoke<{ runtime?: { isRunning?: boolean } }>("get_app_snapshot");
      return snapshot?.runtime?.isRunning === true;
    } catch {
      return false;
    }
  }

  private setStatus(
    record: WorkspaceRecord,
    state: BackendStatus["state"],
    message: string,
    canRetry: boolean,
  ): void {
    record.status = { state, message, canRetry };
    if (!record.window.isDestroyed()) record.window.webContents.send("daily:backend-status", record.status);
  }

  private async loadRenderer(window: BrowserWindow): Promise<void> {
    if (!app.isPackaged && process.env.ELECTRON_RENDERER_URL) {
      if (!isSafeDevRendererUrl(process.env.ELECTRON_RENDERER_URL)) {
        throw new Error("Development renderer URL must use localhost over HTTP.");
      }
      await window.loadURL(process.env.ELECTRON_RENDERER_URL);
    } else {
      await window.loadFile(join(__dirname, "../renderer/index.html"));
    }
  }

  private servePluginModules(): void {
    session.defaultSession.protocol.handle("daily-plugin", async (request) => {
      const file = request.method === "GET" ? await this.pluginModules.readableFilePath(request.url) : null;
      if (!file) return new Response("Not found", { status: 404 });
      try {
        const [body, info] = await Promise.all([readFile(file), stat(file)]);
        return new Response(new Uint8Array(body), {
          headers: {
            "content-type": pluginContentType(file),
            "last-modified": info.mtime.toUTCString(),
            // The app window imports plugin modules from another origin.
            "access-control-allow-origin": "*",
          },
        });
      } catch {
        return new Response("Not found", { status: 404 });
      }
    });
  }

  private touchRecent(workspacePath: string): void {
    this.state.recentWorkspaces = withRecentWorkspace(this.state.recentWorkspaces, workspacePath);
  }

  private installMenu(): void {
    installApplicationMenu(this.state.recentWorkspaces, {
      chooseWorkspace: () => this.runMenuAction(() => this.chooseWorkspace()),
      openWorkspace: (workspacePath) => this.runMenuAction(() => this.openWorkspace(workspacePath)),
    });
  }

  private runMenuAction(action: () => Promise<void>): void {
    void action().catch((error) => {
      dialog.showErrorBox(t("Daily action failed"), errorMessage(error));
    });
  }

  private scheduleStateSave(): void {
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      void this.persistState().catch((error) => {
        console.error("Failed to persist Daily window state", error);
      });
    }, 250);
  }

  private persistState(): Promise<void> {
    const queued = this.stateSaveQueue.then(
      () => this.writeState(),
      () => this.writeState(),
    );
    this.stateSaveQueue = queued.catch(() => undefined);
    return queued;
  }

  private async writeState(): Promise<void> {
    const windows = [...this.records.values()]
      .filter((record) => !record.window.isDestroyed())
      .map((record) => ({
        workspacePath: record.workspacePath,
        bounds: record.window.getBounds(),
        maximized: record.window.isMaximized(),
      }));
    const next = {
      lastActive: this.state.lastActive,
      windows,
      recentWorkspaces: this.state.recentWorkspaces,
    };
    this.state = { ...this.state, ...next };
    await writeWindowState(next);
  }
}

function sidecarExecutable(): string {
  return app.isPackaged
    ? join(process.resourcesPath, "bin", sidecarExecutableName())
    : join(app.getAppPath(), "target", "debug", sidecarExecutableName());
}

function status(state: BackendStatus["state"], message: string, canRetry = false): BackendStatus {
  return { state, message, canRetry };
}
