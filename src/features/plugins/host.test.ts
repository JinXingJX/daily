import { describe, expect, it, vi } from "vitest";
import { createPluginHost } from "@daily/plugin-sdk/host";
import { PLUGIN_RPC_METHODS } from "@daily/plugin-sdk/methods.generated";

function hostMethods() {
  const source = createPluginHost.toString();
  return new Set([...source.matchAll(/request\("([a-zA-Z0-9.]+)"/g)].map((match) => match[1]));
}

describe("plugin host API", () => {
  it("exposes exactly the RPC methods the Rust host dispatches", () => {
    const exposed = hostMethods();
    for (const method of PLUGIN_RPC_METHODS) expect(exposed, method).toContain(method);
    for (const method of exposed) expect(PLUGIN_RPC_METHODS, method).toContain(method);
  });

  it("sends requests through the transport and unwraps their results", async () => {
    const request = vi.fn(async () => ({ value: { text: "hello" } }));
    const host = createPluginHost("local.test", { request, on: () => () => undefined, setDirty: () => undefined });
    await expect(host.storage.get("draft")).resolves.toEqual({ text: "hello" });
    expect(request).toHaveBeenCalledWith("storage.get", { key: "draft" });
    expect(host.pluginId).toBe("local.test");
  });

  it("reports unsaved changes to the app without a host round trip", async () => {
    const request = vi.fn();
    const setDirty = vi.fn();
    const host = createPluginHost("local.test", { request, on: () => () => undefined, setDirty });
    await host.host.setDirty(true);
    expect(setDirty).toHaveBeenCalledWith(true);
    expect(request).not.toHaveBeenCalled();
  });
});
