import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      "@daily/plugin-sdk": fileURLToPath(new URL("./packages/plugin-sdk/src", import.meta.url)),
    },
  },
  test: {
    include: ["src/**/*.test.{ts,tsx}", "electron/**/*.test.ts"],
    // Renderer tests assert the Chinese UI; the i18n test covers English separately.
    setupFiles: ["src/test/setup.ts"],
  },
});
