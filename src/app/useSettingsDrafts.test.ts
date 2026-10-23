import { describe, expect, it } from "vitest";
import { authConfigOptions } from "@/app/useSettingsDrafts";

describe("authentication configuration choices", () => {
  it("adds Antigravity without modifying the existing Codex configuration", () => {
    const codex = authConfigOptions([])[0];
    codex.id = "custom-codex";
    codex.codexHome = "/tmp/custom-codex";
    const configs = [codex];
    const options = authConfigOptions(configs);
    expect(configs).toHaveLength(1);
    expect(options[0]).toBe(codex);
    expect(options[0].codexHome).toBe("/tmp/custom-codex");
    expect(options[1].providerKind).toBe("antigravity_login");
    expect(options[1].models).toEqual([]);
    expect(options[1].defaultModel).toBe("");
    expect(options[1].codexHome).toBeNull();
  });

  it("preserves saved Antigravity selections and does not duplicate providers", () => {
    const configs = authConfigOptions([]);
    configs[1] = { ...configs[1], models: ["gemini-pro", "claude-sonnet"], defaultModel: "claude-sonnet" };
    expect(authConfigOptions(configs)).toEqual(configs);
  });
});
