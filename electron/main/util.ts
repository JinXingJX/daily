import { existsSync } from "node:fs";
import { realpath, stat } from "node:fs/promises";

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Local agent runtime failed.";
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export async function canonicalDirectory(path: string): Promise<string> {
  const resolved = await realpath(path);
  if (!(await isDirectory(resolved))) throw new Error(`Workspace is not a directory: ${resolved}`);
  return resolved;
}

export async function isDirectory(path: string): Promise<boolean> {
  if (!existsSync(path)) return false;
  try {
    return (await stat(path)).isDirectory();
  } catch {
    return false;
  }
}
