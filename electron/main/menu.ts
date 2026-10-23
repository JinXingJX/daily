import { Menu } from "electron";
import { t } from "./i18n";

export type MenuActions = {
  chooseWorkspace: () => void;
  openWorkspace: (workspacePath: string) => void;
};

export function installApplicationMenu(recentWorkspaces: string[], actions: MenuActions): void {
  const recent = recentWorkspaces.map((workspacePath) => ({
    label: workspacePath,
    click: () => actions.openWorkspace(workspacePath),
  }));
  Menu.setApplicationMenu(
    Menu.buildFromTemplate([
      { role: "appMenu" },
      {
        label: t("File"),
        submenu: [
          { label: t("New Workspace Window"), accelerator: "CmdOrCtrl+Shift+N", click: actions.chooseWorkspace },
          { label: t("Open Workspace…"), accelerator: "CmdOrCtrl+O", click: actions.chooseWorkspace },
          { label: t("Recent Workspaces"), submenu: recent.length ? recent : [{ label: t("No Recent Workspaces"), enabled: false }] },
          { type: "separator" },
          { role: "close" },
        ],
      },
      { role: "editMenu" },
      { role: "viewMenu" },
      { role: "windowMenu" },
    ]),
  );
}
