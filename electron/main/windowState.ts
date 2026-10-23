import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";
import { app, screen } from "electron";
import { isRecord } from "./util";

const STATE_VERSION = 1;
const MAX_RECENT_WORKSPACES = 12;

export type SavedWindow = {
  workspacePath: string;
  bounds: Electron.Rectangle;
  maximized: boolean;
};

export type WindowState = {
  version: number;
  lastActive: string | null;
  windows: SavedWindow[];
  recentWorkspaces: string[];
};

export function emptyWindowState(): WindowState {
  return { version: STATE_VERSION, lastActive: null, windows: [], recentWorkspaces: [] };
}

export function withRecentWorkspace(recent: string[], workspacePath: string): string[] {
  return [workspacePath, ...recent.filter((path) => path !== workspacePath)].slice(0, MAX_RECENT_WORKSPACES);
}

export async function loadWindowState(): Promise<WindowState> {
  try {
    return parseWindowState(JSON.parse(await readFile(windowStatePath(), "utf8")));
  } catch {
    return emptyWindowState();
  }
}

export async function writeWindowState(state: Omit<WindowState, "version">): Promise<WindowState> {
  const next: WindowState = { version: STATE_VERSION, ...state };
  const path = windowStatePath();
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(next, null, 2)}\n`, "utf8");
  await rename(temporary, path);
  return next;
}

/** Validates untrusted on-disk window state, dropping anything malformed. */
export function parseWindowState(value: unknown): WindowState {
  if (!isRecord(value) || value.version !== STATE_VERSION || !Array.isArray(value.windows)) return emptyWindowState();
  const windows = value.windows.flatMap((entry) => {
    const saved = parseSavedWindow(entry);
    return saved ? [saved] : [];
  });
  const recentWorkspaces = Array.isArray(value.recentWorkspaces)
    ? [
        ...new Set(
          value.recentWorkspaces.filter((entry): entry is string => typeof entry === "string" && entry.trim().length > 0),
        ),
      ].slice(0, MAX_RECENT_WORKSPACES)
    : [];
  return {
    version: STATE_VERSION,
    lastActive: typeof value.lastActive === "string" ? value.lastActive : null,
    windows,
    recentWorkspaces,
  };
}

export function parseSavedWindow(value: unknown): SavedWindow | null {
  if (!isRecord(value) || typeof value.workspacePath !== "string" || !isRecord(value.bounds)) return null;
  if (!isAbsolute(value.workspacePath) || value.workspacePath.length > 4_096) return null;
  const { x, y, width, height } = value.bounds;
  if (![x, y, width, height].every((item) => typeof item === "number" && Number.isFinite(item))) return null;
  if ((width as number) < 320 || (height as number) < 240 || (width as number) > 16_384 || (height as number) > 16_384) return null;
  return {
    workspacePath: value.workspacePath,
    bounds: { x: x as number, y: y as number, width: width as number, height: height as number },
    maximized: value.maximized === true,
  };
}

/** Returns the saved bounds only if a meaningful part is still on a connected display. */
export function visibleSavedBounds(bounds: Electron.Rectangle): Electron.Rectangle | null {
  const visible = screen.getAllDisplays().some((display) => {
    const left = Math.max(bounds.x, display.workArea.x);
    const top = Math.max(bounds.y, display.workArea.y);
    const right = Math.min(bounds.x + bounds.width, display.workArea.x + display.workArea.width);
    const bottom = Math.min(bounds.y + bounds.height, display.workArea.y + display.workArea.height);
    return right - left >= 120 && bottom - top >= 80;
  });
  return visible ? bounds : null;
}

function windowStatePath(): string {
  return join(app.getPath("userData"), "window-state.json");
}
