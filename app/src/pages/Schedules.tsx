import { useState, useMemo } from "react";
import {
  Trash2,
  AlertTriangle,
  Timer,
  RefreshCw,
  Plus,
  Play,
  ArrowLeft,
  ExternalLink,
  ChevronRight,
  ChevronDown,
} from "lucide-react";
import { useMutation, useQueryClient, useQuery } from "@tanstack/react-query";
import { useSchedules } from "@/lib/hooks/use-schedules";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Link } from "react-router-dom";
import type { AuditLogEntry } from "@/lib/types";

function statusBadge(status: string) {
  switch (status) {
    case "active":
      return <Badge variant="success">Active</Badge>;
    case "paused":
      return <Badge variant="warning">Paused</Badge>;
    case "completed":
      return <Badge variant="secondary">Completed</Badge>;
    case "failed":
      return <Badge variant="destructive">Failed</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function scheduleType(schedule: { cron?: string; fire_at?: string }) {
  if (schedule.cron) return "Cron";
  if (schedule.fire_at) return "One-shot";
  return "Unknown";
}

function target(schedule: {
  target_class?: string;
  target_object_id?: string;
  target_orchestration?: string;
}) {
  if (schedule.target_class && schedule.target_object_id) {
    return `${schedule.target_class}/${schedule.target_object_id}`;
  }
  if (schedule.target_orchestration) {
    return schedule.target_orchestration;
  }
  return "—";
}

function ScheduleExecutionLog({
  scheduleName,
  scheduleId,
}: {
  scheduleName: string;
  scheduleId: string;
}) {
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);

  const { data: allEntries, isLoading } = useQuery({
    queryKey: ["audit-log", 500],
    queryFn: () => api.getAuditLog(500),
    refetchInterval: 10000,
  });

  const filtered = useMemo(() => {
    if (!allEntries) return [];
    const terms = [scheduleName.toLowerCase(), scheduleId.toLowerCase()];
    return allEntries.filter((e) => {
      const text = JSON.stringify(e).toLowerCase();
      return terms.some((t) => text.includes(t));
    });
  }, [allEntries, scheduleName, scheduleId]);

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle>Execution Log</CardTitle>
            <CardDescription>
              Audit events matching this schedule ({filtered.length} entries)
            </CardDescription>
          </div>
          <Link to={`/audit?filter=${encodeURIComponent(scheduleName)}`}>
            <Button variant="ghost" size="sm">
              <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
              Full Log
            </Button>
          </Link>
        </div>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-full" />
          </div>
        ) : filtered.length === 0 ? (
          <p className="text-sm text-muted-foreground text-center py-4">
            No audit events found for this schedule yet.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8 px-2" />
                <TableHead className="whitespace-nowrap">Timestamp</TableHead>
                <TableHead>Event</TableHead>
                <TableHead>Details</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.slice(0, 20).map((entry: AuditLogEntry, idx: number) => {
                const isExpanded = expandedIdx === idx;
                return (
                  <>
                    <TableRow
                      key={`row-${idx}`}
                      className="cursor-pointer"
                      onClick={() => setExpandedIdx(isExpanded ? null : idx)}
                    >
                      <TableCell className="w-8 px-2">
                        {isExpanded ? (
                          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
                        ) : (
                          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                        )}
                      </TableCell>
                      <TableCell className="font-mono text-xs whitespace-nowrap">
                        {entry.timestamp}
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">{entry.type ?? "unknown"}</Badge>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground max-w-[300px] truncate">
                        {entry.user ?? ""}{" "}
                        {typeof entry.name === "string" ? entry.name : ""}
                      </TableCell>
                    </TableRow>
                    {isExpanded && (
                      <TableRow key={`detail-${idx}`}>
                        <TableCell colSpan={4} className="bg-muted/30 p-0">
                          <pre className="overflow-auto p-4 text-xs font-mono leading-relaxed">
                            {JSON.stringify(entry, null, 2)}
                          </pre>
                        </TableCell>
                      </TableRow>
                    )}
                  </>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}

function ScheduleDetail({
  scheduleId,
  onBack,
}: {
  scheduleId: string;
  onBack: () => void;
}) {
  const queryClient = useQueryClient();
  const { data: sched, isLoading, error } = useQuery({
    queryKey: ["schedules", scheduleId],
    queryFn: () => api.getSchedule(scheduleId),
    refetchInterval: 5000,
  });

  const triggerMutation = useMutation({
    mutationFn: () => api.triggerSchedule(scheduleId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
      toast.success("Schedule triggered");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  if (error || !sched) {
    return (
      <div className="space-y-6">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
          <p className="text-sm text-destructive">
            {error instanceof Error ? error.message : "Schedule not found"}
          </p>
        </div>
      </div>
    );
  }

  const hasObjectTarget = sched.target_class && sched.target_object_id;
  const hasOrchestrationTarget = !!sched.target_orchestration;
  const argsJson = sched.args
    ? JSON.stringify(sched.args, null, 2)
    : "{}";

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <div className="flex-1">
          <h1 className="text-3xl font-bold tracking-tight">{sched.name}</h1>
          <p className="text-muted-foreground flex items-center gap-2">
            {statusBadge(sched.status)}
            <Badge variant="outline">{scheduleType(sched)}</Badge>
            <span className="text-sm">
              Created {new Date(sched.created_at).toLocaleString()}
            </span>
          </p>
        </div>
        <Button
          onClick={() => triggerMutation.mutate()}
          disabled={triggerMutation.isPending}
        >
          <Play className="h-4 w-4 mr-2" />
          {triggerMutation.isPending ? "Triggering..." : "Trigger Now"}
        </Button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <Card>
          <CardHeader>
            <CardTitle>Details</CardTitle>
          </CardHeader>
          <CardContent>
            <dl className="space-y-3 text-sm">
              <div className="flex justify-between">
                <dt className="text-muted-foreground">ID</dt>
                <dd className="font-mono text-xs">{sched.id}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Name</dt>
                <dd className="font-medium">{sched.name}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Type</dt>
                <dd><Badge variant="outline">{scheduleType(sched)}</Badge></dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Status</dt>
                <dd>{statusBadge(sched.status)}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Method</dt>
                <dd className="font-mono">{sched.method}</dd>
              </div>
              {sched.cron && (
                <div className="flex justify-between">
                  <dt className="text-muted-foreground">Cron</dt>
                  <dd className="font-mono">{sched.cron}</dd>
                </div>
              )}
              {sched.fire_at && (
                <div className="flex justify-between">
                  <dt className="text-muted-foreground">Fire At</dt>
                  <dd className="font-mono">{sched.fire_at}</dd>
                </div>
              )}
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Last Fired</dt>
                <dd>
                  {sched.last_fired_at
                    ? new Date(sched.last_fired_at).toLocaleString()
                    : "Never"}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Updated</dt>
                <dd>{new Date(sched.updated_at).toLocaleString()}</dd>
              </div>
            </dl>
          </CardContent>
        </Card>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Target</CardTitle>
              <CardDescription>
                {hasObjectTarget
                  ? "This schedule targets a durable object"
                  : hasOrchestrationTarget
                    ? "This schedule targets an orchestration"
                    : "No target configured"}
              </CardDescription>
            </CardHeader>
            <CardContent>
              {hasObjectTarget && (
                <div className="space-y-3">
                  <dl className="space-y-2 text-sm">
                    <div className="flex justify-between">
                      <dt className="text-muted-foreground">Class</dt>
                      <dd className="font-mono">{sched.target_class}</dd>
                    </div>
                    <div className="flex justify-between">
                      <dt className="text-muted-foreground">Object ID</dt>
                      <dd className="font-mono">{sched.target_object_id}</dd>
                    </div>
                  </dl>
                  <Link
                    to="/objects"
                    className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                    View in Objects
                  </Link>
                </div>
              )}
              {hasOrchestrationTarget && (
                <div className="space-y-3">
                  <dl className="text-sm">
                    <div className="flex justify-between">
                      <dt className="text-muted-foreground">Orchestration</dt>
                      <dd className="font-mono">{sched.target_orchestration}</dd>
                    </div>
                  </dl>
                </div>
              )}
              {!hasObjectTarget && !hasOrchestrationTarget && (
                <p className="text-sm text-muted-foreground">
                  This schedule has no specific target. It will invoke the method
                  directly when triggered.
                </p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Arguments</CardTitle>
              <CardDescription>
                Data passed to the method when the schedule fires
              </CardDescription>
            </CardHeader>
            <CardContent>
              <pre className="overflow-auto rounded-md bg-muted p-3 text-xs font-mono max-h-[200px]">
                {argsJson}
              </pre>
            </CardContent>
          </Card>
        </div>
      </div>

      <ScheduleExecutionLog scheduleName={sched.name} scheduleId={sched.id} />
    </div>
  );
}

export function Schedules() {
  const queryClient = useQueryClient();
  const { data: schedules, isLoading, error, refetch, isRefetching } = useSchedules();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newCron, setNewCron] = useState("");
  const [newFireAt, setNewFireAt] = useState("");
  const [newMethod, setNewMethod] = useState("");

  const [selectedScheduleId, setSelectedScheduleId] = useState<string | null>(null);

  const createMutation = useMutation({
    mutationFn: () =>
      api.createSchedule({
        name: newName.trim(),
        cron: newCron.trim() || undefined,
        fire_at: newFireAt.trim() || undefined,
        method: newMethod.trim(),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
      setNewName("");
      setNewCron("");
      setNewFireAt("");
      setNewMethod("");
      setDialogOpen(false);
      toast.success("Schedule created");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.deleteSchedule(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
      toast.success("Schedule deleted");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const triggerMutation = useMutation({
    mutationFn: (id: string) => api.triggerSchedule(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
      toast.success("Schedule triggered");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  if (selectedScheduleId) {
    return (
      <ScheduleDetail
        scheduleId={selectedScheduleId}
        onBack={() => setSelectedScheduleId(null)}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Schedules</h1>
          <p className="text-muted-foreground">
            Manage cron and one-shot scheduled tasks
          </p>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button>
              <Plus className="h-4 w-4 mr-2" />
              New Schedule
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Schedule</DialogTitle>
              <DialogDescription>
                Create a cron or one-shot schedule.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <div className="grid gap-2">
                <Label htmlFor="sched-name">Name</Label>
                <Input
                  id="sched-name"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder="daily-cleanup"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="sched-cron">Cron Expression (or leave empty for one-shot)</Label>
                <Input
                  id="sched-cron"
                  value={newCron}
                  onChange={(e) => setNewCron(e.target.value)}
                  placeholder="0 0 * * *"
                  className="font-mono"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="sched-fire-at">Fire At (ISO 8601, for one-shot)</Label>
                <Input
                  id="sched-fire-at"
                  value={newFireAt}
                  onChange={(e) => setNewFireAt(e.target.value)}
                  placeholder="2026-03-01T00:00:00Z"
                  className="font-mono"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="sched-method">Method</Label>
                <Input
                  id="sched-method"
                  value={newMethod}
                  onChange={(e) => setNewMethod(e.target.value)}
                  placeholder="cleanup"
                  className="font-mono"
                />
              </div>
            </div>
            <DialogFooter>
              <Button
                onClick={() => createMutation.mutate()}
                disabled={
                  !newName.trim() ||
                  !newMethod.trim() ||
                  (!newCron.trim() && !newFireAt.trim()) ||
                  createMutation.isPending
                }
              >
                {createMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Schedules</CardTitle>
          <CardDescription>
            Schedules trigger methods on objects or orchestrations
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          )}

          {error && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
                <div className="flex-1 space-y-2">
                  <p className="text-sm font-medium text-destructive">
                    Failed to load schedules
                  </p>
                  <p className="text-xs text-destructive/80">
                    {error instanceof Error ? error.message : String(error)}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => refetch()}
                    disabled={isRefetching}
                    className="mt-1"
                  >
                    <RefreshCw className={`h-3.5 w-3.5 mr-1.5 ${isRefetching ? "animate-spin" : ""}`} />
                    {isRefetching ? "Retrying..." : "Retry"}
                  </Button>
                </div>
              </div>
            </div>
          )}

          {schedules && schedules.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Method</TableHead>
                  <TableHead>Target</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Last Fired</TableHead>
                  <TableHead>Schedule</TableHead>
                  <TableHead className="w-[100px] text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {schedules.map((sched) => (
                  <TableRow key={sched.id}>
                    <TableCell>
                      <button
                        onClick={() => setSelectedScheduleId(sched.id)}
                        className="font-medium text-primary hover:underline text-left"
                      >
                        {sched.name}
                      </button>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">{scheduleType(sched)}</Badge>
                    </TableCell>
                    <TableCell className="font-mono text-sm">{sched.method}</TableCell>
                    <TableCell className="font-mono text-sm text-muted-foreground">
                      {target(sched)}
                    </TableCell>
                    <TableCell>{statusBadge(sched.status)}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {sched.last_fired_at
                        ? new Date(sched.last_fired_at).toLocaleString()
                        : "Never"}
                    </TableCell>
                    <TableCell className="font-mono text-sm text-muted-foreground">
                      {sched.cron || sched.fire_at || "—"}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          title="Trigger now"
                          onClick={() => triggerMutation.mutate(sched.id)}
                          disabled={triggerMutation.isPending}
                        >
                          <Play className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => deleteMutation.mutate(sched.id)}
                          disabled={deleteMutation.isPending}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}

          {schedules && schedules.length === 0 && !isLoading && (
            <div className="flex flex-col items-center gap-2 py-4 text-center">
              <Timer className="h-8 w-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">No schedules yet</p>
              <p className="text-xs text-muted-foreground/70">
                Create a cron or one-shot schedule using the button above.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
