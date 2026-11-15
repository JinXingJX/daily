import { describe, expect, it } from "vitest";
import { join } from "node:path";
import { PluginModules, pluginContentType } from "./pluginModules";

describe("plugin module origins", () => {
  it("serves files of a registered plugin folder under a stable origin", () => {
    const modules = new PluginModules();
    const root = join("/plugins", "writer", ".daily-plugin");
    const url = modules.moduleUrl(root, "dist/plugin.js", "1700000000000-42");
    expect(url).toMatch(/^daily-plugin:\/\/[0-9a-f]{32}\/dist\/plugin\.js\?v=1700000000000-42$/);
    expect(modules.filePath(url)).toBe(join(root, "dist", "plugin.js"));
    // A chunk the entry imports by relative URL resolves inside the same folder.
    expect(modules.filePath(new URL("./chunk-a.js", url).toString())).toBe(join(root, "dist", "chunk-a.js"));
    // Rebuilding changes only the revision, so the origin (and its caches) stay the same.
    expect(new URL(modules.moduleUrl(root, "dist/plugin.js", "2")).host).toBe(new URL(url).host);
  });

  it("resolves nothing outside registered folders", () => {
    const modules = new PluginModules();
    const url = new URL(modules.moduleUrl(join("/plugins", "writer"), "plugin.js", "1"));
    // Encoded slashes survive URL normalization and must still stay inside the folder.
    expect(modules.filePath(`daily-plugin://${url.host}/..%2f..%2fetc/passwd`)).toBeNull();
    expect(modules.filePath(`daily-plugin://${url.host}/%2e%2e/%2e%2e/etc/passwd`)).toBe(join("/plugins", "writer", "etc", "passwd"));
    expect(modules.filePath(`daily-plugin://${url.host}/`)).toBeNull();
    expect(modules.filePath("daily-plugin://unknown/plugin.js")).toBeNull();
    expect(modules.filePath("file:///etc/passwd")).toBeNull();
  });

  it("labels modules as JavaScript so the renderer can import them", () => {
    expect(pluginContentType("/p/dist/plugin.js")).toContain("text/javascript");
    expect(pluginContentType("/p/dist/logo.PNG")).toBe("image/png");
    expect(pluginContentType("/p/dist/data.bin")).toBe("application/octet-stream");
  });
});
