import { describe, expect, it, vi } from "vitest";
import { definePlugin, type DailyPluginApi } from "@daily/plugin-sdk";
import type { PluginManifest } from "@/types";
import { PluginRuntime } from "./pluginRuntime";

function manifest(patch: Partial<PluginManifest> = {}): PluginManifest {
  return {
    identifier: "local.notes", name: "Notes", description: "", payloadHint: "", version: "1.0.0", author: "Test",
    source: "local_dev", entry: "dist/plugin.js", devEntry: null, devUrl: null, activation: "startup",
    installed: true, enabled: true, builtIn: false,
    contributes: { renderers: [], agentTools: [], views: [{ id: "notes.main", title: "Notes", icon: null }], themes: [], hooks: [], eventViewParsers: [] },
    settingsSchema: {}, settings: {}, runtimeErrors: [],
    ...patch,
  };
}

function setup(activate: (host: DailyPluginApi) => void | (() => void) | Promise<void | (() => void)> = () => undefined) {
  const definition = definePlugin({ activate, views: { "notes.main": { mount: () => undefined } } });
  const importPlugin = vi.fn(async () => ({ definition, url: "daily-plugin://notes/dist/plugin.js?v=1" }));
  const builtModuleUrl = vi.fn(async () => "daily-plugin://notes/dist/plugin.js?v=1");
  const request = vi.fn(async () => ({ value: 1 }));
  return { runtime: new PluginRuntime({ importPlugin, builtModuleUrl, request }), importPlugin, builtModuleUrl, request };
}

describe("plugin runtime", () => {
  it("activates a plugin once and shares the instance between its views and background work", async () => {
    const activate = vi.fn();
    const { runtime, importPlugin } = setup(activate);
    runtime.sync([manifest()]);
    const [first, second] = await Promise.all([runtime.load(manifest()), runtime.load(manifest())]);
    expect(first).toBe(second);
    expect(importPlugin).toHaveBeenCalledTimes(1);
    expect(activate).toHaveBeenCalledTimes(1);
    expect(Object.keys(first.views)).toEqual(["notes.main"]);
  });

  it("starts only plugins that activate at startup and leaves built-in ones alone", async () => {
    const { runtime, importPlugin } = setup();
    runtime.sync([manifest({ activation: "onView" }), manifest({ identifier: "builtin.chart", builtIn: true, entry: null })]);
    expect(importPlugin).not.toHaveBeenCalled();
  });

  it("stops a plugin when it is disabled and bumps its generation so views remount", async () => {
    const stop = vi.fn();
    const { runtime } = setup(() => stop);
    await runtime.load(manifest());
    const changed = vi.fn();
    runtime.subscribe(changed);
    runtime.sync([manifest({ enabled: false })]);
    expect(stop).toHaveBeenCalledTimes(1);
    expect(runtime.generation("local.notes")).toBe(1);
    expect(changed).toHaveBeenCalled();
  });

  it("routes host calls and events by plugin and tracks unsaved changes", async () => {
    let host!: DailyPluginApi;
    const { runtime, request } = setup((api) => { host = api; });
    await runtime.load(manifest());
    await host.storage.get("draft");
    expect(request).toHaveBeenCalledWith("local.notes", "storage.get", { key: "draft" });

    const progress = vi.fn();
    host.on("agent.progress", progress);
    runtime.dispatch("other.plugin", "agent.progress", { streamId: "a" });
    runtime.dispatch("local.notes", "agent.progress", { streamId: "b" });
    expect(progress.mock.calls).toEqual([[{ streamId: "b" }]]);

    expect(runtime.isDirty()).toBe(false);
    await host.host.setDirty(true);
    expect(runtime.isDirty()).toBe(true);
    runtime.unload("local.notes");
    expect(runtime.isDirty()).toBe(false);
  });

  it("restarts a plugin only when its built module changed on disk", async () => {
    const stop = vi.fn();
    const { runtime, builtModuleUrl } = setup(() => stop);
    await runtime.load(manifest());
    await runtime.reloadChanged();
    expect(stop).not.toHaveBeenCalled();
    builtModuleUrl.mockResolvedValue("daily-plugin://notes/dist/plugin.js?v=2");
    await runtime.reloadChanged();
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("cleans up a plugin that is unloaded while it is still starting", async () => {
    const stop = vi.fn();
    const { runtime } = setup(() => stop);
    const loading = runtime.load(manifest());
    runtime.unload("local.notes");
    await expect(loading).rejects.toThrow("stopped");
  });

  it("cleans up activation that finishes after unloading", async () => {
    let finish!: (cleanup: () => void) => void;
    const stop = vi.fn();
    const { runtime } = setup(() => new Promise<() => void>((resolve) => { finish = resolve; }));
    const loading = runtime.load(manifest());
    await Promise.resolve();
    runtime.unload("local.notes");
    finish(stop);
    await expect(loading).rejects.toThrow("stopped");
    expect(stop).toHaveBeenCalledOnce();
  });

  it("rejects calls from a stopped instance and does not deliver more events", async () => {
    let host!: DailyPluginApi;
    const { runtime, request } = setup((api) => { host = api; });
    await runtime.load(manifest());
    const listener = vi.fn();
    host.on("agent.progress", listener);
    runtime.unload("local.notes");
    await expect(host.storage.get("draft")).rejects.toThrow("stopped");
    host.on("agent.progress", listener);
    runtime.dispatch("local.notes", "agent.progress", {});
    expect(request).not.toHaveBeenCalled();
    expect(listener).not.toHaveBeenCalled();
    await expect(runtime.load(manifest({ enabled: false }))).rejects.toThrow("not enabled");
  });
});
