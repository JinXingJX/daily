import config from "./vite.config";
import { defineConfig, mergeConfig } from "vitest/config";
export default mergeConfig(config, defineConfig({ test: { include: ["src/**/*.test.ts"] } }));
