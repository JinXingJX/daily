import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

type IconButtonProps = {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
  className?: string;
};

export function IconButton({ label, disabled, onClick, children, className }: IconButtonProps) {
  return (
    <button
      className={cn(
        "inline-flex size-8 items-center justify-center rounded-[var(--radius-control)] border border-transparent bg-transparent text-[var(--text-muted)] transition-[background-color,border-color,box-shadow,color] duration-150 ease-out hover:border-[var(--line)] hover:bg-[var(--geist-gray-alpha-100)] hover:text-[var(--text-strong)] disabled:cursor-not-allowed disabled:bg-transparent disabled:text-[var(--text-disabled)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)]",
        className,
      )}
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}
