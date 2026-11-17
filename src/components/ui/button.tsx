import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-[var(--radius-control)] border text-sm font-medium leading-5 transition-[background-color,border-color,color,box-shadow] duration-150 ease-out disabled:pointer-events-none disabled:cursor-not-allowed disabled:border-[var(--line)] disabled:bg-[var(--surface-inset)] disabled:text-[var(--text-disabled)] focus-visible:outline-none focus-visible:shadow-[var(--focus-ring)] [&_svg]:pointer-events-none [&_svg]:size-4",
  {
    variants: {
      variant: {
        default:
          "border-[var(--primary)] bg-[var(--primary)] text-[var(--primary-contrast)] hover:border-[var(--primary-hover)] hover:bg-[var(--primary-hover)]",
        secondary:
          "border-[var(--line-strong)] bg-[var(--surface-strong)] text-[var(--text-strong)] hover:border-[var(--line-active)] hover:bg-[var(--surface)]",
        ghost:
          "border-transparent bg-transparent text-[var(--text-strong)] hover:bg-[var(--fill-hover)]",
        outline:
          "border-[var(--line)] bg-transparent text-[var(--text-strong)] hover:border-[var(--line-strong)] hover:bg-[var(--fill-hover)]",
        destructive:
          "border-[var(--danger-fill)] bg-[var(--danger-fill)] text-[var(--danger-contrast)] hover:border-[var(--danger)] hover:bg-[var(--danger)]",
      },
      size: {
        default: "h-9 px-3.5 py-0",
        sm: "h-8 px-3 py-0 text-[13px]",
        lg: "h-11 px-4 py-0 text-base",
        icon: "h-9 w-9 px-0",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />;
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
