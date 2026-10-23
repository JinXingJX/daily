type AuthStatus = { authenticated: boolean; errorCode?: string | null };

/** Stops on navigation, terminal credential errors, success, or the login deadline. */
export async function pollAuthentication({ isCurrent, readStatus, onAuthenticated }: {
  isCurrent: () => boolean;
  readStatus: () => Promise<AuthStatus>;
  onAuthenticated: () => Promise<void>;
}): Promise<"authenticated" | "stopped" | "timeout"> {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline && isCurrent()) {
    await new Promise((resolve) => window.setTimeout(resolve, 2_000));
    if (!isCurrent()) return "stopped";
    const status = await readStatus();
    if (!isCurrent()) return "stopped";
    if (status.errorCode && status.errorCode !== "not_logged_in") return "stopped";
    if (status.authenticated) {
      await onAuthenticated();
      return isCurrent() ? "authenticated" : "stopped";
    }
  }
  return isCurrent() ? "timeout" : "stopped";
}
