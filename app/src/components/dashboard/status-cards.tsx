import { Activity, Square, Box } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { SandboxInfo } from "@/lib/types";

interface StatusCardsProps {
  sandboxes: SandboxInfo[];
}

export function StatusCards({ sandboxes }: StatusCardsProps) {
  const running = sandboxes.filter(
    (s) => s.status.toLowerCase() === "running"
  ).length;
  const stopped = sandboxes.filter(
    (s) => s.status.toLowerCase() === "stopped"
  ).length;
  const total = sandboxes.length;

  return (
    <div className="grid gap-4 sm:grid-cols-3">
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
          <Box className="h-4 w-4 text-blue-500" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">
            {total}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
