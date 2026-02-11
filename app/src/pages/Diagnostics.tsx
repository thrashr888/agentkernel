import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, CheckCircle, AlertTriangle, XCircle, Info, Trash2 } from "lucide-react";
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
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";

function statusIcon(status: string) {
  switch (status) {
    case "ok":
      return <CheckCircle className="h-4 w-4 text-green-600 dark:text-green-400" />;
    case "warning":
      return <AlertTriangle className="h-4 w-4 text-yellow-600 dark:text-yellow-400" />;
    case "error":
      return <XCircle className="h-4 w-4 text-red-600 dark:text-red-400" />;
    default:
      return <Info className="h-4 w-4 text-muted-foreground" />;
  }
}

function statusBadgeVariant(status: string) {
  switch (status) {
    case "ok":
      return "success" as const;
    case "warning":
      return "warning" as const;
    case "error":
      return "destructive" as const;
    default:
      return "secondary" as const;
  }
}

export function Diagnostics() {
  const queryClient = useQueryClient();

  const gcMutation = useMutation({
    mutationFn: () => api.runGc(),
    onMutate: () => {
      const toastId = toast("Running garbage collection...");
      return { toastId };
    },
    onSuccess: (data, _variables, context) => {
      toast.update(
        context.toastId,
        `Removed ${data.count} sandbox${data.count === 1 ? "" : "es"}`,
        "success",
      );
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err, _variables, context) => {
      if (context?.toastId) {
        toast.update(context.toastId, "Garbage collection failed", "error");
      }
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const {
    data: status,
    isLoading: statusLoading,
    error: statusError,
    refetch: refetchStatus,
  } = useQuery({
    queryKey: ["status"],
    queryFn: async () => {
      try {
        return await api.getStatus();
      } catch (err: unknown) {
        toast.error(err instanceof Error ? err.message : String(err));
        throw err;
      }
    },
  });

  const {
    data: doctor,
    isLoading: doctorLoading,
    error: doctorError,
    refetch: refetchDoctor,
  } = useQuery({
    queryKey: ["doctor"],
    queryFn: async () => {
      try {
        return await api.getDoctor();
      } catch (err: unknown) {
        toast.error(err instanceof Error ? err.message : String(err));
        throw err;
      }
    },
  });

  const isLoading = statusLoading || doctorLoading;

  function handleRefresh() {
    refetchStatus();
    refetchDoctor();
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[200px] rounded-lg" />
        <Skeleton className="h-[300px] rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Diagnostics</h1>
          <p className="text-muted-foreground">
            System status and health checks
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={handleRefresh}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {/* System Status */}
      <Card>
        <CardHeader>
          <CardTitle>System Status</CardTitle>
          <CardDescription>
            Current system configuration and version information
          </CardDescription>
        </CardHeader>
        <CardContent>
          {statusError ? (
            <p className="text-sm text-destructive">
              Failed to load status:{" "}
              {statusError instanceof Error
                ? statusError.message
                : String(statusError)}
            </p>
          ) : status ? (
            <div className="space-y-3">
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Version</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {status.version}
                </span>
              </div>
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Backend</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {status.backend}
                </span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium">API Key</span>
                <Badge
                  variant={status.api_key_configured ? "success" : "secondary"}
                >
                  {status.api_key_configured ? "Configured" : "Not configured"}
                </Badge>
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>

      {/* Health Checks */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Health Checks</CardTitle>
              <CardDescription>
                Dependency and runtime health checks
              </CardDescription>
            </div>
            {doctor && (
              <Badge variant={doctor.healthy ? "success" : "destructive"}>
                {doctor.healthy ? "Healthy" : "Unhealthy"}
              </Badge>
            )}
          </div>
        </CardHeader>
        <CardContent>
          {doctorError ? (
            <p className="text-sm text-destructive">
              Failed to run health checks:{" "}
              {doctorError instanceof Error
                ? doctorError.message
                : String(doctorError)}
            </p>
          ) : doctor ? (
            <div className="space-y-3">
              {doctor.checks.map((check) => (
                <div
                  key={check.name}
                  className="flex items-center justify-between border-b pb-2 last:border-0"
                >
                  <div className="flex items-center gap-3">
                    {statusIcon(check.status)}
                    <div>
                      <span className="text-sm font-medium">
                        {check.name}
                      </span>
                      <p className="text-xs text-muted-foreground">
                        {check.message}
                      </p>
                    </div>
                  </div>
                  <Badge variant={statusBadgeVariant(check.status)}>
                    {check.status}
                  </Badge>
                </div>
              ))}
            </div>
          ) : null}
        </CardContent>
      </Card>

      {/* Maintenance */}
      <Card>
        <CardHeader>
          <CardTitle>Maintenance</CardTitle>
          <CardDescription>
            Manage sandbox lifecycle and cleanup
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Garbage Collection</p>
              <p className="text-xs text-muted-foreground">
                Remove expired sandboxes to free up resources
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => gcMutation.mutate()}
              disabled={gcMutation.isPending}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              {gcMutation.isPending ? "Running..." : "Run Garbage Collection"}
            </Button>
          </div>
          {gcMutation.isSuccess && gcMutation.data && (
            <div className="rounded-md border p-3 space-y-2">
              <p className="text-sm font-medium">
                Removed {gcMutation.data.count} sandbox
                {gcMutation.data.count === 1 ? "" : "es"}
              </p>
              {gcMutation.data.removed.length > 0 && (
                <ul className="space-y-1">
                  {gcMutation.data.removed.map((name) => (
                    <li key={name} className="text-xs font-mono text-muted-foreground">
                      {name}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
