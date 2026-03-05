import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Play, Loader2, Zap } from "lucide-react";
import { api } from "@/lib/api";
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

const BENCHMARK_KEY = "agentkernel_last_benchmark";

function loadLastResult(): BenchmarkResult | null {
  try {
    const raw = localStorage.getItem(BENCHMARK_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
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
          {value.toFixed(0)}
          <span className="text-lg font-normal text-muted-foreground">{unit}</span>
        </p>
        {diff !== undefined && (
          <p className={`text-xs ${diff < 0 ? "text-green-600 dark:text-green-400" : diff > 0 ? "text-red-600 dark:text-red-400" : "text-muted-foreground"}`}>
            {diff > 0 ? "+" : ""}{diff.toFixed(0)}{unit} vs last run
          </p>
        )}
      </CardContent>
    </Card>
  );
}

export function Benchmark() {
  const [result, setResult] = useState<BenchmarkResult | null>(null);
  const [lastResult] = useState<BenchmarkResult | null>(loadLastResult);

  const benchmarkMutation = useMutation({
    mutationFn: () => api.runBenchmark(),
    onMutate: () => {
      return { toastId: toast("Running benchmark...") };
    },
    onSuccess: (data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Benchmark complete!", "success");
      setResult(data);
      localStorage.setItem(BENCHMARK_KEY, JSON.stringify(data));
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const current = result ?? lastResult;

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
          onClick={() => benchmarkMutation.mutate()}
          disabled={benchmarkMutation.isPending}
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
              previous={result && lastResult ? lastResult.create_ms : undefined}
            />
            <MetricCard
              label="Execute"
              value={current.exec_ms}
              unit="ms"
              previous={result && lastResult ? lastResult.exec_ms : undefined}
            />
            <MetricCard
              label="Destroy"
              value={current.destroy_ms}
              unit="ms"
              previous={result && lastResult ? lastResult.destroy_ms : undefined}
            />
            <MetricCard
              label="Total"
              value={current.total_ms}
              unit="ms"
              previous={result && lastResult ? lastResult.total_ms : undefined}
            />
          </div>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Zap className="h-5 w-5" />
                Details
              </CardTitle>
              <CardDescription>
                Last benchmark run details
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-2">
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Image</span>
                  <span className="font-mono text-sm text-muted-foreground">{current.image}</span>
                </div>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Timestamp</span>
                  <span className="text-sm text-muted-foreground">
                    {new Date(current.timestamp).toLocaleString()}
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
