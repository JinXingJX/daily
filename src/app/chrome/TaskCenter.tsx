import { useCallback, useEffect, useState } from "react";
import { ListTodo, LoaderCircle } from "lucide-react";
import type { TaskSnapshot } from "@daily/plugin-sdk";
import { localizeHostText } from "@daily/plugin-sdk/hostText";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { invoke } from "@/platform/desktop";
import { errorMessage } from "@/lib/errors";
import { currentLocale, t } from "@/i18n";
import { useAppStore } from "@/useAppStore";

const labels: Record<TaskSnapshot["status"], Parameters<typeof t>[0]> = {
  queued: "Queued", running: "Running", cancelling: "Stopping…", waiting_input: "Waiting for your reply",
  completed: "Completed", failed: "Failed", cancelled: "Stopped", interrupted: "Interrupted",
};

/** Task ownership lives in the sidecar; this component can be remounted freely. */
export function TaskCenter({ ready }: { ready: boolean }) {
  const plugins = useAppStore((state) => state.plugins?.installed);
  const [open, setOpen] = useState(false);
  const [tasks, setTasks] = useState<TaskSnapshot[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<TaskSnapshot | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    const result = await invoke<{ tasks: TaskSnapshot[] }>("list_workspace_tasks");
    setTasks(result.tasks); setError(null);
  }, []);
  useEffect(() => {
    if (!ready) return;
    let disposed = false;
    let loading = false;
    const load = async () => {
      if (loading) return;
      loading = true;
      try { const result = await invoke<{ tasks: TaskSnapshot[] }>("list_workspace_tasks"); if (!disposed) { setTasks(result.tasks); setError(null); } }
      catch (error) { if (!disposed) setError(errorMessage(error)); }
      finally { loading = false; }
    };
    void load();
    const interval = setInterval(() => void load(), open ? 1_000 : 3_000);
    return () => { disposed = true; clearInterval(interval); };
  }, [open, ready]);
  const active = tasks.filter((task) => ["queued", "running", "cancelling"].includes(task.status)).length;
  const act = async (task: TaskSnapshot, action: "get" | "cancel" | "retry") => {
    setPending(task.id);
    try {
      const next = await invoke<TaskSnapshot>("workspace_task", { input: { taskId: task.id, action } });
      if (action === "get") setDetail(next);
      await refresh();
    } catch (error) { setError(errorMessage(error)); }
    finally { setPending(null); }
  };
  return <Dialog open={open} onOpenChange={setOpen}>
    <DialogTrigger asChild><button type="button" className="status-bar-item rounded px-1 hover:bg-[var(--surface-inset)]" aria-label={t("Task center")}>
      {active ? <LoaderCircle className="size-3 animate-spin" /> : <ListTodo className="size-3" />}{t("Tasks")}{active ? ` · ${active}` : ""}
    </button></DialogTrigger>
    <DialogContent className="max-h-[76vh] overflow-y-auto">
      <DialogHeader><DialogTitle>{t("Task center")}</DialogTitle><DialogDescription>{t("Tasks continue when you switch pages. Results and checkpoints are saved.")}</DialogDescription></DialogHeader>
      {error ? <p role="alert" className="mt-4 text-sm text-[var(--danger)]">{localizeHostText(error, currentLocale())}</p> : null}
      {!tasks.length ? <p className="py-10 text-sm text-[var(--text-muted)]">{t("No background tasks yet.")}</p> : null}
      <div className="mt-5 divide-y divide-[var(--line)]">{tasks.map((task) => <div key={task.id} className="flex items-start gap-3 py-4">
        <div className="min-w-0 flex-1"><div className="truncate text-sm font-medium">{task.title || task.kind}</div><div className="mt-1 text-xs text-[var(--text-muted)]">{t(labels[task.status])} · {plugins?.find((plugin) => plugin.identifier === task.pluginId)?.name ?? task.pluginId}</div>{task.error ? <p className="mt-2 break-words text-xs text-[var(--danger)]">{localizeHostText(task.error, currentLocale())}</p> : null}</div>
        <div className="flex shrink-0 gap-2">
          {task.status === "completed" ? <Button size="sm" variant="outline" disabled={pending === task.id} onClick={() => void act(task, "get")}>{t("View result")}</Button> : null}
          {["failed", "interrupted", "cancelled"].includes(task.status) ? <Button size="sm" variant="outline" disabled={pending === task.id} onClick={() => void act(task, "retry")}>{t("Continue")}</Button> : null}
          {["queued", "running", "waiting_input"].includes(task.status) ? <Button size="sm" variant="outline" disabled={pending === task.id} onClick={() => void act(task, "cancel")}>{t("Stop")}</Button> : null}
        </div>
      </div>)}</div>
      {detail ? <div className="mt-5 rounded border border-[var(--line)] bg-[var(--surface-inset)] p-4"><p className="mb-3 text-sm font-medium">{detail.title || t("Result")}</p><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs leading-6">{resultText(detail.result)}</pre></div> : null}
    </DialogContent>
  </Dialog>;
}

function resultText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(resultText).filter(Boolean).join("\n\n");
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const text = [record.reply, record.answer, record.title, record.summary, record.content, record.text, record.caption, record.detail, record.reason, record.problem, record.suggestion].filter((value): value is string => typeof value === "string").join("\n\n");
    const children = [record.comments, record.verdicts, record.voices, record.digest].filter((value) => value !== undefined).map(resultText).join("\n\n");
    return [text, children].filter(Boolean).join("\n\n") || t("The result has been saved in the plugin.");
  }
  return t("The result has been saved in the plugin.");
}
