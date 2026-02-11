import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

interface SandboxStatusBadgeProps {
  status: string;
  className?: string;
}

export function SandboxStatusBadge({
  status,
  className,
}: SandboxStatusBadgeProps) {
  const normalized = status.toLowerCase();

  if (normalized === "running") {
    return (
      <Badge variant="success" className={cn("gap-1", className)}>
        <span className="h-1.5 w-1.5 rounded-full bg-green-600 dark:bg-green-400" />
        Running
      </Badge>
    );
  }

  if (normalized === "stopped") {
    return (
      <Badge variant="secondary" className={className}>
        Stopped
      </Badge>
    );
  }

  return (
    <Badge variant="outline" className={className}>
      {status}
    </Badge>
  );
}
