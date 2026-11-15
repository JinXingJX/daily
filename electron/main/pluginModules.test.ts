import { describe, expect, it } from "vitest";
import { join, resolve } from "node:path";
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { PluginModules, pluginContentType } from "./pluginModules";

describe("plugin module origins", () => {
  it("serves real plugin files while blocking symlinks and junctions outside the folder", async () => {
    const temporary = await mkdtemp(join(tmpdir(), "daily plugin 中文-"));
    try {
      const root = join(temporary, "writer");
      const outside = join(temporary, "outside");
      await Promise.all([mkdir(root), mkdir(outside)]);
      await writeFile(join(root, "plugin.js"), "export default {};");
      await writeFile(join(outside, "private.txt"), "outside fixture");
      await symlink(outside, join(root, "escape"), process.platform === "win32" ? "junction" : "dir");
      const modules = new PluginModules();
      const url = modules.moduleUrl(root, "plugin.js", "1");
      expect(await modules.readableFilePath(url)).toBe(await realpath(join(root, "plugin.js")));
      expect(await modules.readableFilePath(new URL("escape/private.txt", url).toString())).toBeNull();
      expect(await modules.readableFilePath(new URL("missing.js", url).toString())).toBeNull();
      expect(await modules.readableFilePath("daily-plugin://unknown/plugin.js")).toBeNull();
    } finally {
      await rm(temporary, { force: true, recursive: true });
    }
  });

  it("serves files of a registered plugin folder under a stable origin", () => {
    const modules = new PluginModules();
    const root = resolve("/plugins", "writer", ".daily-plugin");
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
    const root = resolve("/plugins", "writer");
    const url = new URL(modules.moduleUrl(root, "plugin.js", "1"));
    // Encoded slashes survive URL normalization and must still stay inside the folder.
    expect(modules.filePath(`daily-plugin://${url.host}/..%2f..%2fetc/passwd`)).toBeNull();
    expect(modules.filePath(`daily-plugin://${url.host}/%2e%2e/%2e%2e/etc/passwd`)).toBe(join(root, "etc", "passwd"));
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
