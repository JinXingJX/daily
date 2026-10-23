// The Rust runtime writes its user-visible text (event view titles, runtime status, default
// titles) in English. Hosts and plugins call `localizeHostText` to show it in their language.
// Each entry is an English template from the runtime; `{name}` matches any text.

export type HostTextLocale = "en" | "zh-CN";

const ZH_CN: Array<[string, string]> = [
  // Agent progress
  ["Agent thinking", "Agent 思考"],
  ["Requesting {provider} / {model}.", "正在请求 {provider} / {model}。"],
  ["Tool call: {name}", "调用工具：{name}"],
  ["Tool result: {name}", "工具结果：{name}"],
  ["Tool done: {name}", "工具完成：{name}"],
  ["Tool failed: {name}", "工具失败：{name}"],
  ["Waiting for tool: {name}", "等待工具：{name}"],
  ["Running: {name}", "执行：{name}"],
  ["Created sub-agent: {name}", "创建子 agent：{name}"],
  ["Result from {name}", "{name} 的结果"],
  ["Final reply", "最终回复"],
  ["Final result", "最终结果"],
  ["Run started", "运行开始"],
  ["Run completed", "运行完成"],
  ["Run failed", "运行失败"],
  ["Request failed", "请求失败"],
  ["User input", "用户输入"],
  ["Agent started working.", "Agent 已开始处理。"],
  ["Agent received the request and started.", "Agent 已接收请求并开始运行。"],
  ["Task failed: {error}", "任务失败：{error}"],
  ["Stopped.", "已停止。"],
  // Compaction
  ["Compacted tool history", "压缩工具记录"],
  ["Folded {count} earlier tool results into a {chars}-character summary.", "已把 {count} 条较早的工具结果压缩为 {chars} 字的摘要。"],
  ["Tool history compaction failed", "压缩工具记录失败"],
  ["Summary failed; truncated earlier tool results instead: {error}", "摘要生成失败，已改为截断较早的工具结果：{error}"],
  ["Compacted context", "压缩上下文"],
  ["Folded {count} earlier conversation entries into a summary.", "已把较早的 {count} 条对话记录压缩为摘要。"],
  ["Context compaction failed", "压缩上下文失败"],
  ["Summary failed; truncated earlier entries instead: {error}", "摘要生成失败，已改为截断较早的记录：{error}"],
  // Runtime status
  ["Ready", "就绪"],
  ["Settings updated", "设置已更新"],
  ["Turn queued", "已排队"],
  ["Streaming response", "正在输出回复"],
  ["Ply {ply} running", "第 {ply} 轮运行中"],
  ["Tool selected on ply {ply}", "第 {ply} 轮已选择工具"],
  ["Tool result collected on ply {ply}", "第 {ply} 轮已收到工具结果"],
  ["Final answer selected", "已生成最终回复"],
  ["Turn failed", "运行失败"],
  ["Continuing turn", "正在从断点继续"],
  ["Waiting for async tool", "等待异步工具"],
  ["Resuming after async tool", "异步工具完成，继续运行"],
  ["Running on_start hooks.", "正在运行开始前钩子"],
  ["Running on_finish hooks", "正在运行完成后钩子"],
  // Settings
  ["Project settings define auth.configs, which are ignored: provider endpoints and keys are only read from user or local settings so a shared repository cannot redirect your credentials.", "项目设置中定义的 auth.configs 已被忽略：提供商地址与密钥只从用户或本地设置读取，避免共享仓库把你的凭据发往别处。"],
  // Default titles
  ["New task", "新任务"],
  ["New conversation", "新对话"],
];

function compile(template: string): RegExp {
  const pattern = template
    .split(/(\{\w+\})/)
    .map((part) => (/^\{\w+\}$/.test(part) ? `(?<${part.slice(1, -1)}>[\\s\\S]+?)` : part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
    .join("");
  return new RegExp(`^${pattern}$`);
}

const COMPILED = ZH_CN.map(([english, chinese]) => [compile(english), chinese] as const);

/** Translates a runtime-produced string; unknown text is returned unchanged. */
export function localizeHostText(text: string, locale: HostTextLocale): string {
  if (locale !== "zh-CN" || !text) return text;
  for (const [pattern, template] of COMPILED) {
    const match = pattern.exec(text);
    if (match) return template.replace(/\{(\w+)\}/g, (_, name: string) => match.groups?.[name] ?? "");
  }
  return text;
}
