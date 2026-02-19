import React from "react";
import { Link, useNavigate } from "react-router-dom";
import {
  MoreHorizontal,
  Copy,
  Terminal,
  Loader2,
  Rocket,
  Brain,
} from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useSandboxes } from "@/lib/hooks/use-sandboxes";
import { useLlmUsage } from "@/lib/hooks/use-llm-usage";
import { api } from "@/lib/api";
import { StatusCards } from "@/components/dashboard/status-cards";
import { QuickActions } from "@/components/dashboard/quick-actions";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
  const { data: llmUsage } = useLlmUsage();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const llmStats = React.useMemo(() => {
    if (!llmUsage) return null;
    let totalRequests = 0;
    let totalTokens = 0;
    const providers = new Set<string>();
    for (const entries of Object.values(llmUsage)) {
      for (const entry of entries) {
        totalRequests += entry.request_count;
        totalTokens += entry.total_tokens;
        providers.add(entry.provider);
      }
    }
    if (totalRequests === 0) return null;
    return { totalRequests, totalTokens, providers: providers.size };
  }, [llmUsage]);

  const openTerminalMutation = useMutation({
    mutationFn: (sandboxName: string) => api.openTerminal(sandboxName),
    onMutate: () => {
      return { toastId: toast("Opening terminal...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId)
        toast.update(context.toastId, "Terminal opened", "success");
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          err instanceof Error ? err.message : String(err),
          "error",
        );
    },
  });

  const quickstartMutation = useMutation({
    mutationFn: ({ agent, name }: { agent: string; name: string }) =>
      api.quickstartAgent(agent, name),
    onMutate: ({ agent, name }) => {
      // Navigate immediately so the user sees the detail page while sandbox is being created
      navigate(`/sandboxes/${name}`);
      return {
        toastId: toast(
          `Starting ${agent}... creating sandbox and installing CLI`,
        ),
        agent,
      };
    },
    onSuccess: (_sandboxName, _vars, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          `${context.agent} is ready! Terminal opened.`,
          "success",
        );
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          err instanceof Error ? err.message : String(err),
          "error",
        );
    },
  });

  const recentSandboxes = sandboxes
    ? [...sandboxes]
        .sort((a, b) => {
          // Sort by created_at descending (most recent first), fall back to name
          if (a.created_at && b.created_at)
            return b.created_at.localeCompare(a.created_at);
          if (a.created_at) return -1;
          if (b.created_at) return 1;
          return a.name.localeCompare(b.name);
        })
        .slice(0, 9)
    : [];

  return (
    <div className="space-y-6">
      {/* Status metrics bar */}
      {isLoading ? (
        <Skeleton className="h-10 rounded-md" />
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

      {/* LLM usage bar (compact) */}
      {llmStats && (
        <div className="flex items-center gap-6 rounded-md border px-4 py-3">
          <Brain className="h-4 w-4 text-violet-500 shrink-0" />
          <span className="text-sm font-medium">LLM Usage</span>
          <div className="flex items-center gap-6 text-sm">
            <span>
              <span className="font-mono font-medium">
                {llmStats.totalRequests.toLocaleString()}
              </span>
              <span className="text-muted-foreground ml-1">requests</span>
            </span>
            <span>
              <span className="font-mono font-medium">
                {llmStats.totalTokens >= 1_000_000
                  ? `${(llmStats.totalTokens / 1_000_000).toFixed(1)}M`
                  : llmStats.totalTokens >= 1_000
                    ? `${(llmStats.totalTokens / 1_000).toFixed(1)}K`
                    : llmStats.totalTokens.toLocaleString()}
              </span>
              <span className="text-muted-foreground ml-1">tokens</span>
            </span>
            <span>
              <span className="font-mono font-medium">
                {llmStats.providers}
              </span>
              <span className="text-muted-foreground ml-1">
                {llmStats.providers === 1 ? "provider" : "providers"}
              </span>
            </span>
          </div>
        </div>
      )}

      {/* Two-column layout: Recent Sandboxes | Quick Actions + Quickstart */}
      <div className="grid gap-6 lg:grid-cols-[1fr_340px]">
        {/* Left column — Recent Sandboxes */}
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-lg font-semibold">Recent Sandboxes</h2>
            <Link
              to="/sandboxes"
              className="text-sm text-blue-500 hover:underline"
            >
              View all
            </Link>
          </div>
          {isLoading ? (
            <div className="space-y-2">
              {[1, 2, 3].map((i) => (
                <Skeleton key={i} className="h-14 rounded-lg" />
              ))}
            </div>
          ) : recentSandboxes.length === 0 ? (
            <div className="rounded-md border p-6 text-center">
              <p className="text-sm text-muted-foreground">
                No sandboxes yet. Create one to get started.
              </p>
            </div>
          ) : (
            <div className="rounded-md border divide-y">
              {recentSandboxes.map((sandbox) => (
                <div
                  key={sandbox.name}
                  className="flex items-center justify-between px-4 py-3 hover:bg-accent/50 transition-colors"
                >
                  <Link
                    to={`/sandboxes/${sandbox.name}`}
                    className="flex items-center gap-3 flex-1 min-w-0"
                  >
                    <SandboxStatusBadge status={sandbox.status} />
                    <div className="min-w-0">
                      <p className="font-medium text-sm truncate">
                        {sandbox.name}
                      </p>
                      <p className="text-xs text-muted-foreground font-mono truncate">
                        {sandbox.image ?? sandbox.backend}
                      </p>
                    </div>
                  </Link>
                  <div className="flex items-center gap-2 shrink-0">
                    {sandbox.created_at && (
                      <span className="text-xs text-muted-foreground">
                        {formatRelativeDate(sandbox.created_at)}
                      </span>
                    )}
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7">
                          <MoreHorizontal className="h-4 w-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          onClick={() => {
                            navigator.clipboard.writeText(
                              `agentkernel attach ${sandbox.name}`,
                            );
                            toast.success("Connection string copied");
                          }}
                        >
                          <Copy className="mr-2 h-4 w-4" />
                          Copy Connection String
                        </DropdownMenuItem>
                        {sandbox.status.toLowerCase() === "running" && (
                          <DropdownMenuItem
                            onClick={() =>
                              openTerminalMutation.mutate(sandbox.name)
                            }
                          >
                            <Terminal className="mr-2 h-4 w-4" />
                            Open Terminal
                          </DropdownMenuItem>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Right column — Quickstart */}
        <div className="space-y-6">
          <div className="space-y-3">
            <h2 className="text-lg font-semibold">Quickstart</h2>
            <QuickActions />
            <div className="rounded-md border divide-y">
              {AGENTS.map((agent) => (
                <div
                  key={agent.id}
                  className="flex items-center justify-between px-3 py-2.5 hover:bg-accent/50 transition-colors"
                >
                  <div>
                    <p className="font-medium text-sm">{agent.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {agent.description}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-7 text-xs"
                    disabled={quickstartMutation.isPending}
                    onClick={() => {
                      const name = `${agent.id}-${Date.now() % 10000}`;
                      quickstartMutation.mutate({ agent: agent.id, name });
                    }}
                  >
                    {quickstartMutation.isPending &&
                    quickstartMutation.variables?.agent === agent.id ? (
                      <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                    ) : (
                      <Rocket className="mr-1 h-3 w-3" />
                    )}
                    Start
                  </Button>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
