import { type ReactNode, useMemo, useState } from "react";
import { motion } from "motion/react";
import { AlertTriangle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function PluginShell(props: {
  identifier: string;
  label: string;
  icon: ReactNode;
  partial?: boolean;
  source?: string;
  children: ReactNode;
}) {
  return (
    <motion.section
      layout
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className="workbench-panel overflow-hidden"
    >
      <div className="flex items-center justify-between gap-4 px-5 py-4">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-[var(--radius-control)] bg-[var(--surface-muted)] text-[var(--text-soft)]">
            {props.icon}
          </div>
          <div className="space-y-1">
            <div className="font-medium text-[var(--text-strong)]">{props.label}</div>
            <div className="text-xs font-medium text-[var(--text-muted)]">{props.identifier}</div>
          </div>
        </div>
        <Badge variant={props.partial ? "success" : "secondary"}>
          {props.partial ? "Streaming" : "Interactive"}
        </Badge>
      </div>
      <div className="space-y-4 p-5">{props.children}</div>
      {props.partial ? (
        <pre className="overflow-x-auto bg-[var(--surface-muted)] px-5 py-4 text-xs leading-6 text-[var(--text-soft)]">
          {props.source?.trim() || "{ … }"}
        </pre>
      ) : null}
    </motion.section>
  );
}

export function PluginFallback(props: { identifier: string; source: string; reason: string }) {
  const [attempt, setAttempt] = useState(0);
  const result = useMemo(() => {
    try {
      return JSON.parse(props.source);
    } catch {
      return null;
    }
  }, [props.source, attempt]);

  return (
    <motion.section
      layout
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className="workbench-panel overflow-hidden"
    >
      <div className="flex items-center justify-between gap-4 px-5 py-4">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-[var(--radius-control)] bg-[var(--surface-muted)] text-[var(--danger)]">
            <AlertTriangle className="size-4" />
          </div>
          <div className="space-y-1">
            <div className="font-medium text-[var(--text-strong)]">Plugin Parse Error</div>
            <div className="text-xs font-medium text-[var(--text-muted)]">{props.identifier}</div>
          </div>
        </div>
        <Button size="sm" variant="secondary" onClick={() => setAttempt((value) => value + 1)}>
          Retry
        </Button>
      </div>
      <div className="space-y-3 px-5 py-4 text-sm text-[var(--text)]">
        <p>{props.reason}</p>
        <div className="text-[var(--text-muted)]">
          {result
            ? "JSON is valid, but the payload shape does not match the registered schema."
            : "Raw payload is shown below so the conversation remains intact."}
        </div>
      </div>
      <pre className="overflow-x-auto bg-[var(--surface-muted)] px-5 py-4 text-xs leading-6 text-[var(--text-soft)]">
        {props.source}
      </pre>
    </motion.section>
  );
}
