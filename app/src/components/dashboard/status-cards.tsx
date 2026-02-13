import { Activity, Square, Box, Cpu, MemoryStick } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { SandboxInfo } from "@/lib/types";

interface StatusCardsProps {
  sandboxes: SandboxInfo[];
}

export function StatusCards({ sandboxes }: StatusCardsProps) {
  const runningSandboxes = sandboxes.filter(
    (s) => s.status.toLowerCase() === "running"
  );
  const running = runningSandboxes.length;
  const stopped = sandboxes.filter(
    (s) => s.status.toLowerCase() === "stopped"
  ).length;
  const total = sandboxes.length;
  const totalVcpus = runningSandboxes.reduce(
    (sum, s) => sum + (s.vcpus ?? 0),
    0
  );
  const totalMemoryMb = runningSandboxes.reduce(
    (sum, s) => sum + (s.memory_mb ?? 0),
    0
  );

  return (
    <div className="grid gap-4 sm:grid-cols-3 lg:grid-cols-5">
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">
            Running Sandboxes
          </CardTitle>
          <Activity className="h-4 w-4 text-green-500" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold text-green-600 dark:text-green-400">
            {running}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">Stopped</CardTitle>
          <Square className="h-4 w-4 text-muted-foreground" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold text-muted-foreground">
            {stopped}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">Total</CardTitle>
          <Box className="h-4 w-4 text-foreground" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold">
            {total}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">vCPUs</CardTitle>
          <Cpu className="h-4 w-4 text-blue-500" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">
            {totalVcpus}
          </div>
          <p className="text-xs text-muted-foreground">allocated</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-sm font-medium">Memory</CardTitle>
          <MemoryStick className="h-4 w-4 text-purple-500" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold text-purple-600 dark:text-purple-400">
            {totalMemoryMb >= 1024
              ? `${(totalMemoryMb / 1024).toFixed(1)} GB`
              : `${totalMemoryMb} MB`}
          </div>
          <p className="text-xs text-muted-foreground">allocated</p>
        </CardContent>
      </Card>
    </div>
  );
}
