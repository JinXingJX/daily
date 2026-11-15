// The Rust runtime writes its user-visible text (event view titles, runtime status, default
// titles) in English. Hosts and plugins call `localizeHostText` to show it in their language.
// Each entry is an English template from the runtime; `{name}` matches any text.

export type HostTextLocale = "en" | "zh-CN";

const ZH_CN: Array<[string, string]> = [
  // Shared agent execution and checkpoint recovery
  ["Run cancelled.", "运行已取消。"],
  ["Run deadline exceeded.", "运行已超过截止时间。"],
  ["Shared model request or token budget exhausted.", "多个 agent 共享的模型请求或 token 预算已用尽。"],
  ["Shared tool call budget exhausted.", "多个 agent 共享的工具调用预算已用尽。"],
  ["Maximum agent nesting depth reached.", "已达到 agent 嵌套深度上限。"],
  ["Run checkpoint was not found.", "未找到运行断点。"],
  ["Run is still executing.", "运行尚未结束。"],
  ["This run must be restored from its checkpoint before executing again.", "再次执行前，需要从断点恢复该运行。"],
  ["Checkpoint reference requires storage.", "断点引用需要可用的存储。"],
  ["Checkpoint model binding is unavailable.", "断点使用的模型绑定不可用。"],
  ["Checkpoint model identity changed.", "断点对应的模型或服务地址已改变。"],
  ["Checkpoint model binding {key} changed identity.", "断点中的模型绑定 {key} 已改变。"],
  ["Model binding {key} is unavailable.", "模型绑定 {key} 不可用。"],
  ["Tool binding {id} is unavailable.", "工具绑定 {id} 不可用。"],
  ["Unsupported checkpoint version {version}.", "不支持断点版本 {version}。"],
  ["Resume result does not match the pending tool call.", "恢复结果与等待中的工具调用不匹配。"],
  ["Recovery requires an explicit matching model retry or tool result; uncertain operations are never replayed automatically.", "恢复时需要明确重试模型或提供对应工具结果；结果未知的操作不会自动重放。"],
  ["A completed run cannot be resumed.", "已完成的运行无法继续。"],
  ["Stale checkpoint revision.", "断点版本已过期。"],
  ["Agent checkpoint storage failed: {error}", "Agent 断点存储失败：{error}"],
  ["Agent run limits exceed the desktop host bounds.", "Agent 运行限制超出了桌面宿主允许的范围。"],
  ["Agent definition references an unknown model binding.", "Agent 定义引用了未知模型绑定。"],
  ["Agent definition references an unknown delegate.", "Agent 定义引用了未知的委派 agent。"],
  ["Agent definition ids must be unique.", "Agent 定义的标识必须唯一。"],
  ["Invalid agent generation parameters.", "Agent 生成参数无效。"],
  ["Invalid generation parameters.", "生成参数无效。"],
  ["Transfer to agent", "移交给 agent"],
  // Durable tasks
  ["Task runtime restarted; continue from the saved checkpoint.", "任务运行时已重启，可从保存的断点继续。"],
  ["Task worker stopped; continue from the saved checkpoint.", "后台执行器已停止，可从保存的断点继续。"],
  ["Task operation was interrupted.", "任务操作已中断。"],
  ["Task interrupted.", "任务已中断。"],
  ["Task was cancelled.", "任务已停止。"],
  ["Task lease expired or belongs to another attempt.", "任务执行权已过期，请从断点继续。"],
  ["Operation interrupted; verify its outcome before retrying.", "操作已中断，执行结果尚不确定，请核实结果后再重试。"],
  ["Task cannot be continued in its current state.", "任务当前状态无法继续。"],
  ["A task with this exclusive key is already active.", "这个任务已经在处理中。"],
  ["Another task with this exclusive key is active.", "同一会话中已有其他任务正在处理。"],
  ["Task data exceeds the 8 MiB limit.", "任务数据超过 8 MiB 上限。"],
  ["Task progress exceeds 512 KiB.", "任务进度数据超过 512 KiB 上限。"],
  ["Task input and checkpoint exceed the 12 MiB total limit.", "任务输入和断点合计超过 12 MiB 上限。"],
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
  ["Codex API requires an API key. Set it in Model providers or OPENAI_API_KEY.", "Codex API 需要 API Key，请在模型提供商设置中填写，或通过 OPENAI_API_KEY 配置。"],
  ["Codex API request failed: {error}", "Codex API 请求失败：{error}"],
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
