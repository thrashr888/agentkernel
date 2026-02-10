import { useQuery } from "@tanstack/react-query";
import { Shield, RefreshCw } from "lucide-react";
import { api } from "@/lib/api";
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

export function Policy() {
  const {
    data: policyStatus,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["policy-status"],
    queryFn: () => api.getPolicyStatus(),
    retry: false,
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[200px] rounded-lg" />
      </div>
    );
  }

  // If the query errored (e.g. 404 because enterprise is not enabled)
  if (error) {
    return (
      <div className="space-y-6">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Policy</h1>
          <p className="text-muted-foreground">
            Enterprise policy management
          </p>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12 text-center">
            <Shield className="mb-4 h-12 w-12 text-muted-foreground/50" />
            <h2 className="text-lg font-semibold">
              Enterprise features are not available
            </h2>
            <p className="mt-2 max-w-md text-sm text-muted-foreground">
              Policy management requires the enterprise feature flag. Rebuild
              the server with{" "}
              <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                --features enterprise
              </code>{" "}
              to enable policy enforcement.
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Policy</h1>
          <p className="text-muted-foreground">
            Enterprise policy management
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {policyStatus && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle>Policy Engine</CardTitle>
                <CardDescription>
                  Current policy engine configuration
                </CardDescription>
              </div>
              <Badge variant={policyStatus.enabled ? "success" : "secondary"}>
                {policyStatus.enabled ? "Enabled" : "Disabled"}
              </Badge>
            </div>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Version</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.version}
                </span>
              </div>
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Organization ID</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.org_id ?? "N/A"}
                </span>
              </div>
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Offline Mode</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.offline_mode ?? "N/A"}
                </span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium">Policy Server</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.policy_server ?? "N/A"}
                </span>
              </div>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
