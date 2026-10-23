import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-[var(--radius-control)] border px-2 py-0.5 text-xs font-medium leading-4",
  {
    variants: {
      variant: {
        default: "border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]",
        secondary: "border-[var(--line)] bg-[var(--surface)] text-[var(--text-soft)]",
        success: "border-[var(--success-line)] bg-[var(--success-soft)] text-[var(--success)]",
        warning: "border-[var(--warning-line)] bg-[var(--warning-soft)] text-[var(--warning)]",
        destructive: "border-[var(--danger-line)] bg-[var(--danger-soft)] text-[var(--danger)]",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps extends React.HTMLAttributes<HTMLDivElement>, VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
