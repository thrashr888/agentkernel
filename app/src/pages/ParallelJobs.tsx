import { FormEvent, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { CheckCircle2, GitFork, Loader2, Play, XCircle } from "lucide-react";
import { api } from "@/lib/api";
import type { BatchResult } from "@/lib/types";
import { toast } from "@/components/ui/use-toast";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const MAX_SANDBOXES = 20;

function resultStatus(result: BatchResult): "success" | "error" {
  return result.error ? "error" : "success";
}

export function ParallelJobs() {
  const [sandboxCount, setSandboxCount] = useState("3");
  const [command, setCommand] = useState("echo hello from parallel sandbox");
  const [results, setResults] = useState<BatchResult[] | null>(null);

  const count = useMemo(() => {
    const parsed = Number.parseInt(sandboxCount, 10);
    if (!Number.isFinite(parsed)) return 0;
    return Math.min(MAX_SANDBOXES, Math.max(1, parsed));
  }, [sandboxCount]);

  const runMutation = useMutation({
    mutationFn: () =>
      api.batchRun(
        Array.from({ length: count }, () => ({
          // The HTTP batch endpoint runs each command in a fresh ephemeral sandbox.
          command: ["sh", "-c", command.trim()],
        })),
      ),
    onMutate: () => {
      setResults(null);
      return { toastId: toast(`Running in ${count} sandboxes...`) };
    },
    onSuccess: (data, _variables, context) => {
      setResults(data.results);
      if (context?.toastId) toast.update(context.toastId, "Parallel run complete", "success");
    },
    onError: (error, _variables, context) => {
      if (context?.toastId) {
        toast.update(
          context.toastId,
          error instanceof Error ? error.message : String(error),
          "error",
        );
      }
    },
  });

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!command.trim() || count < 1) return;
    runMutation.mutate();
  }

  const passed = results?.filter((result) => resultStatus(result) === "success").length ?? 0;
  const failed = results?.length ? results.length - passed : 0;

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Parallel Jobs</h1>
          <p className="text-muted-foreground">
            Fan out one command across multiple ephemeral sandboxes and compare the results.
          </p>
        </div>
        <GitFork className="mt-1 h-7 w-7 text-muted-foreground" />
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Start a parallel run</CardTitle>
          <CardDescription>
            Each job gets a fresh sandbox. Runs are submitted together and execute concurrently.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-5">
            <div className="grid gap-5 sm:grid-cols-[minmax(0,1fr)_180px]">
              <div className="space-y-2">
                <Label htmlFor="parallel-command">Command</Label>
                <Input
                  id="parallel-command"
                  value={command}
                  onChange={(event) => setCommand(event.target.value)}
                  placeholder="pytest -q"
                  className="font-mono"
                  disabled={runMutation.isPending}
                />
                <p className="text-xs text-muted-foreground">
                  Enter a shell command to run in every sandbox.
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="parallel-count">Sandbox jobs</Label>
                <Input
                  id="parallel-count"
                  type="number"
                  min={1}
                  max={MAX_SANDBOXES}
                  value={sandboxCount}
                  onChange={(event) => setSandboxCount(event.target.value)}
                  disabled={runMutation.isPending}
                />
                <p className="text-xs text-muted-foreground">1–{MAX_SANDBOXES} jobs per run.</p>
              </div>
            </div>
            <Button type="submit" disabled={runMutation.isPending || !command.trim() || count < 1}>
              {runMutation.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Running...
                </>
              ) : (
                <>
                  <Play className="mr-2 h-4 w-4" />
                  Run in {count || "—"} sandboxes
                </>
              )}
            </Button>
          </form>
        </CardContent>
      </Card>

      {results && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between gap-3">
              <div>
                <CardTitle>Run results</CardTitle>
                <CardDescription className="mt-1">
                  {results.length} sandbox jobs completed.
                </CardDescription>
              </div>
              <div className="flex gap-2">
                <Badge variant="success">{passed} passed</Badge>
                {failed > 0 && <Badge variant="destructive">{failed} failed</Badge>}
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid gap-3 md:grid-cols-2">
              {results.map((result, index) => {
                const succeeded = resultStatus(result) === "success";
                return (
                  <div key={index} className="rounded-md border bg-muted/20 p-4">
                    <div className="mb-3 flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        {succeeded ? (
                          <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400" />
                        ) : (
                          <XCircle className="h-4 w-4 text-destructive" />
                        )}
                        <span className="font-medium">Sandbox {index + 1}</span>
                      </div>
                      <Badge variant={succeeded ? "success" : "destructive"}>
                        {succeeded ? "passed" : "failed"}
                      </Badge>
                    </div>
                    <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded bg-background p-3 text-xs leading-relaxed">
                      {result.output ?? result.error ?? "No output."}
                    </pre>
                  </div>
                );
              })}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
