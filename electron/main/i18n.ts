import { app } from "electron";

// Menus and native dialogs follow the system language. Keys are the English text; `{name}`
// placeholders are filled from `vars`. The renderer has its own catalog in src/i18n.
const zhCN: Record<string, string> = {
  File: "文件",
  "New Workspace Window": "新建工作区窗口",
  "Open Workspace…": "打开工作区…",
  "Recent Workspaces": "最近的工作区",
  "No Recent Workspaces": "没有最近的工作区",
  "Open Daily Workspace": "打开 Daily 工作区",
  "Open Workspace": "打开工作区",
  Cancel: "取消",
  "Quit Daily": "退出 Daily",
  "Unsaved plugin changes": "插件有未保存的修改",
  "Tasks are still running": "仍有任务在运行",
  "A task is still running": "有任务正在运行",
  "Workspaces with unsaved plugin changes: {count}.": "{count} 个工作区有未保存的插件修改。",
  "Workspaces with an active task: {count}.": "{count} 个工作区有正在运行的任务。",
  "Workspaces that also have active tasks: {count}. Quitting discards the drafts and stops the local runtimes.":
    "另有 {count} 个工作区有正在运行的任务。退出会丢弃草稿并停止本地运行时。",
  "Quitting discards those drafts.": "退出会丢弃这些草稿。",
  "Quitting stops the active tasks and closes their local runtimes.": "退出会停止正在运行的任务并关闭本地运行时。",
  "Keep Open": "保持打开",
  "Discard and Close": "放弃并关闭",
  "Stop and Close": "停止并关闭",
  "Close this workspace and discard its unsaved plugin changes?": "关闭此工作区并放弃未保存的插件修改？",
  "Close this workspace and stop its active task?": "关闭此工作区并停止正在运行的任务？",
  "The active task will also be stopped.": "正在运行的任务也会被停止。",
  "Could not close workspace": "无法关闭工作区",
  "Could not open link": "无法打开链接",
  "Daily action failed": "操作失败",
  "Daily could not start": "Daily 无法启动",
  "Could not open workspace": "无法打开工作区",
};

function useChinese(): boolean {
  try {
    return app.getLocale().toLowerCase().startsWith("zh");
  } catch {
    return false;
  }
}

export function t(key: string, vars?: Record<string, string | number>): string {
  const template = useChinese() ? zhCN[key] ?? key : key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}
