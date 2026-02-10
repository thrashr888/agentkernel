import { Link, useNavigate } from "react-router-dom";
import { Wifi, WifiOff, MoreHorizontal, Copy, Terminal, Loader2, Rocket } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useSandboxes } from "@/lib/hooks/use-sandboxes";
import { useHealth } from "@/lib/hooks/use-health";
import { api } from "@/lib/api";
import { StatusCards } from "@/components/dashboard/status-cards";
import { QuickActions } from "@/components/dashboard/quick-actions";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/use-toast";
import { formatRelativeDate } from "@/lib/utils";

const AGENTS = [
  { id: "claude", name: "Claude Code", description: "Anthropic" },
  { id: "codex", name: "Codex", description: "OpenAI" },
  { id: "gemini", name: "Gemini CLI", description: "Google" },
  { id: "amp", name: "Amp", description: "Sourcegraph" },
  { id: "opencode", name: "OpenCode", description: "Multi-provider" },
  { id: "pi", name: "Pi", description: "Mario Zechner" },
  { id: "copilot", name: "Copilot CLI", description: "GitHub" },
] as const;

export function Dashboard() {
  const { data: sandboxes, isLoading, error } = useSandboxes();
  const { isConnected } = useHealth();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const openTerminalMutation = useMutation({
    mutationFn: (sandboxName: string) => api.openTerminal(sandboxName),
    onMutate: () => {
      return { toastId: toast("Opening terminal...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Terminal opened", "success");
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const quickstartMutation = useMutation({
    mutationFn: ({ agent, name }: { agent: string; name: string }) =>
      api.quickstartAgent(agent, name),
    onMutate: ({ agent, name }) => {
      // Navigate immediately so the user sees the detail page while sandbox is being created
      navigate(`/sandboxes/${name}`);
      return { toastId: toast(`Starting ${agent}... creating sandbox and installing CLI`), agent };
    },
    onSuccess: (_sandboxName, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, `${context.agent} is ready! Terminal opened.`, "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

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
        <h2 className="text-lg font-semibold">Quickstart</h2>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {AGENTS.map((agent) => (
            <Card key={agent.id} className="transition-colors hover:bg-accent/50">
              <CardContent className="flex items-center justify-between p-4">
                <div>
                  <p className="font-medium text-sm">{agent.name}</p>
                  <p className="text-xs text-muted-foreground">{agent.description}</p>
                </div>
                <Button
                  size="sm"
                  disabled={quickstartMutation.isPending}
                  onClick={() => {
                    const name = `${agent.id}-${Date.now() % 10000}`;
                    quickstartMutation.mutate({ agent: agent.id, name });
                  }}
                >
                  {quickstartMutation.isPending && quickstartMutation.variables?.agent === agent.id ? (
                    <Loader2 className="mr-2 h-3 w-3 animate-spin" />
                  ) : (
                    <Rocket className="mr-2 h-3 w-3" />
                  )}
                  Start
                </Button>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>

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
              <Card key={sandbox.name} className="transition-colors hover:bg-accent/50">
                <CardContent className="flex items-center justify-between p-4">
                  <Link
                    to={`/sandboxes/${sandbox.name}`}
                    className="flex items-center gap-4 flex-1 min-w-0"
                  >
                    <div>
                      <p className="font-medium">{sandbox.name}</p>
                      <p className="text-sm text-muted-foreground">
                        {sandbox.image ?? sandbox.backend}
                      </p>
                    </div>
                  </Link>
                  <div className="flex items-center gap-4 shrink-0">
                    {sandbox.created_at && (
                      <span className="text-sm text-muted-foreground">
                        {formatRelativeDate(sandbox.created_at)}
                      </span>
                    )}
                    <SandboxStatusBadge status={sandbox.status} />
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-8 w-8">
                          <MoreHorizontal className="h-4 w-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          onClick={() => {
                            navigator.clipboard.writeText(`agentkernel attach ${sandbox.name}`);
                            toast.success("Connection string copied");
                          }}
                        >
                          <Copy className="mr-2 h-4 w-4" />
                          Copy Connection String
                        </DropdownMenuItem>
                        {sandbox.status.toLowerCase() === "running" && (
                          <DropdownMenuItem
                            onClick={() => openTerminalMutation.mutate(sandbox.name)}
                          >
                            <Terminal className="mr-2 h-4 w-4" />
                            Open Terminal
                          </DropdownMenuItem>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
