import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { AgentInfo, AgentIntegrationResult } from "@/lib/types";
import { Check, Copy, Loader2, PlugZap } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/use-toast";

type InstallRequest = {
  agent: AgentInfo;
  scope: "project" | "global";
  confirm: boolean;
};

export function Plugins() {
  const queryClient = useQueryClient();
  const [copiedAgent, setCopiedAgent] = useState<string | null>(null);
  const [preview, setPreview] = useState<{
    agent: AgentInfo;
    result: AgentIntegrationResult;
  } | null>(null);

  const { data: agents, isLoading, error } = useQuery({
    queryKey: ["agents"],
    queryFn: api.listAgents,
  });

  const installMutation = useMutation({
    mutationFn: ({ agent, scope, confirm }: InstallRequest) =>
      api.installAgent(agent.name, scope, confirm),
    onSuccess: (result, request) => {
      if (request.confirm) {
        toast.success(`${request.agent.display_name} integration installed`);
        setPreview(null);
        queryClient.invalidateQueries({ queryKey: ["agents"] });
      } else {
        setPreview({ agent: request.agent, result });
      }
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  function copyInstallCommand(agent: AgentInfo) {
    navigator.clipboard.writeText(agent.install_command);
    setCopiedAgent(agent.name);
    setTimeout(() => setCopiedAgent(null), 2000);
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Agent integrations</h1>
        <p className="text-muted-foreground">
          See which agent CLIs are available on the server and connect them to AgentKernel.
        </p>
      </div>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3, 4].map((i) => (
            <Skeleton key={i} className="h-[240px] rounded-lg" />
          ))}
        </div>
      ) : error ? (
        <Card>
          <CardContent className="pt-6 text-sm text-destructive">
            Failed to load integrations: {error instanceof Error ? error.message : String(error)}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {(agents ?? []).map((agent) => {
            const busy =
              installMutation.isPending && installMutation.variables?.agent.name === agent.name;
            return (
              <Card key={agent.name}>
                <CardHeader>
                  <CardTitle className="text-base">{agent.display_name}</CardTitle>
                  <CardDescription>{agent.description}</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <section className="space-y-2">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium">CLI on server</span>
                      <Badge variant={agent.cli_installed ? "success" : "secondary"}>
                        {agent.cli_installed ? "Installed" : "Not installed"}
                      </Badge>
                    </div>
                    {agent.cli_installed ? (
                      <div className="text-xs text-muted-foreground">
                        <div className="truncate font-mono" title={agent.cli_version}>
                          {agent.cli_version ?? "Version unavailable"}
                        </div>
                        <span>
                          {agent.compatibility_status === "tested"
                            ? `Matches tested ${agent.tested_version}`
                            : `Tested with ${agent.tested_version}`}
                        </span>
                      </div>
                    ) : (
                      <button
                        type="button"
                        className="group flex max-w-full items-center gap-1.5 text-left text-xs text-muted-foreground hover:text-foreground"
                        onClick={() => copyInstallCommand(agent)}
                        title="Copy install command"
                      >
                        <span className="truncate font-mono">{agent.install_command}</span>
                        {copiedAgent === agent.name ? (
                          <Check className="h-3 w-3 shrink-0 text-green-500" />
                        ) : (
                          <Copy className="h-3 w-3 shrink-0 opacity-50" />
                        )}
                      </button>
                    )}
                  </section>

                  <section className="space-y-2 border-t pt-3">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium">AgentKernel integration</span>
                      {!agent.integration_supported ? (
                        <Badge variant="outline">Not managed</Badge>
                      ) : agent.integration_project_installed || agent.integration_global_installed ? (
                        <Badge variant="success">Connected</Badge>
                      ) : (
                        <Badge variant="secondary">Not connected</Badge>
                      )}
                    </div>
                    {agent.integration_supported && (
                      <div className="flex flex-wrap gap-2">
                        {agent.integration_project_installed ? (
                          <Badge variant="outline">Project</Badge>
                        ) : (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={busy}
                            onClick={() =>
                              installMutation.mutate({ agent, scope: "project", confirm: false })
                            }
                          >
                            {busy ? (
                              <Loader2 className="mr-2 h-3 w-3 animate-spin" />
                            ) : (
                              <PlugZap className="mr-2 h-3 w-3" />
                            )}
                            Set up project
                          </Button>
                        )}
                        {agent.integration_global_supported &&
                          (agent.integration_global_installed ? (
                            <Badge variant="outline">Global</Badge>
                          ) : (
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={busy}
                              onClick={() =>
                                installMutation.mutate({ agent, scope: "global", confirm: false })
                              }
                            >
                              Set up globally
                            </Button>
                          ))}
                      </div>
                    )}
                  </section>
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}

      <Dialog open={preview !== null} onOpenChange={(open) => !open && setPreview(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Connect {preview?.agent.display_name}</DialogTitle>
            <DialogDescription>
              AgentKernel will merge its integration into the configured server&apos;s{" "}
              {preview?.result.scope} settings. Existing unrelated settings are preserved.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <p className="text-sm font-medium">Files to create or update</p>
            <ul className="space-y-1 rounded-md bg-muted p-3 text-xs font-mono">
              {preview?.result.files.map((file) => (
                <li key={file} className="break-all">{file}</li>
              ))}
            </ul>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPreview(null)}>Cancel</Button>
            <Button
              disabled={installMutation.isPending || !preview}
              onClick={() =>
                preview && installMutation.mutate({
                  agent: preview.agent,
                  scope: preview.result.scope,
                  confirm: true,
                })
              }
            >
              {installMutation.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Confirm setup
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
