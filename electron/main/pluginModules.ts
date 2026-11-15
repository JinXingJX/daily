import { createHash } from "node:crypto";
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

  /** The file a request names, or null when it is not inside a registered plugin folder. */
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
}

export function pluginContentType(file: string): string {
  return CONTENT_TYPES[extname(file).toLowerCase()] ?? "application/octet-stream";
}
