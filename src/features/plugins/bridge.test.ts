import { describe, expect, it } from "vitest";
import { bridgeScript, installDailyBridge } from "@daily/plugin-sdk/bridge";
import { PLUGIN_RPC_METHODS } from "@daily/plugin-sdk/methods.generated";

// Handled by the renderer host itself; never forwarded to the Rust sidecar.
const RENDERER_METHODS = ["host.setDirty"];

function bridgeMethods() {
  const source = installDailyBridge.toString();
  return new Set([...source.matchAll(/request\("([a-zA-Z.]+)"/g)].map((match) => match[1]));
}

describe("plugin bridge", () => {
  it("exposes exactly the RPC methods the Rust host dispatches", () => {
    const exposed = bridgeMethods();
    for (const method of PLUGIN_RPC_METHODS) expect(exposed, method).toContain(method);
    for (const method of exposed) {
      expect([...PLUGIN_RPC_METHODS, ...RENDERER_METHODS], method).toContain(method);
    }
  });

  it("runs from its serialized source and posts requests to the parent frame", () => {
    const posted: Array<Record<string, unknown>> = [];
    const fakeWindow: Record<string, unknown> = { addEventListener: () => undefined };
    const fakeParent = { postMessage: (message: Record<string, unknown>) => posted.push(message) };
    new Function("window", "parent", bridgeScript("local.test", "main"))(fakeWindow, fakeParent);

    const api = fakeWindow.dailyPlugin as { storage: { get: (key: string) => Promise<unknown> } };
    void api.storage.get("draft");
    expect(posted).toEqual([
      expect.objectContaining({ source: "daily-plugin", pluginId: "local.test", viewId: "main", method: "storage.get", payload: { key: "draft" } }),
    ]);
  });
});
