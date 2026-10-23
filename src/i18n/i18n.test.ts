import { afterEach, describe, expect, it } from "vitest";
import { setLocaleForTests, t } from "@/i18n";
import { zhCN } from "@/i18n/zh-CN";

const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

describe("i18n", () => {
  afterEach(() => setLocaleForTests("zh-CN"));

  it("keeps the same placeholders in every translation", () => {
    for (const [key, value] of Object.entries(zhCN)) {
      expect(placeholders(value), key).toEqual(placeholders(key));
      expect(value.trim(), key).not.toBe("");
    }
  });

  it("uses the English key as the English text and fills placeholders", () => {
    setLocaleForTests("en");
    expect(t("Step {step}", { step: 3 })).toBe("Step 3");
    setLocaleForTests("zh-CN");
    expect(t("Step {step}", { step: 3 })).toBe("第 3 步");
  });
});

describe("localizeHostText", () => {
  it("translates runtime templates and leaves other text alone", async () => {
    const { localizeHostText } = await import("@daily/plugin-sdk/hostText");
    expect(localizeHostText("Tool call: workspace.read", "zh-CN")).toBe("调用工具：workspace.read");
    expect(localizeHostText("Folded 3 earlier tool results into a 900-character summary.", "zh-CN")).toBe(
      "已把 3 条较早的工具结果压缩为 900 字的摘要。",
    );
    expect(localizeHostText("Tool call: workspace.read", "en")).toBe("Tool call: workspace.read");
    expect(localizeHostText("The model said Tool call: x", "zh-CN")).toBe("The model said Tool call: x");
  });
});
