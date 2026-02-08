import { Link } from "react-router-dom";
import { Wifi, WifiOff } from "lucide-react";
import { useSandboxes } from "@/lib/hooks/use-sandboxes";
import { useHealth } from "@/lib/hooks/use-health";
import { StatusCards } from "@/components/dashboard/status-cards";
import { QuickActions } from "@/components/dashboard/quick-actions";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeDate } from "@/lib/utils";

export function Dashboard() {
  const { data: sandboxes, isLoading, error } = useSandboxes();
  const { isConnected } = useHealth();

  const recentSandboxes = sandboxes
    ? [...sandboxes].sort((a, b) => a.name.localeCompare(b.name)).slice(0, 5)
    : [];

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">
          Overview of your AgentKernel sandboxes
        </p>
      </div>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-3">
          {[1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-[108px] rounded-lg" />
          ))}
        </div>
      ) : error ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load sandboxes: {error.message}
            </p>
          </CardContent>
        </Card>
      ) : (
        <StatusCards sandboxes={sandboxes ?? []} />
      )}

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <div>
            <CardTitle className="text-sm font-medium">
              Connection Status
            </CardTitle>
            <CardDescription>API server connectivity</CardDescription>
          </div>
          {isConnected ? (
            <Wifi className="h-5 w-5 text-green-500" />
          ) : (
            <WifiOff className="h-5 w-5 text-destructive" />
          )}
        </CardHeader>
        <CardContent>
          <p className="text-sm">
            {isConnected ? (
              <span className="text-green-600 dark:text-green-400">
                Connected
              </span>
            ) : (
              <span className="text-destructive">Disconnected</span>
            )}
            {" -- "}
            <span className="text-muted-foreground">
              http://localhost:18888
            </span>
          </p>
        </CardContent>
      </Card>

      <div className="space-y-4">
        <h2 className="text-lg font-semibold">Quick Actions</h2>
        <QuickActions />
      </div>

      <div className="space-y-4">
        <h2 className="text-lg font-semibold">Recent Sandboxes</h2>
        {isLoading ? (
          <div className="space-y-2">
            {[1, 2, 3].map((i) => (
              <Skeleton key={i} className="h-16 rounded-lg" />
            ))}
          </div>
        ) : recentSandboxes.length === 0 ? (
          <Card>
            <CardContent className="pt-6">
              <p className="text-sm text-muted-foreground">
                No sandboxes yet. Create one to get started.
              </p>
            </CardContent>
          </Card>
        ) : (
          <div className="space-y-2">
            {recentSandboxes.map((sandbox) => (
              <Link
                key={sandbox.name}
                to={`/sandboxes/${sandbox.name}`}
                className="block"
              >
                <Card className="transition-colors hover:bg-accent/50">
                  <CardContent className="flex items-center justify-between p-4">
                    <div className="flex items-center gap-4">
                      <div>
                        <p className="font-medium">{sandbox.name}</p>
                        <p className="text-sm text-muted-foreground">
                          {sandbox.image ?? sandbox.backend}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-4">
                      {sandbox.created_at && (
                        <span className="text-sm text-muted-foreground">
                          {formatRelativeDate(sandbox.created_at)}
                        </span>
                      )}
                      <SandboxStatusBadge status={sandbox.status} />
                    </div>
                  </CardContent>
                </Card>
              </Link>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
