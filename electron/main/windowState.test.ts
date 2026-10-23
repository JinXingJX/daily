import { describe, expect, it, vi } from "vitest";

vi.mock("electron", () => ({ app: {}, screen: {}, session: {} }));

const { parseWindowState, withRecentWorkspace } = await import("./windowState");
const { isSafeDevRendererUrl, isSafeExternalUrl } = await import("./security");

describe("window state", () => {
  it("drops malformed or unsafe saved windows", () => {
    const state = parseWindowState({
      version: 1,
      lastActive: "/work/a",
      windows: [
        { workspacePath: "/work/a", bounds: { x: 0, y: 0, width: 800, height: 600 }, maximized: true },
        { workspacePath: "relative/path", bounds: { x: 0, y: 0, width: 800, height: 600 } },
        { workspacePath: "/work/b", bounds: { x: 0, y: 0, width: 10, height: 600 } },
      ],
      recentWorkspaces: ["/work/a", "/work/a", 3, " "],
    });
    expect(state.windows.map((window) => window.workspacePath)).toEqual(["/work/a"]);
    expect(state.recentWorkspaces).toEqual(["/work/a"]);
  });

  it("resets unknown versions and keeps recent workspaces bounded", () => {
    expect(parseWindowState({ version: 2, windows: [] }).windows).toEqual([]);
    const recent = Array.from({ length: 20 }, (_, index) => `/w/${index}`);
    expect(withRecentWorkspace(recent, "/w/5")).toHaveLength(12);
    expect(withRecentWorkspace(recent, "/w/5")[0]).toBe("/w/5");
  });
});

describe("navigation guards", () => {
  it("only opens https links externally and loads dev renderers from localhost", () => {
    expect(isSafeExternalUrl("https://example.com")).toBe(true);
    expect(isSafeExternalUrl("file:///etc/passwd")).toBe(false);
    expect(isSafeDevRendererUrl("http://127.0.0.1:5173")).toBe(true);
    expect(isSafeDevRendererUrl("http://evil.example:5173")).toBe(false);
  });
});
