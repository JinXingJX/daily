import * as React from "react";
import { cn } from "@/lib/utils";

const Textarea = React.forwardRef<HTMLTextAreaElement, React.ComponentProps<"textarea">>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        ref={ref}
        className={cn(
          "flex min-h-28 w-full rounded-[var(--radius-control)] border border-[var(--line)] bg-[var(--surface-strong)] px-3 py-2.5 text-sm leading-5 text-[var(--text-strong)] shadow-none transition-[background-color,border-color,box-shadow,color] duration-150 ease-out placeholder:text-[var(--text-muted)] hover:border-[var(--line-strong)] focus-visible:border-[var(--line-strong)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)] disabled:cursor-not-allowed disabled:bg-[var(--surface-inset)] disabled:text-[var(--text-disabled)]",
          className,
        )}
        {...props}
      />
    );
  },
);
Textarea.displayName = "Textarea";

export { Textarea };
