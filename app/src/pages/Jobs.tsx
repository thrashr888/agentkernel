import { useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Search, ChevronRight, ChevronDown, Square } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { DetachedCommand } from "@/lib/types";

function statusVariant(status: string) {
  if (status === "running") return "success" as const;
  if (status === "completed") return "secondary" as const;
  return "destructive" as const;
}

function JobRow({ job }: { job: DetachedCommand & { sandboxName: string } }) {
  const [expanded, setExpanded] = useState(false);
  const queryClient = useQueryClient();

  const { data: logs } = useQuery({
    queryKey: ["job-logs", job.sandboxName, job.id],
    queryFn: () => api.getDetachedLogs(job.sandboxName, job.id),
    enabled: expanded,
    refetchInterval: job.status === "running" ? 2000 : false,
  });

  const killMutation = useMutation({
    mutationFn: () => api.killDetached(job.sandboxName, job.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["all-jobs"] });
      toast.success("Job killed");
    },
    onError: (err) => toast.error(err instanceof Error ? err.message : String(err)),
  });

  return (
    <>
      <TableRow className="cursor-pointer" onClick={() => setExpanded(!expanded)}>
        <TableCell className="w-8 px-2">
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </TableCell>
        <TableCell className="font-mono text-xs">{job.sandboxName}</TableCell>
        <TableCell className="font-mono text-xs max-w-[300px] truncate">
          {job.command.join(" ")}
        </TableCell>
        <TableCell>
          <Badge variant={statusVariant(job.status)}>{job.status}</Badge>
        </TableCell>
        <TableCell className="font-mono text-xs whitespace-nowrap">
          {job.started_at ? new Date(job.started_at).toLocaleTimeString() : "-"}
        </TableCell>
        <TableCell>
          {job.status === "running" && (
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={(e) => {
                e.stopPropagation();
                killMutation.mutate();
              }}
              disabled={killMutation.isPending}
              title="Kill job"
            >
              <Square className="h-3.5 w-3.5 text-destructive" />
            </Button>
          )}
        </TableCell>
      </TableRow>
      {expanded && (
        <TableRow>
          <TableCell colSpan={6} className="bg-muted/30 p-0">
            <pre className="overflow-auto p-4 text-xs font-mono leading-relaxed max-h-[300px]">
              {logs?.stdout || logs?.stderr ? (
                <>
                  {logs.stdout}
                  {logs.stderr && (
                    <span className="text-red-500">{logs.stderr}</span>
                  )}
                </>
              ) : (
                <span className="text-muted-foreground">No output yet.</span>
              )}
            </pre>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

export function Jobs() {
  const [filter, setFilter] = useState("");

  // Fetch all sandboxes, then fetch detached jobs for each
  const { data: sandboxes } = useQuery({
    queryKey: ["sandboxes"],
    queryFn: () => api.listSandboxes(),
    refetchInterval: 5000,
  });

  const runningSandboxes = useMemo(
    () => (sandboxes ?? []).filter((s) => s.status.toLowerCase() === "running"),
    [sandboxes],
  );

  const { data: allJobs, isLoading, refetch } = useQuery({
    queryKey: ["all-jobs", runningSandboxes.map((s) => s.name).join(",")],
    queryFn: async () => {
      const results: (DetachedCommand & { sandboxName: string })[] = [];
      for (const sb of runningSandboxes) {
        try {
          const jobs = await api.listDetached(sb.name);
          for (const job of jobs) {
            results.push({ ...job, sandboxName: sb.name });
          }
        } catch {
          // sandbox may have stopped between queries
        }
      }
      return results.sort(
        (a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime(),
      );
    },
    enabled: runningSandboxes.length > 0,
    refetchInterval: 5000,
  });

  const filtered = useMemo(() => {
    if (!allJobs) return [];
    if (!filter.trim()) return allJobs;
    const q = filter.toLowerCase();
    return allJobs.filter(
      (j) =>
        j.sandboxName.toLowerCase().includes(q) ||
        j.command.join(" ").toLowerCase().includes(q) ||
        j.status.toLowerCase().includes(q),
    );
  }, [allJobs, filter]);

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[400px] rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Jobs</h1>
          <p className="text-muted-foreground">
            Background commands running across all sandboxes
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      <div className="relative max-w-sm">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          placeholder="Filter jobs..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="pl-9"
        />
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8 px-2" />
                <TableHead>Sandbox</TableHead>
                <TableHead>Command</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Started</TableHead>
                <TableHead className="w-[60px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="h-24 text-center text-muted-foreground">
                    {runningSandboxes.length === 0
                      ? "No running sandboxes. Start a sandbox to run background jobs."
                      : "No background jobs found."}
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((job) => <JobRow key={job.id} job={job} />)
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
