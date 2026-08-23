import { useEffect, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Shield,
  RefreshCw,
  Play,
  CheckCircle2,
  XCircle,
  Save,
} from "lucide-react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { PolicyActivationRequest, PolicyCheckResult } from "@/lib/types";

const ACTIONS = [
  "Run",
  "Exec",
  "Create",
  "Attach",
  "Mount",
  "Network",
  "PortMap",
  "SSH",
];

export function Policy() {
  const queryClient = useQueryClient();
  const [checkAction, setCheckAction] = useState("Run");
  const [checkSandbox, setCheckSandbox] = useState("");
  const [checkResult, setCheckResult] = useState<PolicyCheckResult | null>(
    null
  );
  const [editorConfig, setEditorConfig] = useState("");
  const [editorPolicy, setEditorPolicy] = useState("");

  // --- Queries ---

  const {
    data: policyStatus,
    isLoading,
    error,
    refetch: refetchStatus,
  } = useQuery({
    queryKey: ["policy-status"],
    queryFn: () => api.getPolicyStatus(),
    retry: false,
  });

  const materialQuery = useQuery({
    queryKey: ["local-policy-material"],
    queryFn: () => api.getLocalPolicyMaterial(),
    retry: false,
  });
  const materialError = materialQuery.error
    ? materialQuery.error instanceof Error
      ? materialQuery.error.message
      : String(materialQuery.error)
    : null;

  useEffect(() => {
    if (materialQuery.data) {
      setEditorConfig(materialQuery.data.config);
      setEditorPolicy(materialQuery.data.policy);
    }
  }, [materialQuery.data]);

  // --- Mutations ---

  const reloadMutation = useMutation({
    mutationFn: () => api.reloadPolicy(),
    onMutate: () => ({ toastId: toast("Reloading policies...") }),
    onSuccess: (data, _vars, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          data.reloaded
            ? `Policies reloaded (v${data.version})`
            : "No policy server configured",
          data.reloaded ? "success" : "error"
        );
      queryClient.invalidateQueries({ queryKey: ["policy-status"] });
    },
    onError: (err, _vars, context) => {
      if (context?.toastId)
        toast.update(
          context.toastId,
          err instanceof Error ? err.message : String(err),
          "error"
        );
    },
  });

  const activateMutation = useMutation({
    mutationFn: (request: PolicyActivationRequest) =>
      api.activateLocalPolicy(request),
    onSuccess: (data) => {
      toast(
        `Local policy activated${data.config_backup ? " (previous files backed up)" : ""}`,
        "success"
      );
      queryClient.invalidateQueries({ queryKey: ["policy-status"] });
      queryClient.invalidateQueries({ queryKey: ["local-policy-material"] });
    },
    onError: (err) => {
      toast(err instanceof Error ? err.message : String(err), "error");
    },
  });

  const checkMutation = useMutation({
    mutationFn: ({
      action,
      sandbox,
    }: {
      action: string;
      sandbox: string;
    }) => api.checkPolicy(action, sandbox),
    onSuccess: (data) => {
      setCheckResult(data);
    },
    onError: (err) => {
      toast(err instanceof Error ? err.message : String(err));
    },
  });

  // --- Loading state ---

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[200px] rounded-lg" />
        <Skeleton className="h-[200px] rounded-lg" />
      </div>
    );
  }

  // --- Error state (enterprise feature not compiled) ---

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
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Policy</h1>
          <p className="text-muted-foreground">
            Cedar policy engine &mdash; authorization for sandbox operations
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => refetchStatus()}
        >
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {/* Section 1: Policy Engine Status */}
      {policyStatus && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle className="flex items-center gap-2">
                  <Shield className="h-5 w-5" />
                  Policy Engine
                </CardTitle>
                <CardDescription>
                  Current policy engine configuration
                </CardDescription>
              </div>
              <div className="flex items-center gap-2">
                <Badge
                  variant={policyStatus.enforcing && policyStatus.healthy ? "success" : "secondary"}
                >
                  {policyStatus.enforcing && policyStatus.healthy ? "Enforcing" : "Not enforcing"}
                </Badge>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => reloadMutation.mutate()}
                  disabled={reloadMutation.isPending}
                >
                  <RefreshCw
                    className={`mr-2 h-4 w-4 ${reloadMutation.isPending ? "animate-spin" : ""}`}
                  />
                  Reload
                </Button>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-2 border-b pb-3 text-sm sm:grid-cols-5">
                {([
                  ["Compiled", policyStatus.compiled],
                  ["Configured", policyStatus.configured],
                  ["Active", policyStatus.active],
                  ["Enforcing", policyStatus.enforcing],
                  ["Healthy", policyStatus.healthy],
                ] as const).map(([label, value]) => (
                  <div key={label} className="flex items-center gap-1.5">
                    {value ? (
                      <CheckCircle2 className="h-3.5 w-3.5 text-green-600" />
                    ) : (
                      <XCircle className="h-3.5 w-3.5 text-muted-foreground" />
                    )}
                    <span>{label}</span>
                  </div>
                ))}
              </div>
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Version</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.version}
                </span>
              </div>
              <div className="flex items-center justify-between border-b pb-2">
                <span className="text-sm font-medium">Policy Source</span>
                <span className="font-mono text-sm text-muted-foreground">
                  {policyStatus.source ?? "none"}
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
              {policyStatus.config_path && (
                <div className="flex items-center justify-between border-t pt-2">
                  <span className="text-sm font-medium">Configuration Path</span>
                  <span className="max-w-[70%] truncate font-mono text-sm text-muted-foreground" title={policyStatus.config_path}>
                    {policyStatus.config_path}
                  </span>
                </div>
              )}
              {policyStatus.initialization_error && (
                <p className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
                  Policy initialization failed: {policyStatus.initialization_error}
                </p>
              )}
              {policyStatus.source === "default_permit_all" && (
                <p className="rounded-md border border-yellow-500/40 bg-yellow-500/5 p-3 text-sm text-yellow-700 dark:text-yellow-300">
                  Permit-all compatibility fallback: this is not meaningful policy enforcement. Configure a local Cedar file or a managed policy server.
                </p>
              )}
              {policyStatus.admin_guidance && (
                <p className="rounded-md border p-3 text-sm text-muted-foreground">
                  {policyStatus.admin_guidance}
                </p>
              )}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Section 2: Local policy editor */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Save className="h-5 w-5" />
            Local policy activation
          </CardTitle>
          <CardDescription>
            Edit the app-owned TOML and Cedar files, then atomically restart the local sidecar.
            Remote and separately managed servers are read-only.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {materialQuery.error ? (
            <p className="rounded-md border border-yellow-500/40 bg-yellow-500/5 p-3 text-sm text-yellow-700 dark:text-yellow-300">
              {materialError}
              {materialError?.includes("read-only") &&
                " Ask the server administrator to update policy."}
            </p>
          ) : (
            <>
              <div className="space-y-2">
                <Label htmlFor="policy-config">AgentKernel TOML</Label>
                <textarea
                  id="policy-config"
                  className="min-h-48 w-full rounded-md border bg-background px-3 py-2 font-mono text-sm"
                  value={editorConfig}
                  onChange={(event) => setEditorConfig(event.target.value)}
                  disabled={materialQuery.isLoading || activateMutation.isPending}
                  spellCheck={false}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="policy-cedar">Cedar policy</Label>
                <textarea
                  id="policy-cedar"
                  className="min-h-48 w-full rounded-md border bg-background px-3 py-2 font-mono text-sm"
                  value={editorPolicy}
                  onChange={(event) => setEditorPolicy(event.target.value)}
                  disabled={materialQuery.isLoading || activateMutation.isPending}
                  spellCheck={false}
                />
              </div>
              <div className="flex items-center justify-between gap-4">
                <p className="text-xs text-muted-foreground">
                  Validation happens before either file changes. A failed startup restores both files and the previous sidecar.
                </p>
                <Button
                  onClick={() =>
                    activateMutation.mutate({
                      config: editorConfig,
                      policy: editorPolicy,
                    })
                  }
                  disabled={
                    materialQuery.isLoading ||
                    activateMutation.isPending ||
                    !editorConfig.trim() ||
                    !editorPolicy.trim()
                  }
                >
                  <Save className="mr-2 h-4 w-4" />
                  {activateMutation.isPending ? "Activating..." : "Activate locally"}
                </Button>
              </div>
            </>
          )}
        </CardContent>
      </Card>

      {/* Section 3: Policy Check Tester */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Play className="h-5 w-5" />
            Policy Check
          </CardTitle>
          <CardDescription>
            Test whether an action would be permitted by the policy engine
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-end gap-4">
            <div className="w-48 space-y-2">
              <Label>Action</Label>
              <Select value={checkAction} onValueChange={setCheckAction}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {ACTIONS.map((a) => (
                    <SelectItem key={a} value={a}>
                      {a}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex-1 space-y-2">
              <Label>Sandbox Name</Label>
              <Input
                placeholder="my-sandbox"
                value={checkSandbox}
                onChange={(e) => setCheckSandbox(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && checkSandbox.trim()) {
                    checkMutation.mutate({
                      action: checkAction.toLowerCase(),
                      sandbox: checkSandbox.trim(),
                    });
                  }
                }}
              />
            </div>
            <Button
              onClick={() =>
                checkMutation.mutate({
                  action: checkAction.toLowerCase(),
                  sandbox: checkSandbox.trim() || "test",
                })
              }
              disabled={checkMutation.isPending}
            >
              {checkMutation.isPending ? "Checking..." : "Check"}
            </Button>
          </div>

          {/* Result display */}
          {checkResult && (
            <div className="rounded-lg border p-4 space-y-3">
              <div className="flex items-center gap-3">
                {checkResult.decision === "permit" ? (
                  <CheckCircle2 className="h-5 w-5 text-green-600 dark:text-green-400" />
                ) : (
                  <XCircle className="h-5 w-5 text-red-600 dark:text-red-400" />
                )}
                <Badge
                  variant={
                    checkResult.decision === "permit"
                      ? "success"
                      : "destructive"
                  }
                >
                  {checkResult.decision.toUpperCase()}
                </Badge>
                <span className="text-sm text-muted-foreground">
                  {checkResult.evaluation_time_us}&#181;s
                </span>
              </div>
              <p className="text-sm text-muted-foreground">
                {checkResult.reason}
              </p>
              {checkResult.matched_policies.length > 0 && (
                <div className="space-y-1">
                  <span className="text-xs font-medium uppercase text-muted-foreground">
                    Matched Policies
                  </span>
                  <div className="flex flex-wrap gap-1">
                    {checkResult.matched_policies.map((p) => (
                      <Badge key={p} variant="outline" className="font-mono text-xs">
                        {p}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>

    </div>
  );
}
