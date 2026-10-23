import { cn } from "@/lib/utils";

function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("animate-pulse rounded-[var(--radius-control)] bg-[var(--geist-gray-alpha-200)]", className)} {...props} />;
}

export { Skeleton };
