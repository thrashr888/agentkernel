import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Download, Loader2, Copy, Check } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/use-toast";

const INSTALL_COMMANDS: Record<string, string> = {
  claude: "npm install -g @anthropic-ai/claude-code",
  gemini: "npm install -g @anthropic-ai/gemini-cli",
  codex: "npm install -g @openai/codex",
  opencode: "cargo install opencode",
  amp: "npm install -g @sourcegraph/amp",
  pi: "npm install -g @mariozechner/pi-coding-agent",
  copilot: "npm install -g @githubnext/github-copilot-cli",
};

export function Plugins() {
  const queryClient = useQueryClient();
  const [copiedAgent, setCopiedAgent] = useState<string | null>(null);

  function copyInstallCmd(agentName: string) {
    const cmd = INSTALL_COMMANDS[agentName];
    if (!cmd) return;
    navigator.clipboard.writeText(cmd);
    setCopiedAgent(agentName);
    setTimeout(() => setCopiedAgent(null), 2000);
  }
  const {
    data: agents,
    isLoading,
    error,
  } = useQuery({
    queryKey: ["agents"],
    queryFn: api.listAgents,
  });

  const installMutation = useMutation({
    mutationFn: (name: string) => api.installAgent(name),
    onMutate: (name) => {
      return { toastId: toast(`Installing ${name}...`), name };
    },
    onSuccess: (msg, _name, context) => {
      if (context?.toastId) toast.update(context.toastId, msg, "success");
      queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
    onError: (err: unknown, _name, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          err instanceof Error ? err.message : String(err),
          "error"
        );
    },
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Plugins</h1>
        <p className="text-muted-foreground">
          Agent integrations supported by AgentKernel
        </p>
      </div>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3, 4].map((i) => (
            <Skeleton key={i} className="h-[140px] rounded-lg" />
          ))}
        </div>
      ) : error ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load plugins:{" "}
              {error instanceof Error ? error.message : String(error)}
            </p>
          </CardContent>
        </Card>
      ) : agents && agents.length === 0 ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-muted-foreground">
              No agent plugins available.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {(agents ?? []).map((agent) => (
            <Card key={agent.name}>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <CardTitle className="text-base">
                    {agent.display_name}
                  </CardTitle>
                  <Badge variant={agent.enabled ? "success" : "secondary"}>
                    {agent.enabled ? "Installed" : "Not Installed"}
                  </Badge>
                </div>
                <CardDescription>{agent.description}</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center justify-between gap-2">
                  <button
                    type="button"
                    className="group flex items-center gap-1.5 text-xs text-muted-foreground font-mono truncate hover:text-foreground"
                    onClick={() => copyInstallCmd(agent.name)}
                    title="Copy install command"
                  >
                    <span className="truncate">
                      {INSTALL_COMMANDS[agent.name] ?? agent.name}
                    </span>
                    {copiedAgent === agent.name ? (
                      <Check className="h-3 w-3 shrink-0 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-50" />
                    )}
                  </button>
                  {!agent.enabled && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                      disabled={installMutation.isPending}
                      onClick={() => installMutation.mutate(agent.name)}
                    >
                      {installMutation.isPending &&
                      installMutation.variables === agent.name ? (
                        <Loader2 className="mr-2 h-3 w-3 animate-spin" />
                      ) : (
                        <Download className="mr-2 h-3 w-3" />
                      )}
                      Install
                    </Button>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
