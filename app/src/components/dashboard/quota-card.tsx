import { Gauge } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { QuotaScopeStatus } from "@/lib/types";

function limit(value: number | undefined): string {
  return value === undefined ? "unlimited" : value.toLocaleString();
}

function Scope({ scope }: { scope: QuotaScopeStatus }) {
  const rows = [
    ["Running", scope.usage.running_sandboxes, scope.limits.max_running_sandboxes],
    ["Sandboxes", scope.usage.total_sandboxes, scope.limits.max_total_sandboxes],
    ["vCPUs", scope.usage.total_vcpus, scope.limits.max_total_vcpus],
    ["Memory", `${(scope.usage.total_memory_mb / 1024).toFixed(1)} GB`, scope.limits.max_total_memory_mb === undefined ? undefined : `${(scope.limits.max_total_memory_mb / 1024).toFixed(1)} GB`],
  ] as const;

  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-muted-foreground truncate">{scope.id}</p>
      {rows.map(([label, usage, configuredLimit]) => (
        <div key={label} className="flex items-center justify-between gap-3 text-xs">
          <span className="text-muted-foreground">{label}</span>
          <span className="font-mono">
            {usage} / {typeof configuredLimit === "number" ? limit(configuredLimit) : configuredLimit ?? "unlimited"}
          </span>
        </div>
      ))}
    </div>
  );
}

export function QuotaCard() {
  const { data, error, isLoading } = useQuery({
    queryKey: ["quotas"],
    queryFn: api.getQuotas,
    retry: false,
    staleTime: 15_000,
  });

  return (
    <div className="rounded-md border px-4 py-3">
      <div className="flex items-center gap-2 mb-3">
        <Gauge className="h-4 w-4 text-amber-500" />
        <span className="text-sm font-medium">Resource quotas</span>
      </div>
      {isLoading ? (
        <p className="text-xs text-muted-foreground">Loading quota usage…</p>
      ) : error ? (
        <p className="text-xs text-muted-foreground">Quota reporting is unavailable on this server.</p>
      ) : !data?.enabled ? (
        <p className="text-xs text-muted-foreground">No enterprise quota policy is configured.</p>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2">
          <Scope scope={data.user} />
          <Scope scope={data.organization} />
        </div>
      )}
    </div>
  );
}
