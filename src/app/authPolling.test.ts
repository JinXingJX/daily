import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pollAuthentication } from "./authPolling";

describe("login polling lifecycle", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("window", globalThis);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("stops checking credentials after navigation or changing authentication", async () => {
    let current = true;
    const readStatus = vi.fn().mockResolvedValue({ authenticated: false, errorCode: "not_logged_in" });
    const completed = vi.fn();
    const polling = pollAuthentication({ isCurrent: () => current, readStatus, onAuthenticated: completed });
    await vi.advanceTimersByTimeAsync(2_000);
    expect(readStatus).toHaveBeenCalledTimes(1);
    current = false;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(await polling).toBe("stopped");
    expect(readStatus).toHaveBeenCalledTimes(1);
    expect(completed).not.toHaveBeenCalled();
  });

  it("does not repeat a denied Keychain access request", async () => {
    const readStatus = vi.fn().mockResolvedValue({ authenticated: false, errorCode: "access_denied" });
    const polling = pollAuthentication({ isCurrent: () => true, readStatus, onAuthenticated: vi.fn() });
    await vi.advanceTimersByTimeAsync(120_000);
    expect(await polling).toBe("stopped");
    expect(readStatus).toHaveBeenCalledTimes(1);
  });

  it("refreshes the catalog once after login and reports a deadline for unfinished login", async () => {
    const completed = vi.fn().mockResolvedValue(undefined);
    const polling = pollAuthentication({ isCurrent: () => true, readStatus: vi.fn().mockResolvedValue({ authenticated: true }), onAuthenticated: completed });
    await vi.advanceTimersByTimeAsync(2_000);
    expect(await polling).toBe("authenticated");
    expect(completed).toHaveBeenCalledTimes(1);
    const unfinished = pollAuthentication({ isCurrent: () => true, readStatus: vi.fn().mockResolvedValue({ authenticated: false }), onAuthenticated: completed });
    await vi.advanceTimersByTimeAsync(120_000);
    expect(await unfinished).toBe("timeout");
    expect(completed).toHaveBeenCalledTimes(1);
  });
});
