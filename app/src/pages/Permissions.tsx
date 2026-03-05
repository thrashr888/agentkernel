import { Trash2, AlertTriangle, ShieldCheck, RefreshCw } from "lucide-react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const scopeColors: Record<string, string> = {
  once: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200",
  session: "bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200",
  always: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200",
};

export function Permissions() {
  const queryClient = useQueryClient();

  const {
    data: grants,
    isLoading,
    error,
    refetch,
    isRefetching,
  } = useQuery({
    queryKey: ["permissions"],
    queryFn: () => api.listPermissions(),
    retry: false,
  });

  const revokeMutation = useMutation({
    mutationFn: (id: string) => api.revokePermission(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["permissions"] });
      toast.success("Permission revoked");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Permissions</h1>
        <p className="text-muted-foreground">
          Manage interactive permission grants for sandbox operations
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Active Grants</CardTitle>
          <CardDescription>
            Permission grants control which operations agents can perform.
            Grants can be scoped to a single use, the current session, or
            persisted permanently.
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
                    Failed to load permissions
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
                    <RefreshCw
                      className={`h-3.5 w-3.5 mr-1.5 ${isRefetching ? "animate-spin" : ""}`}
                    />
                    {isRefetching ? "Retrying..." : "Retry"}
                  </Button>
                </div>
              </div>
            </div>
          )}

          {grants && grants.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Permission</TableHead>
                  <TableHead>Scope</TableHead>
                  <TableHead>Sandbox</TableHead>
                  <TableHead>Granted By</TableHead>
                  <TableHead>Granted At</TableHead>
                  <TableHead className="w-[100px] text-right">
                    Actions
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {grants.map((grant) => (
                  <TableRow key={grant.id}>
                    <TableCell className="font-mono text-sm">
                      {grant.kind}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant="secondary"
                        className={scopeColors[grant.scope] || ""}
                      >
                        {grant.scope}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-sm">
                      {grant.sandbox || "all"}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {grant.granted_by}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {new Date(grant.granted_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => revokeMutation.mutate(grant.id)}
                        disabled={revokeMutation.isPending}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}

          {grants && grants.length === 0 && !isLoading && (
            <div className="flex flex-col items-center gap-2 py-4 text-center">
              <ShieldCheck className="h-8 w-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">
                No active permission grants
              </p>
              <p className="text-xs text-muted-foreground/70">
                Permission grants are created when agents request elevated
                access and you approve. They can also be managed via the MCP
                permission_grant tool or HTTP API.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
