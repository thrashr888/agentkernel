import {
  Activity,
  Square,
  Box,
  Cpu,
  MemoryStick,
  Blocks,
  Timer,
  Database,
} from "lucide-react";
import { Link } from "react-router-dom";
import { useObjects } from "@/lib/hooks/use-objects";
import { useSchedules } from "@/lib/hooks/use-schedules";
import { useStores } from "@/lib/hooks/use-stores";
import type { SandboxInfo } from "@/lib/types";

interface StatusCardsProps {
  sandboxes: SandboxInfo[];
}

function Metric({
  icon: Icon,
  iconClass,
  label,
  value,
  sub,
  to,
}: {
  icon: React.ComponentType<{ className?: string }>;
  iconClass: string;
  label: string;
  value: string | number;
  sub?: string;
  to?: string;
}) {
  const content = (
    <div className="flex items-center gap-2 min-w-0">
      <Icon className={`h-3.5 w-3.5 shrink-0 ${iconClass}`} />
      <span className="font-mono font-semibold text-sm">{value}</span>
      <span className="text-xs text-muted-foreground truncate">
        {label}
        {sub ? ` (${sub})` : ""}
      </span>
    </div>
  );
  if (to) {
    return (
      <Link to={to} className="hover:text-foreground transition-colors">
        {content}
      </Link>
    );
  }
  return content;
}

export function StatusCards({ sandboxes }: StatusCardsProps) {
  const { data: objects } = useObjects();
  const { data: schedules } = useSchedules();
  const { data: stores } = useStores();

  const runningSandboxes = sandboxes.filter(
    (s) => s.status.toLowerCase() === "running",
  );
  const running = runningSandboxes.length;
  const stopped = sandboxes.filter(
    (s) => s.status.toLowerCase() === "stopped",
  ).length;
  const total = sandboxes.length;
  const totalVcpus = runningSandboxes.reduce(
    (sum, s) => sum + (s.vcpus ?? 0),
    0,
  );
  const totalMemoryMb = runningSandboxes.reduce(
    (sum, s) => sum + (s.memory_mb ?? 0),
    0,
  );

  const objectCount = objects?.length ?? 0;
  const activeObjects =
    objects?.filter((o) => o.status === "active").length ?? 0;
  const scheduleCount = schedules?.length ?? 0;
  const activeSchedules =
    schedules?.filter((s) => s.status === "active").length ?? 0;
  const storeCount = stores?.length ?? 0;

  const memoryLabel =
    totalMemoryMb >= 1024
      ? `${(totalMemoryMb / 1024).toFixed(1)} GB`
      : `${totalMemoryMb} MB`;

  return (
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-md border px-4 py-2.5 text-sm">
      <Metric
        icon={Activity}
        iconClass="text-green-500"
        label="running"
        value={running}
      />
      <Metric
        icon={Square}
        iconClass="text-muted-foreground"
        label="stopped"
        value={stopped}
      />
      <Metric
        icon={Box}
        iconClass="text-foreground"
        label="total"
        value={total}
      />

      <span className="hidden sm:block text-border">|</span>

      <Metric
        icon={Cpu}
        iconClass="text-blue-500"
        label="vCPUs"
        value={totalVcpus}
      />
      <Metric
        icon={MemoryStick}
        iconClass="text-purple-500"
        label="memory"
        value={memoryLabel}
      />

      <span className="hidden sm:block text-border">|</span>

      <Metric
        icon={Blocks}
        iconClass="text-orange-500"
        label="objects"
        value={objectCount}
        sub={activeObjects > 0 ? `${activeObjects} active` : undefined}
        to="/objects"
      />
      <Metric
        icon={Timer}
        iconClass="text-teal-500"
        label="schedules"
        value={scheduleCount}
        sub={activeSchedules > 0 ? `${activeSchedules} active` : undefined}
        to="/schedules"
      />
      <Metric
        icon={Database}
        iconClass="text-indigo-500"
        label="stores"
        value={storeCount}
        to="/stores"
      />
    </div>
  );
}
