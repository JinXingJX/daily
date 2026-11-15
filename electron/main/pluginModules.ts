import { createHash } from "node:crypto";
import { realpath } from "node:fs/promises";
import { extname, resolve, sep } from "node:path";

const CONTENT_TYPES: Record<string, string> = {
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".webp": "image/webp",
  ".gif": "image/gif",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
};

/**
 * Maps plugin folders to `daily-plugin://<folder id>/` origins, so the renderer can import a
 * plugin's entry module and the module can reach its own chunks and assets by relative URL.
 */
export class PluginModules {
  private readonly roots = new Map<string, string>();

  /** Registers a plugin folder and returns the URL of a module inside it. */
  moduleUrl(root: string, entry: string, revision: string): string {
    const id = createHash("sha256").update(root).digest("hex").slice(0, 32);
    this.roots.set(id, root);
    const path = entry.split(/[\\/]+/).filter(Boolean).map(encodeURIComponent).join("/");
    return `daily-plugin://${id}/${path}?v=${encodeURIComponent(revision)}`;
  }

  /** Resolves the URL lexically; serving a file also requires readableFilePath's real-path check. */
  filePath(requestUrl: string): string | null {
    try {
      const url = new URL(requestUrl);
      const root = url.protocol === "daily-plugin:" ? this.roots.get(url.hostname) : undefined;
      if (!root) return null;
      const file = resolve(root, `.${decodeURIComponent(url.pathname)}`);
      return file.startsWith(root + sep) ? file : null;
    } catch {
      return null;
    }
  }

  /** Rejects symlinks and junctions that resolve outside the plugin's registered folder. */
  async readableFilePath(requestUrl: string): Promise<string | null> {
    const file = this.filePath(requestUrl);
    if (!file) return null;
    const root = this.roots.get(new URL(requestUrl).hostname);
    if (!root) return null;
    try {
      const [canonicalRoot, canonicalFile] = await Promise.all([realpath(root), realpath(file)]);
      return canonicalFile.startsWith(canonicalRoot + sep) ? canonicalFile : null;
    } catch {
      return null;
    }
  }
}

export function pluginContentType(file: string): string {
  return CONTENT_TYPES[extname(file).toLowerCase()] ?? "application/octet-stream";
}
