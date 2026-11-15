import { app, BrowserWindow, dialog, protocol } from "electron";
import { errorMessage } from "./util";
import { WorkspaceWindowManager } from "./workspaceManager";
import { t } from "./i18n";

const manager = new WorkspaceWindowManager();
// Plugin modules are imported into the app window from their own folder origins.
protocol.registerSchemesAsPrivileged([
  { scheme: "daily-plugin", privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true, codeCache: true } },
]);
let quitRequested = false;

app.whenReady().then(() => manager.initialize()).catch((error) => {
  console.error("Failed to initialize Daily", error);
  dialog.showErrorBox(t("Daily could not start"), errorMessage(error));
  app.quit();
});
app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    void manager.chooseWorkspace().catch((error) => dialog.showErrorBox(t("Could not open workspace"), errorMessage(error)));
  }
});
app.on("before-quit", (event) => {
  if (quitRequested) return;
  event.preventDefault();
  quitRequested = true;
  void manager.requestQuit().finally(() => {
    quitRequested = false;
  });
});
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});
