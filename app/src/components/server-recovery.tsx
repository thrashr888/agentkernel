import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { Loader2, RefreshCw, Server, Settings2, Wrench } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

interface ServerRecoveryProps {
  error: unknown;
  onRetry: () => void | Promise<unknown>;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isBackendUnavailable(message: string): boolean {
  return /no sandbox backend available|no sandbox backend|backend error/i.test(message);
}

export function ServerRecovery({ error, onRetry }: ServerRecoveryProps) {
  const navigate = useNavigate();
  const [action, setAction] = useState<"starting" | "preparing" | null>(null);
  const message = errorMessage(error);
  const backendUnavailable = isBackendUnavailable(message);

  async function startServer() {
    setAction("starting");
    try {
      await api.startServer();
      for (let attempt = 0; attempt < 10; attempt += 1) {
        if ((await api.checkConnection().catch(() => "")) === "ok") break;
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      toast.success("AgentKernel server is ready");
      await onRetry();
    } catch (startError) {
      toast.error(startError instanceof Error ? startError.message : String(startError));
    } finally {
      setAction(null);
    }
  }

  async function prepareBackend() {
    setAction("preparing");
    try {
      const result = await api.prepareBackend();
      toast.success(result);
      await new Promise((resolve) => setTimeout(resolve, 750));
      await onRetry();
    } catch (prepareError) {
      toast.error(prepareError instanceof Error ? prepareError.message : String(prepareError));
    } finally {
      setAction(null);
    }
  }

  return (
    <Card>
      <CardContent className="space-y-3 pt-6">
        <div className="flex items-center gap-2 text-sm font-medium text-destructive">
          {backendUnavailable ? (
            <Wrench className="h-4 w-4" />
          ) : (
            <Server className="h-4 w-4" />
          )}
          {backendUnavailable
            ? "The server is running, but no sandbox backend is ready."
            : "The AgentKernel server is not reachable."}
        </div>
        <p className="text-sm text-muted-foreground">
          {backendUnavailable
            ? "Prepare Apple Containers or Docker, then retry."
            : "Start the local server or check the active server connection in Settings."}
        </p>
        <p className="break-words rounded bg-muted px-2 py-1 text-xs text-muted-foreground">
          {message}
        </p>
        <div className="flex flex-wrap gap-2">
          {backendUnavailable ? (
            <Button size="sm" onClick={prepareBackend} disabled={action !== null}>
              {action === "preparing" ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Wrench className="mr-2 h-4 w-4" />
              )}
              Prepare Backend
            </Button>
          ) : (
            <Button size="sm" onClick={startServer} disabled={action !== null}>
              {action === "starting" ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Server className="mr-2 h-4 w-4" />
              )}
              Start Server
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={() => navigate("/settings")}>
            <Settings2 className="mr-2 h-4 w-4" />
            Open Settings
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void onRetry()} disabled={action !== null}>
            <RefreshCw className="mr-2 h-4 w-4" />
            Retry
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
