import * as React from "react";
import { cn } from "@/lib/utils";

const Input = React.forwardRef<HTMLInputElement, React.ComponentProps<"input">>(({ className, ...props }, ref) => {
  return (
    <input
      ref={ref}
      className={cn(
        "flex h-9 w-full rounded-[var(--radius-control)] border border-[var(--line-strong)] bg-[var(--surface-strong)] px-3 py-0 text-sm leading-5 text-[var(--text-strong)] shadow-none transition-[background-color,border-color,box-shadow,color] duration-150 ease-out file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-[var(--text-muted)] hover:border-[var(--line-strong)] focus-visible:border-[var(--line-strong)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)] disabled:cursor-not-allowed disabled:bg-[var(--surface-inset)] disabled:text-[var(--text-disabled)]",
        className,
      )}
      {...props}
    />
  );
});
Input.displayName = "Input";

export { Input };
