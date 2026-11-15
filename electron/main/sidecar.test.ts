import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { isDesktopCommand } from "../shared/contracts";
import { parseIncomingFrame, requestTimeout, SidecarClient, sidecarExecutableName } from "./sidecar";

const cleanupPaths: string[] = [];
const clients: SidecarClient[] = [];
const executable = resolve(process.env.DAILY_TEST_SIDECAR ?? join("target/debug", sidecarExecutableName()));

afterEach(async () => {
  await Promise.all(clients.splice(0).map((client) => client.shutdown()));
  await Promise.all(cleanupPaths.splice(0).map((path) => rm(path, { force: true, recursive: true })));
});

describe("desktop command contract", () => {
  it("accepts the fixed command surface and rejects arbitrary IPC methods", () => {
    expect(isDesktopCommand("get_app_snapshot")).toBe(true);
    expect(isDesktopCommand("get_antigravity_auth_status")).toBe(true);
    expect(isDesktopCommand("start_antigravity_login")).toBe(true);
    expect(isDesktopCommand("get_antigravity_model_catalog")).toBe(true);
    expect(isDesktopCommand("refresh_antigravity_models")).toBe(true);
    expect(isDesktopCommand("save_auth_settings_content")).toBe(true);
    expect(isDesktopCommand("plugin_host_rpc")).toBe(true);
    expect(isDesktopCommand("shell.exec")).toBe(false);
    expect(isDesktopCommand(null)).toBe(false);
  });
});

describe("request timeouts", () => {
  it("gives long-running plugin RPCs no deadline, reading the renderer's input envelope", () => {
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "model.invoke", payload: {} } })).toBeNull();
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "agent.task.resume", payload: {} } })).toBeNull();
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "agent.task.retry", payload: {} } })).toBeNull();
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "image.generate", payload: {} } })).toBeNull();
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "plugin.files.readText", payload: {} } })).toBe(30_000);
    expect(requestTimeout("plugin_host_rpc", { input: { pluginId: "p", method: "agent.session.get", payload: {} } })).toBe(30_000);
    expect(requestTimeout("run_session_turn", {})).toBe(30_000);
  });
});

describe("sidecar frame validation", () => {
  it("accepts valid frames and rejects malformed or unknown frames", () => {
    expect(parseIncomingFrame('{"kind":"event","event":"app://state","payload":{}}').kind).toBe("event");
    expect(() => parseIncomingFrame("null")).toThrow("must be an object");
    expect(() => parseIncomingFrame('{"kind":"response","id":"1","ok":false}')).toThrow("missing an error");
    expect(() => parseIncomingFrame('{"kind":"surprise"}')).toThrow("Unknown protocol frame kind");
  });
});

describe("SidecarClient", () => {
  it("uses the native executable name on each supported platform", () => {
    expect(sidecarExecutableName("win32")).toBe("daily-app-server.exe");
    expect(sidecarExecutableName("darwin")).toBe("daily-app-server");
    expect(sidecarExecutableName("linux")).toBe("daily-app-server");
  });

  it("handshakes, correlates responses, forwards events, and reports command errors", async () => {
    const workspacePath = await mkdtemp(join(tmpdir(), "daily sidecar 中文-"));
    cleanupPaths.push(workspacePath);
    const client = new SidecarClient(executable, workspacePath);
    clients.push(client);

    const events: Array<{ event: string; payload: unknown }> = [];
    client.on("event", (event) => events.push(event));
    await client.start();

    const snapshot = await client.invoke<{ workspace: { rootId: number } }>("get_app_snapshot");
    expect(snapshot.workspace.rootId).toBeTypeOf("number");

    await client.invoke("create_workspace_node", {
      input: { parentId: snapshot.workspace.rootId, kind: "session", name: "Protocol Test" },
    });
    expect(events.some((event) => event.event === "app://state")).toBe(true);

    await expect(client.invoke("not_a_real_command")).rejects.toThrow("Unknown desktop command");
    await expect(client.invoke("not_a_real_command")).rejects.toMatchObject({ code: "unknown_command" });
    await expect(client.invoke("open_session", { input: { sessionId: "nope" } })).rejects.toMatchObject({
      code: "invalid_params",
    });
  }, 20_000);

  it("waits for readiness when a request arrives while the runtime is still starting", async () => {
    const workspacePath = await mkdtemp(join(tmpdir(), "daily-sidecar-test-"));
    cleanupPaths.push(workspacePath);
    const client = new SidecarClient(executable, workspacePath);
    clients.push(client);
    const starting = client.start();
    const snapshot = await client.invoke<{ workspace: { rootId: number } }>("get_app_snapshot");
    await starting;
    expect(snapshot.workspace.rootId).toBeTypeOf("number");
  }, 20_000);
});
