import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

type PageHeaderProps = {
  eyebrow: string;
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
};

export function PageHeader({ eyebrow, title, description, actions, className }: PageHeaderProps) {
  return (
    <header className={cn("flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between", className)}>
      <div className="min-w-0">
        <Kicker>{eyebrow}</Kicker>
        <h1 className="mt-2 text-2xl font-semibold text-[var(--text-strong)]">{title}</h1>
        {description ? (
          <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--text-muted)]">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
    </header>
  );
}

export function Kicker({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("text-xs font-medium leading-4 text-[var(--text-muted)]", className)}>
      {children}
    </div>
  );
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="space-y-2">
      <Kicker>{label}</Kicker>
      {children}
    </label>
  );
}

export function SectionBlock({
  eyebrow,
  title,
  description,
  children,
  className,
}: {
  eyebrow: string;
  title: string;
  description: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("space-y-4 border-t border-[var(--line)] pt-5", className)}>
      <div>
        <Kicker>{eyebrow}</Kicker>
        <div className="mt-2 text-lg font-semibold text-[var(--text-strong)]">{title}</div>
        <div className="mt-2 max-w-3xl text-sm leading-6 text-[var(--text-muted)]">{description}</div>
      </div>
      {children}
    </section>
  );
}

export function EmptyPanel({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "grid min-h-64 place-items-center border-y border-[var(--line)] bg-transparent px-6 text-center text-sm leading-6 text-[var(--text-muted)]",
        className,
      )}
    >
      {children}
    </div>
  );
}
