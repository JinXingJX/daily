import { DESKTOP_COMMANDS } from "./commands.generated";

export { DESKTOP_COMMANDS };

export type DesktopCommand = (typeof DESKTOP_COMMANDS)[number];

export type BackendStatus = {
  state: "starting" | "ready" | "restarting" | "failed" | "stopped";
  message: string;
  canRetry: boolean;
};

export type WorkspaceInfo = {
  path: string;
  name: string;
};

export type AppEventMessage = {
  event: string;
  payload: unknown;
};

export const isDesktopCommand = (value: unknown): value is DesktopCommand =>
  typeof value === "string" && (DESKTOP_COMMANDS as readonly string[]).includes(value);


/** Error returned by a desktop command, carrying the sidecar's stable error code. */
export type DesktopErrorPayload = { code: string; message: string };

/** IPC envelope for `daily:invoke`; Electron drops custom Error fields across IPC. */
export type InvokeResult<T = unknown> = { ok: true; result: T } | { ok: false; error: DesktopErrorPayload };

export class DesktopCommandError extends Error {
  readonly code: string;

  constructor({ code, message }: DesktopErrorPayload) {
    super(message);
    this.name = "DesktopCommandError";
    this.code = code;
  }
}
