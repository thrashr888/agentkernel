import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Play, Loader2, Zap } from "lucide-react";
import { api } from "@/lib/api";
import { useSettings } from "@/lib/hooks/use-settings";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { BenchmarkResult } from "@/lib/types";
import {
  appendBenchmarkHistory,
  benchmarkServerKey,
  loadBenchmarkHistory,
  normalizeBenchmarkResult,
} from "@/lib/benchmark-history";

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime())
    ? timestamp
    : date.toISOString().replace("T", " ").replace("Z", " UTC");
}

function formatMs(ms: number): string {
  if (ms >= 100) return ms.toFixed(0);
  if (ms >= 10) return ms.toFixed(1);
  if (ms >= 1) return ms.toFixed(2);
  return ms.toFixed(3);
}

function MetricCard({ label, value, unit, previous }: {
  label: string;
  value: number;
  unit: string;
  previous?: number;
}) {
  const diff = previous !== undefined ? value - previous : undefined;
  return (
    <Card>
      <CardContent className="pt-6">
        <p className="text-sm text-muted-foreground">{label}</p>
        <p className="text-3xl font-bold tabular-nums">
          {formatMs(value)}
          <span className="text-lg font-normal text-muted-foreground">{unit}</span>
        </p>
        {diff !== undefined && (
          <p className={`text-xs ${diff < 0 ? "text-green-600 dark:text-green-400" : diff > 0 ? "text-red-600 dark:text-red-400" : "text-muted-foreground"}`}>
            {diff > 0 ? "+" : ""}{formatMs(diff)}{unit} vs previous matching run
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function BenchmarkTable({ history }: { history: BenchmarkResult[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Run history</CardTitle>
        <CardDescription>Newest runs for the active server</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="border-b text-xs text-muted-foreground">
              <tr>
                <th className="pb-3 pr-4 font-medium">Timestamp</th>
                <th className="pb-3 pr-4 font-medium">Backend</th>
                <th className="pb-3 pr-4 font-medium">Image</th>
                <th className="pb-3 pr-4 text-right font-medium">Create</th>
                <th className="pb-3 pr-4 text-right font-medium">Execute</th>
                <th className="pb-3 pr-4 text-right font-medium">Destroy</th>
                <th className="pb-3 text-right font-medium">Total</th>
              </tr>
            </thead>
            <tbody>
              {history.map((entry, index) => (
                <tr key={`${entry.timestamp}-${index}`} className="border-b last:border-0">
                  <td className="whitespace-nowrap py-3 pr-4 text-muted-foreground">
                    {formatTimestamp(entry.timestamp)}
                  </td>
                  <td className="py-3 pr-4 font-mono">{entry.backend}</td>
                  <td className="max-w-48 truncate py-3 pr-4 font-mono" title={entry.image}>
                    {entry.image}
                  </td>
                  <td className="py-3 pr-4 text-right tabular-nums">{formatMs(entry.create_ms)} ms</td>
                  <td className="py-3 pr-4 text-right tabular-nums">{formatMs(entry.exec_ms)} ms</td>
                  <td className="py-3 pr-4 text-right tabular-nums">{formatMs(entry.destroy_ms)} ms</td>
                  <td className="py-3 text-right font-medium tabular-nums">{formatMs(entry.total_ms)} ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

export function Benchmark() {
  const { settings } = useSettings();
  const [history, setHistory] = useState<BenchmarkResult[]>([]);

  const activeServer = useMemo(() => {
    if (!settings) return null;
    const name = settings.active_server ?? settings.servers[0]?.name ?? "Local";
    const entry = settings.servers.find((server) => server.name === name);
    return {
      name,
      url: entry?.url ?? settings.api_url ?? "http://localhost:18888",
    };
  }, [settings]);

  const serverKey = useMemo(
    () => activeServer && benchmarkServerKey(activeServer.name, activeServer.url),
    [activeServer],
  );

  useEffect(() => {
    if (serverKey) setHistory(loadBenchmarkHistory(serverKey));
    else setHistory([]);
  }, [serverKey]);

  const benchmarkMutation = useMutation({
    mutationFn: (_targetServerKey: string) => api.runBenchmark(),
    onMutate: () => {
      return { toastId: toast("Running benchmark...") };
    },
    onSuccess: (data, targetServerKey, context) => {
      if (context?.toastId) toast.update(context.toastId, "Benchmark complete!", "success");
      const normalized = normalizeBenchmarkResult(data);
      if (!normalized) return;
      const updatedHistory = appendBenchmarkHistory(targetServerKey, normalized);
      if (targetServerKey === serverKey) setHistory(updatedHistory);
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const current = history[0];
  const previousMatching = current
    ? history.slice(1).find((entry) => entry.backend === current.backend && entry.image === current.image)
    : undefined;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Benchmark</h1>
          <p className="text-muted-foreground">
            Measure sandbox creation, execution, and teardown performance
          </p>
        </div>
        <Button
          onClick={() => serverKey && benchmarkMutation.mutate(serverKey)}
          disabled={benchmarkMutation.isPending || !serverKey}
        >
          {benchmarkMutation.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Running...
            </>
          ) : (
            <>
              <Play className="mr-2 h-4 w-4" />
              Run Benchmark
            </>
          )}
        </Button>
      </div>

      {current ? (
        <>
          <div className="grid gap-4 md:grid-cols-4">
            <MetricCard
              label="Create"
              value={current.create_ms}
              unit="ms"
              previous={previousMatching?.create_ms}
            />
            <MetricCard
              label="Execute"
              value={current.exec_ms}
              unit="ms"
              previous={previousMatching?.exec_ms}
            />
            <MetricCard
              label="Destroy"
              value={current.destroy_ms}
              unit="ms"
              previous={previousMatching?.destroy_ms}
            />
            <MetricCard
              label="Total"
              value={current.total_ms}
              unit="ms"
              previous={previousMatching?.total_ms}
            />
          </div>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Zap className="h-5 w-5" />
                Latest run
              </CardTitle>
              <CardDescription>
                {current.backend} · {current.image}
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-2">
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Backend</span>
                  <span className="font-mono text-sm text-muted-foreground">{current.backend}</span>
                </div>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Image</span>
                  <span className="font-mono text-sm text-muted-foreground">{current.image}</span>
                </div>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Timestamp</span>
                  <span className="font-mono text-sm text-muted-foreground">
                    {formatTimestamp(current.timestamp)}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium">Performance</span>
                  <span className="text-sm text-muted-foreground">
                    {current.total_ms < 5000 ? "Excellent" : current.total_ms < 10000 ? "Good" : "Needs improvement"}
                  </span>
                </div>
              </div>
            </CardContent>
          </Card>
          <BenchmarkTable history={history} />
        </>
      ) : (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <Zap className="h-12 w-12 text-muted-foreground/30 mb-4" />
            <p className="text-muted-foreground">
              Run a benchmark to measure your sandbox performance.
            </p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
