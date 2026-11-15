import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "electron-vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const developmentCsp = {
  name: "daily-development-csp",
  apply: "serve" as const,
  transformIndexHtml(html: string) {
    // Vite injects an inline Fast Refresh preamble into the development page.
    return html.replace("script-src 'self'", "script-src 'self' 'unsafe-inline'");
  },
};

export default defineConfig({
  main: {
    build: {
      outDir: "out/main",
      lib: {
        entry: fileURLToPath(new URL("./electron/main/index.ts", import.meta.url)),
      },
    },
  },
  preload: {
    build: {
      outDir: "out/preload",
      lib: {
        entry: fileURLToPath(new URL("./electron/preload/index.ts", import.meta.url)),
      },
      rollupOptions: {
        output: {
          format: "cjs",
          entryFileNames: "index.js",
        },
      },
    },
  },
  renderer: {
    root: fileURLToPath(new URL(".", import.meta.url)),
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
        "@daily/plugin-sdk": fileURLToPath(new URL("./packages/plugin-sdk/src", import.meta.url)),
      },
    },
    plugins: [developmentCsp, tailwindcss(), react()],
    build: {
      outDir: "out/renderer",
      rollupOptions: {
        input: fileURLToPath(new URL("./index.html", import.meta.url)),
        output: {
          manualChunks(id) {
            if (!id.includes("node_modules")) return;
            if (id.includes("react-syntax-highlighter") || id.includes("refractor")) {
              return "syntax-highlighting";
            }
            if (
              id.includes("katex") ||
              id.includes("react-markdown") ||
              id.includes("remark-") ||
              id.includes("rehype-") ||
              id.includes("unified") ||
              id.includes("micromark")
            ) {
              return "markdown";
            }
            if (id.includes("@radix-ui")) return "radix-ui";
          },
        },
      },
    },
  },
});
