import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  resolve: { alias: {
    "@daily/plugin-sdk": fileURLToPath(new URL("../../packages/plugin-sdk/src/index.ts", import.meta.url)),
    "@/i18n": fileURLToPath(new URL("./src/i18n/index.ts", import.meta.url)),
  } },
  server: { cors: true, fs: { allow: [fileURLToPath(new URL("../..", import.meta.url))] } },
  build: {
    outDir: ".daily-plugin/dist",
    emptyOutDir: true,
    lib: { entry: "src/main.tsx", formats: ["es"], fileName: () => "plugin.js" },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
