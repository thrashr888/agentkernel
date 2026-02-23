import { useState, useRef, useEffect } from "react";
import { useParams, Link, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  Trash2,
  Play,
  Square,
  Clock,
  Copy,
  Check,
  Terminal,
  Loader2,
  X,
  Download,
  Brain,
  Shield,
  ExternalLink,
  Pencil,
  Tag,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSandbox } from "@/lib/hooks/use-sandbox";
import { useExec } from "@/lib/hooks/use-exec";
import { useLlmUsageBySandbox } from "@/lib/hooks/use-llm-usage";
import { api } from "@/lib/api";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import { FileBrowser } from "@/components/sandbox/file-browser";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { toast } from "@/components/ui/use-toast";
import { formatDate } from "@/lib/utils";
import type { RunOutput, DetachedCommand, AuditLogEntry, LlmUsageEntry } from "@/lib/types";

interface CommandEntry {
  command: string;
  output: RunOutput;
  /** When set, this entry shows streaming output from a detached job */
  detachedJobId?: string;
}

interface ParsedPort {
  hostPort: string;
  containerPort: string;
  protocol: string;
}

function parsePortBinding(binding: string): ParsedPort {
  const [portsPart, protoPart] = binding.split("/");
  const protocol = (protoPart || "tcp").toLowerCase();
  const pieces = portsPart.split(":");
  if (pieces.length >= 2) {
    return {
      hostPort: pieces[0] || "auto",
      containerPort: pieces[1] || "unknown",
      protocol,
    };
  }
  return {
    hostPort: "auto",
    containerPort: pieces[0] || "unknown",
    protocol,
  };
}

export function SandboxDetail() {
  const { name } = useParams<{ name: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: sandbox, isLoading, error } = useSandbox(name ?? "");
  const { data: llmUsage } = useLlmUsageBySandbox(name ?? "");
  const execMutation = useExec();

  const [commandInput, setCommandInput] = useState("");
  const [history, setHistory] = useState<CommandEntry[]>([]);
  const [extendSeconds, setExtendSeconds] = useState(300);
  const [extendDialogOpen, setExtendDialogOpen] = useState(false);
  const [resizeDialogOpen, setResizeDialogOpen] = useState(false);
  const [resizeVcpus, setResizeVcpus] = useState(sandbox?.vcpus ?? 2);
  const [resizeMemory, setResizeMemory] = useState(sandbox?.memory_mb ?? 512);
  const [selectedJob, setSelectedJob] = useState<string | null>(null);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const [runInBackground, setRunInBackground] = useState(false);
  const [activeDetachedJobId, setActiveDetachedJobId] = useState<string | null>(null);
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [editLabels, setEditLabels] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const outputRef = useRef<HTMLDivElement>(null);
  const proxySecretCount = sandbox?.secret_mappings
    ? Object.keys(sandbox.secret_mappings).length
    : 0;
  const secretFileCount = sandbox?.secret_files?.length ?? 0;

  function copyToClipboard(text: string, field: string) {
    navigator.clipboard.writeText(text);
    setCopiedField(field);
    setTimeout(() => setCopiedField(null), 2000);
  }

  const startMutation = useMutation({
    mutationFn: () => api.startSandbox(name ?? ""),
    onMutate: () => {
      return { toastId: toast("Starting sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox started!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const stopMutation = useMutation({
    mutationFn: () => api.stopSandbox(name ?? ""),
    onMutate: () => {
      return { toastId: toast("Stopping sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox stopped!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const removeMutation = useMutation({
    mutationFn: (sandboxName: string) => api.removeSandbox(sandboxName),
    onMutate: () => {
      return { toastId: toast("Removing sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox removed!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      navigate("/sandboxes");
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const openTerminalMutation = useMutation({
    mutationFn: () => api.openTerminal(name ?? ""),
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

  const exportMutation = useMutation({
    mutationFn: () => api.exportSandbox(name ?? ""),
    onMutate: () => {
      return { toastId: toast("Exporting sandbox...") };
    },
    onSuccess: (path, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, `Exported to ${path}`, "success");
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const extendMutation = useMutation({
    mutationFn: ({ sandboxName, seconds }: { sandboxName: string; seconds: number }) =>
      api.extendTtl(sandboxName, `${seconds}s`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      setExtendDialogOpen(false);
      toast.success("TTL extended");
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const resizeMutation = useMutation({
    mutationFn: ({
      sandboxName,
      vcpus,
      memoryMb,
    }: {
      sandboxName: string;
      vcpus: number;
      memoryMb: number;
    }) => api.resizeSandbox(sandboxName, vcpus, memoryMb),
    onMutate: () => {
      return { toastId: toast("Resizing sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId)
        toast.update(context.toastId, "Sandbox resized!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      setResizeDialogOpen(false);
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

  const updateMutation = useMutation({
    mutationFn: ({
      sandboxName,
      labels,
      description,
    }: {
      sandboxName: string;
      labels: Record<string, string>;
      description?: string;
    }) => api.updateSandbox(sandboxName, labels, description),
    onMutate: () => {
      return { toastId: toast("Updating sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId)
        toast.update(context.toastId, "Sandbox updated!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      setEditDialogOpen(false);
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

  const { data: sandboxLogs } = useQuery({
    queryKey: ["sandbox-logs", name],
    queryFn: () => api.getSandboxLogs(name ?? ""),
    enabled: !!name,
    refetchInterval: 5000,
  });

  const { data: detachedJobs } = useQuery({
    queryKey: ["detached", name],
    queryFn: () => api.listDetached(name ?? ""),
    enabled: !!name,
    refetchInterval: 5000,
  });

  const { data: jobLogs } = useQuery({
    queryKey: ["detached-logs", name, selectedJob],
    queryFn: () => api.getDetachedLogs(name ?? "", selectedJob ?? ""),
    enabled: !!name && !!selectedJob,
    refetchInterval: 3000,
  });

  const killJobMutation = useMutation({
    mutationFn: (cmdId: string) => api.killDetached(name ?? "", cmdId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["detached", name] });
      toast.success("Job killed");
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const execDetachedMutation = useMutation({
    mutationFn: ({ sandboxName, command }: { sandboxName: string; command: string[] }) =>
      api.execDetached(sandboxName, command),
    onSuccess: (job) => {
      setActiveDetachedJobId(job.id);
      setHistory((prev) => [
        ...prev,
        {
          command: `[background] ${job.command.join(" ")}`,
          output: { output: "" },
          detachedJobId: job.id,
        },
      ]);
      queryClient.invalidateQueries({ queryKey: ["detached", name] });
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  // Poll active detached job output and update the corresponding history entry
  const { data: activeJobLogs } = useQuery({
    queryKey: ["exec-detached-logs", name, activeDetachedJobId],
    queryFn: () => api.getDetachedLogs(name ?? "", activeDetachedJobId ?? ""),
    enabled: !!name && !!activeDetachedJobId,
    refetchInterval: 1000,
  });

  // Also poll the job status to know when it finishes
  const { data: activeJobStatus } = useQuery({
    queryKey: ["exec-detached-status", name, activeDetachedJobId],
    queryFn: async () => {
      const jobs = await api.listDetached(name ?? "");
      return jobs.find((j) => j.id === activeDetachedJobId) ?? null;
    },
    enabled: !!name && !!activeDetachedJobId,
    refetchInterval: 1000,
  });

  // Update history entry with latest logs from the active detached job
  useEffect(() => {
    if (!activeDetachedJobId || !activeJobLogs) return;
    const combined =
      (activeJobLogs.stdout || "") +
      (activeJobLogs.stderr ? `\n${activeJobLogs.stderr}` : "");
    setHistory((prev) =>
      prev.map((entry) =>
        entry.detachedJobId === activeDetachedJobId
          ? { ...entry, output: { output: combined } }
          : entry
      )
    );
  }, [activeDetachedJobId, activeJobLogs]);

  // Stop polling when the job finishes
  useEffect(() => {
    if (activeJobStatus && activeJobStatus.status !== "running") {
      setActiveDetachedJobId(null);
      queryClient.invalidateQueries({ queryKey: ["detached", name] });
    }
  }, [activeJobStatus, name, queryClient]);

  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [history]);

  const isExecBusy =
    execMutation.isPending ||
    execDetachedMutation.isPending ||
    !!activeDetachedJobId;

  function handleExec() {
    if (!commandInput.trim() || !name) return;
    const cmd = commandInput.trim();
    setCommandInput("");

    if (runInBackground) {
      execDetachedMutation.mutate({
        sandboxName: name,
        command: ["sh", "-c", cmd],
      });
    } else {
      execMutation.mutate(
        {
          name,
          command: ["sh", "-c", cmd],
        },
        {
          onSuccess: (output) => {
            setHistory((prev) => [...prev, { command: cmd, output }]);
          },
          onError: (err: unknown) => {
            toast.error(err instanceof Error ? err.message : String(err));
          },
        }
      );
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-64 rounded-lg" />
      </div>
    );
  }

  if (error || !sandbox) {
    return (
      <div className="space-y-6">
        <Link
          to="/sandboxes"
          className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Sandboxes
        </Link>
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              {error
                ? `Failed to load sandbox: ${error.message}`
                : `Sandbox "${name}" not found.`}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const isRunning = sandbox.status.toLowerCase() === "running";

  return (
    <div className="space-y-0">
      {/* Breadcrumb */}
      <div className="mb-2">
        <Link to="/sandboxes" className="text-sm text-blue-500 hover:underline">
          Sandboxes
        </Link>
        <span className="text-sm text-muted-foreground"> / {sandbox.name}</span>
      </div>

      {/* Header — Docker Desktop style */}
      <div className="flex items-start justify-between pb-4">
        <div className="flex items-start gap-3">
          <Link
            to="/sandboxes"
            className="mt-1.5 text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="h-5 w-5" />
          </Link>
          <div>
            <h1 className="text-2xl font-semibold leading-tight">
              {sandbox.name}
            </h1>
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
              <span className="font-mono">{sandbox.backend}</span>
              {sandbox.uuid && (
                <>
                  <span className="text-border">|</span>
                  <button
                    type="button"
                    className="group inline-flex items-center gap-1 font-mono hover:text-foreground"
                    onClick={() => copyToClipboard(sandbox.uuid!, "uuid")}
                    title="Copy sandbox UUID"
                  >
                    {sandbox.uuid}
                    {copiedField === "uuid" ? (
                      <Check className="h-3 w-3 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                    )}
                  </button>
                </>
              )}
              {sandbox.image && (
                <>
                  <span className="text-border">|</span>
                  <button
                    type="button"
                    className="group inline-flex items-center gap-1 font-mono text-blue-500 hover:underline"
                    onClick={() => copyToClipboard(sandbox.image!, "image")}
                    title="Copy image name"
                  >
                    {sandbox.image}
                    {copiedField === "image" ? (
                      <Check className="h-3 w-3 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                    )}
                  </button>
                </>
              )}
              {sandbox.ip && (
                <>
                  <span className="text-border">|</span>
                  <button
                    type="button"
                    className="group inline-flex items-center gap-1 font-mono hover:text-foreground"
                    onClick={() => copyToClipboard(sandbox.ip!, "ip")}
                    title="Copy IP"
                  >
                    {sandbox.ip}
                    {copiedField === "ip" ? (
                      <Check className="h-3 w-3 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                    )}
                  </button>
                </>
              )}
              {sandbox.ports && sandbox.ports.length > 0 && (
                <>
                  <span className="text-border">|</span>
                  <span className="font-mono">{sandbox.ports.join(", ")}</span>
                </>
              )}
            </div>
          </div>
        </div>

        {/* Status + action icon buttons */}
        <div className="flex items-center gap-3 shrink-0">
          <div className="text-right mr-1">
            <SandboxStatusBadge status={sandbox.status} />
          </div>
          {isRunning ? (
            <Button
              variant="outline"
              size="icon"
              onClick={() => stopMutation.mutate()}
              disabled={stopMutation.isPending}
              title="Stop"
            >
              <Square className="h-4 w-4" />
            </Button>
          ) : (
            <Button
              variant="outline"
              size="icon"
              onClick={() => startMutation.mutate()}
              disabled={startMutation.isPending}
              title="Start"
            >
              <Play className="h-4 w-4" />
            </Button>
          )}
          {isRunning && (
            <Button
              variant="outline"
              size="icon"
              onClick={() => openTerminalMutation.mutate()}
              disabled={openTerminalMutation.isPending}
              title="Open terminal"
            >
              <Terminal className="h-4 w-4" />
            </Button>
          )}
          {isRunning && (
            <Button
              variant="outline"
              size="icon"
              onClick={() => exportMutation.mutate()}
              disabled={exportMutation.isPending}
              title="Export"
            >
              <Download className="h-4 w-4" />
            </Button>
          )}
          <Button
            variant="outline"
            size="icon"
            className="text-destructive hover:bg-destructive hover:text-destructive-foreground"
            onClick={() => removeMutation.mutate(sandbox.name)}
            disabled={removeMutation.isPending}
            title="Remove"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Open in external terminal link */}
      <div className="flex items-center gap-4 pb-2 text-sm">
        <button
          type="button"
          className="group inline-flex items-center gap-1 font-mono text-xs text-muted-foreground hover:text-foreground"
          onClick={() =>
            copyToClipboard(`agentkernel attach ${sandbox.name}`, "connect")
          }
          title="Copy connect command"
        >
          <Terminal className="h-3 w-3" />
          agentkernel attach {sandbox.name}
          {copiedField === "connect" ? (
            <Check className="h-3 w-3 text-green-500" />
          ) : (
            <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
          )}
        </button>
        {isRunning && (
          <button
            type="button"
            className="inline-flex items-center gap-1 text-xs text-blue-500 hover:underline"
            onClick={() => openTerminalMutation.mutate()}
          >
            Open in external terminal
            <ExternalLink className="h-3 w-3" />
          </button>
        )}
      </div>

      <Tabs defaultValue="inspect">
        <TabsList>
          <TabsTrigger value="inspect">Inspect</TabsTrigger>
          <TabsTrigger value="secrets">Secrets</TabsTrigger>
          <TabsTrigger value="ports">Ports</TabsTrigger>
          <TabsTrigger value="exec">Exec</TabsTrigger>
          <TabsTrigger value="files">Files</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
        </TabsList>

        <TabsContent value="inspect" className="pt-2">
          {/* Flat inspect-style key-value table */}
          <div className="rounded-md border">
            <table className="w-full text-sm">
              <tbody>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground w-40">
                    Status
                  </td>
                  <td className="px-4 py-2.5">
                    <SandboxStatusBadge status={sandbox.status} />
                  </td>
                </tr>
                {sandbox.description && (
                  <tr className="border-b">
                    <td className="px-4 py-2.5 text-muted-foreground">
                      Description
                    </td>
                    <td className="px-4 py-2.5">
                      {sandbox.description}
                    </td>
                  </tr>
                )}
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">Backend</td>
                  <td className="px-4 py-2.5">
                    <button
                      type="button"
                      className="group inline-flex items-center gap-1.5 font-mono hover:text-foreground"
                      onClick={() =>
                        copyToClipboard(sandbox.backend, "backend-inspect")
                      }
                      title="Copy"
                    >
                      {sandbox.backend}
                      {copiedField === "backend-inspect" ? (
                        <Check className="h-3 w-3 text-green-500" />
                      ) : (
                        <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                      )}
                    </button>
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">UUID</td>
                  <td className="px-4 py-2.5">
                    {sandbox.uuid ? (
                      <button
                        type="button"
                        className="group inline-flex items-center gap-1.5 font-mono hover:text-foreground"
                        onClick={() =>
                          copyToClipboard(sandbox.uuid!, "uuid-inspect")
                        }
                        title="Copy UUID"
                      >
                        {sandbox.uuid}
                        {copiedField === "uuid-inspect" ? (
                          <Check className="h-3 w-3 text-green-500" />
                        ) : (
                          <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                        )}
                      </button>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">Image</td>
                  <td className="px-4 py-2.5">
                    {sandbox.image ? (
                      <button
                        type="button"
                        className="group inline-flex items-center gap-1.5 font-mono hover:text-foreground"
                        onClick={() =>
                          copyToClipboard(sandbox.image!, "image-inspect")
                        }
                        title="Copy"
                      >
                        {sandbox.image}
                        {copiedField === "image-inspect" ? (
                          <Check className="h-3 w-3 text-green-500" />
                        ) : (
                          <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                        )}
                      </button>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">vCPUs</td>
                  <td className="px-4 py-2.5 font-mono">
                    {sandbox.vcpus ?? "—"}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">Memory</td>
                  <td className="px-4 py-2.5 font-mono">
                    {sandbox.memory_mb ? `${sandbox.memory_mb} MB` : "—"}
                  </td>
                </tr>
                {sandbox.ip && (
                  <tr className="border-b">
                    <td className="px-4 py-2.5 text-muted-foreground">
                      IP Address
                    </td>
                    <td className="px-4 py-2.5">
                      <button
                        type="button"
                        className="group inline-flex items-center gap-1.5 font-mono hover:text-foreground"
                        onClick={() =>
                          copyToClipboard(sandbox.ip!, "ip-inspect")
                        }
                        title="Copy"
                      >
                        {sandbox.ip}
                        {copiedField === "ip-inspect" ? (
                          <Check className="h-3 w-3 text-green-500" />
                        ) : (
                          <Copy className="h-3 w-3 opacity-0 group-hover:opacity-70" />
                        )}
                      </button>
                    </td>
                  </tr>
                )}
                {sandbox.ports && sandbox.ports.length > 0 && (
                  <tr className="border-b">
                    <td className="px-4 py-2.5 text-muted-foreground">Ports</td>
                    <td className="px-4 py-2.5 font-mono">
                      {sandbox.ports.join(", ")}
                    </td>
                  </tr>
                )}
                {sandbox.created_at && (
                  <tr className="border-b">
                    <td className="px-4 py-2.5 text-muted-foreground">
                      Created
                    </td>
                    <td className="px-4 py-2.5">
                      {formatDate(sandbox.created_at)}
                    </td>
                  </tr>
                )}
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">
                    Template
                  </td>
                  <td className="px-4 py-2.5">
                    {sandbox.created_from_template ? (
                      <span className="font-mono">
                        {sandbox.created_from_template}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">
                    Template Notes
                  </td>
                  <td className="px-4 py-2.5">
                    {sandbox.template_help_text ? (
                      <pre className="whitespace-pre-wrap text-xs text-muted-foreground">
                        {sandbox.template_help_text}
                      </pre>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">Secrets</td>
                  <td className="px-4 py-2.5">
                    {proxySecretCount > 0 || secretFileCount > 0 ? (
                      <span className="font-mono">
                        {proxySecretCount} bindings, {secretFileCount} files
                      </span>
                    ) : (
                      <span className="text-muted-foreground">None</span>
                    )}
                  </td>
                </tr>
                <tr className="border-b">
                  <td className="px-4 py-2.5 text-muted-foreground">Labels</td>
                  <td className="px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      {sandbox.labels && Object.keys(sandbox.labels).length > 0 ? (
                        <div className="flex flex-wrap items-center gap-1.5">
                          {Object.entries(sandbox.labels).map(([key, value]) => (
                            <span
                              key={key}
                              className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-0.5 text-xs font-medium"
                            >
                              <Tag className="h-3 w-3 text-muted-foreground" />
                              <span className="text-muted-foreground">{key}:</span>
                              <span>{value}</span>
                            </span>
                          ))}
                        </div>
                      ) : (
                        <span className="text-muted-foreground">None</span>
                      )}
                    </div>
                  </td>
                </tr>
                <tr>
                  <td className="px-4 py-2.5 text-muted-foreground">Actions</td>
                  <td className="px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      <Dialog
                        open={editDialogOpen}
                        onOpenChange={(open) => {
                          setEditDialogOpen(open);
                          if (open) {
                            const labels = sandbox.labels ?? {};
                            setEditLabels(
                              Object.entries(labels)
                                .map(([k, v]) => `${k}=${v}`)
                                .join(", ")
                            );
                            setEditDescription(sandbox.description ?? "");
                          }
                        }}
                      >
                        <DialogTrigger asChild>
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-7 text-xs"
                          >
                            <Pencil className="mr-1 h-3 w-3" />
                            Edit
                          </Button>
                        </DialogTrigger>
                        <DialogContent>
                          <DialogHeader>
                            <DialogTitle>Edit Sandbox</DialogTitle>
                            <DialogDescription>
                              Update sandbox metadata. Changes take effect
                              immediately.
                            </DialogDescription>
                          </DialogHeader>
                          <div className="grid gap-4 py-4">
                            <div className="grid gap-2">
                              <Label htmlFor="edit-description">Description</Label>
                              <Input
                                id="edit-description"
                                placeholder="What this sandbox is for"
                                value={editDescription}
                                onChange={(e) => setEditDescription(e.target.value)}
                              />
                            </div>
                            <div className="grid gap-2">
                              <Label htmlFor="edit-labels">
                                Labels{" "}
                                <span className="text-xs text-muted-foreground font-normal">
                                  (comma-separated key=value)
                                </span>
                              </Label>
                              <Input
                                id="edit-labels"
                                placeholder="env=prod, team=platform"
                                value={editLabels}
                                onChange={(e) => setEditLabels(e.target.value)}
                              />
                            </div>
                          </div>
                          {!!updateMutation.error && (
                            <p className="text-sm text-destructive">
                              {updateMutation.error instanceof Error
                                ? updateMutation.error.message
                                : String(updateMutation.error)}
                            </p>
                          )}
                          <DialogFooter>
                            <Button
                              variant="outline"
                              onClick={() => setEditDialogOpen(false)}
                            >
                              Cancel
                            </Button>
                            <Button
                              onClick={() => {
                                const labels: Record<string, string> = {};
                                if (editLabels.trim()) {
                                  for (const part of editLabels.split(",")) {
                                    const eqIdx = part.indexOf("=");
                                    if (eqIdx > 0) {
                                      labels[part.slice(0, eqIdx).trim()] =
                                        part.slice(eqIdx + 1).trim();
                                    }
                                  }
                                }
                                updateMutation.mutate({
                                  sandboxName: sandbox.name,
                                  labels,
                                  description: editDescription || undefined,
                                });
                              }}
                              disabled={updateMutation.isPending}
                            >
                              {updateMutation.isPending ? "Saving..." : "Save"}
                            </Button>
                          </DialogFooter>
                        </DialogContent>
                      </Dialog>

                      <Dialog
                        open={resizeDialogOpen}
                        onOpenChange={(open) => {
                          setResizeDialogOpen(open);
                          if (open) {
                            setResizeVcpus(sandbox.vcpus ?? 2);
                            setResizeMemory(sandbox.memory_mb ?? 512);
                          }
                        }}
                      >
                        <DialogTrigger asChild>
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-7 text-xs"
                          >
                            Resize
                          </Button>
                        </DialogTrigger>
                        <DialogContent>
                          <DialogHeader>
                            <DialogTitle>Resize Sandbox</DialogTitle>
                            <DialogDescription>
                              Change CPU and memory allocation. The sandbox will
                              be stopped and recreated with the new resources.
                            </DialogDescription>
                          </DialogHeader>
                          <div className="grid gap-4 py-4">
                            <div className="grid gap-2">
                              <Label htmlFor="resize-vcpus">vCPUs</Label>
                              <Input
                                id="resize-vcpus"
                                type="number"
                                inputMode="numeric"
                                min={1}
                                max={16}
                                value={resizeVcpus}
                                onChange={(e) =>
                                  setResizeVcpus(Number(e.target.value))
                                }
                              />
                            </div>
                            <div className="grid gap-2">
                              <Label htmlFor="resize-memory">Memory (MB)</Label>
                              <Input
                                id="resize-memory"
                                type="number"
                                inputMode="numeric"
                                min={128}
                                step={128}
                                value={resizeMemory}
                                onChange={(e) =>
                                  setResizeMemory(Number(e.target.value))
                                }
                              />
                            </div>
                          </div>
                          {!!resizeMutation.error && (
                            <p className="text-sm text-destructive">
                              {resizeMutation.error instanceof Error
                                ? resizeMutation.error.message
                                : String(resizeMutation.error)}
                            </p>
                          )}
                          <DialogFooter>
                            <Button
                              variant="outline"
                              onClick={() => setResizeDialogOpen(false)}
                            >
                              Cancel
                            </Button>
                            <Button
                              onClick={() =>
                                resizeMutation.mutate({
                                  sandboxName: sandbox.name,
                                  vcpus: resizeVcpus,
                                  memoryMb: resizeMemory,
                                })
                              }
                              disabled={resizeMutation.isPending}
                            >
                              {resizeMutation.isPending
                                ? "Resizing..."
                                : "Resize"}
                            </Button>
                          </DialogFooter>
                        </DialogContent>
                      </Dialog>

                      <Dialog
                        open={extendDialogOpen}
                        onOpenChange={setExtendDialogOpen}
                      >
                        <DialogTrigger asChild>
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-7 text-xs"
                          >
                            <Clock className="mr-1 h-3 w-3" />
                            Extend TTL
                          </Button>
                        </DialogTrigger>
                        <DialogContent>
                          <DialogHeader>
                            <DialogTitle>Extend TTL</DialogTitle>
                            <DialogDescription>
                              Add more time before this sandbox expires.
                            </DialogDescription>
                          </DialogHeader>
                          <div className="grid gap-2 py-4">
                            <Label htmlFor="extend-seconds">
                              Seconds to add
                            </Label>
                            <Input
                              id="extend-seconds"
                              type="number"
                              inputMode="numeric"
                              min={60}
                              step={60}
                              value={extendSeconds}
                              onChange={(e) =>
                                setExtendSeconds(Number(e.target.value))
                              }
                            />
                          </div>
                          {!!extendMutation.error && (
                            <p className="text-sm text-destructive">
                              {extendMutation.error instanceof Error
                                ? extendMutation.error.message
                                : String(extendMutation.error)}
                            </p>
                          )}
                          <DialogFooter>
                            <Button
                              variant="outline"
                              onClick={() => setExtendDialogOpen(false)}
                            >
                              Cancel
                            </Button>
                            <Button
                              onClick={() =>
                                extendMutation.mutate({
                                  sandboxName: sandbox.name,
                                  seconds: extendSeconds,
                                })
                              }
                              disabled={extendMutation.isPending}
                            >
                              {extendMutation.isPending
                                ? "Extending..."
                                : "Extend"}
                            </Button>
                          </DialogFooter>
                        </DialogContent>
                      </Dialog>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          {/* LLM Usage */}
          {llmUsage && llmUsage.length > 0 && (
            <div className="mt-4">
              <h3 className="text-sm font-medium mb-2 flex items-center gap-2">
                <Brain className="h-4 w-4 text-violet-500" />
                LLM Usage
              </h3>
              <div className="rounded-md border">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b bg-muted/50">
                      <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                        Model
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                        Requests
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                        Input
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                        Output
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                        Total
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {llmUsage.map((entry: LlmUsageEntry, i: number) => (
                      <tr
                        key={`${entry.provider}-${entry.model}`}
                        className={i < llmUsage.length - 1 ? "border-b" : ""}
                      >
                        <td className="px-4 py-2">
                          <span className="font-medium">{entry.model}</span>
                          <span className="ml-2 text-xs text-muted-foreground">
                            {entry.provider}
                          </span>
                        </td>
                        <td className="px-4 py-2 text-right font-mono">
                          {entry.request_count.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right font-mono">
                          {entry.total_input_tokens.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right font-mono">
                          {entry.total_output_tokens.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right font-mono">
                          {entry.total_tokens.toLocaleString()}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </TabsContent>

        <TabsContent value="secrets" className="pt-2">
          <div className="space-y-4">
            {proxySecretCount > 0 ? (
              <div className="rounded-md border">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b bg-muted/50">
                      <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                        Environment Variable
                      </th>
                      <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                        Proxy Target Host
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(sandbox.secret_mappings ?? {}).map(
                      ([envVar, host], i) => (
                        <tr
                          key={envVar}
                          className={i < proxySecretCount - 1 ? "border-b" : ""}
                        >
                          <td className="px-4 py-2">
                            <div className="flex items-center gap-2">
                              <Shield className="h-3.5 w-3.5 text-green-500 shrink-0" />
                              <code className="font-mono text-xs">{envVar}</code>
                            </div>
                          </td>
                          <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                            {host}
                          </td>
                        </tr>
                      ),
                    )}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                No proxy secret bindings configured.
              </p>
            )}

            {secretFileCount > 0 ? (
              <div className="rounded-md border">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b bg-muted/50">
                      <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                        Secret File Key
                      </th>
                      <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                        Mounted Path
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {(sandbox.secret_files ?? []).map((key, i) => (
                      <tr key={key} className={i < secretFileCount - 1 ? "border-b" : ""}>
                        <td className="px-4 py-2">
                          <code className="font-mono text-xs">{key}</code>
                        </td>
                        <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                          /run/agentkernel/secrets/{key}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                No secret files configured.
              </p>
            )}
          </div>
        </TabsContent>

        <TabsContent value="ports" className="pt-2">
          {sandbox.ports && sandbox.ports.length > 0 ? (
            <div className="rounded-md border">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                      Host Port
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                      Container Port
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                      Protocol
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                      Binding
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {sandbox.ports.map((binding, i) => {
                    const parsed = parsePortBinding(binding);
                    return (
                      <tr
                        key={`${binding}-${i}`}
                        className={
                          i < sandbox.ports!.length - 1 ? "border-b" : ""
                        }
                      >
                        <td className="px-4 py-2 font-mono text-xs">
                          {parsed.hostPort}
                        </td>
                        <td className="px-4 py-2 font-mono text-xs">
                          {parsed.containerPort}
                        </td>
                        <td className="px-4 py-2 font-mono text-xs uppercase text-muted-foreground">
                          {parsed.protocol}
                        </td>
                        <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                          {binding}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground py-4">
              No mapped ports. Add mappings when creating the sandbox (for
              example <code className="font-mono text-xs">8080:80</code>).
            </p>
          )}
        </TabsContent>

        <TabsContent value="exec" className="space-y-4">
          <div className="flex gap-2">
            <Input
              placeholder="Enter command (e.g., ls -la)"
              value={commandInput}
              onChange={(e) => setCommandInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleExec();
              }}
              disabled={isExecBusy}
            />
            <Button
              onClick={handleExec}
              disabled={!commandInput.trim() || isExecBusy}
            >
              {isExecBusy ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Play className="mr-2 h-4 w-4" />
              )}
              {isExecBusy ? "Running..." : "Run"}
            </Button>
          </div>

          <div className="flex items-center justify-between">
            <label className="flex items-center gap-2 cursor-pointer select-none">
              <input
                type="checkbox"
                checked={runInBackground}
                onChange={(e) => setRunInBackground(e.target.checked)}
                className="h-3.5 w-3.5 rounded border-neutral-400 accent-neutral-700"
              />
              <span className="text-xs text-muted-foreground">
                Run in background
              </span>
            </label>
            {history.length > 0 && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 text-xs text-muted-foreground hover:text-foreground"
                onClick={() => setHistory([])}
              >
                <X className="mr-1 h-3 w-3" />
                Clear
              </Button>
            )}
          </div>

          <div
            ref={outputRef}
            className="h-[400px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-sm text-neutral-200"
          >
            {history.length === 0 && !isExecBusy ? (
              <p className="text-neutral-500">
                Run a command to see output here.
              </p>
            ) : (
              <>
                {history.map((entry, i) => (
                  <div key={i} className="mb-4">
                    <div className="flex items-center gap-2 text-green-400">
                      <span>$ {entry.command}</span>
                      {entry.detachedJobId &&
                        entry.detachedJobId === activeDetachedJobId && (
                          <Loader2 className="h-3 w-3 animate-spin text-neutral-400" />
                        )}
                    </div>
                    {entry.output.output && (
                      <pre className="whitespace-pre-wrap text-neutral-200">
                        {entry.output.output}
                      </pre>
                    )}
                    {entry.detachedJobId &&
                      !entry.output.output &&
                      entry.detachedJobId === activeDetachedJobId && (
                        <span className="text-neutral-500 text-xs">
                          Waiting for output...
                        </span>
                      )}
                    {i < history.length - 1 && (
                      <Separator className="my-2 bg-neutral-800" />
                    )}
                  </div>
                ))}
                {execMutation.isPending && !runInBackground && (
                  <div className="flex items-center gap-2 text-neutral-500">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    <span className="text-xs">Executing...</span>
                  </div>
                )}
              </>
            )}
          </div>
        </TabsContent>

        <TabsContent value="logs" className="space-y-4">
          {/* Activity Log */}
          <div>
            <p className="text-sm font-medium mb-2">Activity Log</p>
            <div className="h-[300px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-xs text-neutral-200">
              {!sandboxLogs || sandboxLogs.length === 0 ? (
                <p className="text-neutral-500">No activity yet.</p>
              ) : (
                sandboxLogs.map((entry: AuditLogEntry, i: number) => {
                  const ts = entry.timestamp
                    ? new Date(entry.timestamp).toLocaleTimeString()
                    : "";
                  const eventType = entry.type ?? "unknown";
                  let detail = "";
                  let color = "text-neutral-300";
                  switch (eventType) {
                    case "sandbox_created":
                      detail = `Created (image: ${entry.image ?? "?"}, backend: ${entry.backend ?? "?"})`;
                      color = "text-blue-400";
                      break;
                    case "sandbox_started":
                      detail = "Started";
                      color = "text-green-400";
                      break;
                    case "sandbox_stopped":
                      detail = "Stopped";
                      color = "text-yellow-400";
                      break;
                    case "sandbox_removed":
                      detail = "Removed";
                      color = "text-red-400";
                      break;
                    case "command_executed": {
                      const cmd = Array.isArray(entry.command)
                        ? (entry.command as string[]).join(" ")
                        : String(entry.command ?? "");
                      const code = entry.exit_code;
                      detail = `$ ${cmd}`;
                      if (code !== undefined && code !== null && code !== 0) {
                        detail += ` (exit ${code})`;
                        color = "text-red-400";
                      } else {
                        color = "text-green-300";
                      }
                      break;
                    }
                    case "file_written":
                      detail = `Wrote ${entry.path ?? "?"}`;
                      break;
                    case "file_read":
                      detail = `Read ${entry.path ?? "?"}`;
                      break;
                    case "ssh_connected":
                      detail = `SSH connected (user: ${entry.ssh_user ?? "?"})`;
                      color = "text-blue-300";
                      break;
                    case "ssh_disconnected":
                      detail = "SSH disconnected";
                      break;
                    default: {
                      const parts: string[] = [];
                      for (const [k, v] of Object.entries(entry)) {
                        if (["timestamp", "pid", "user", "type"].includes(k))
                          continue;
                        if (v !== null && v !== undefined) {
                          parts.push(
                            `${k}=${Array.isArray(v) ? (v as string[]).join(" ") : String(v)}`,
                          );
                        }
                      }
                      detail = parts.join("  ");
                      break;
                    }
                  }
                  return (
                    <div key={i} className="leading-5">
                      <span className="text-neutral-600">{ts}</span>{" "}
                      <span className={color}>{detail}</span>
                    </div>
                  );
                })
              )}
            </div>
          </div>

          {/* Background Jobs */}
          <div className="flex gap-4">
            <div className="w-64 space-y-2">
              <p className="text-sm font-medium">Background Jobs</p>
              {!detachedJobs || detachedJobs.length === 0 ? (
                <p className="text-xs text-muted-foreground">
                  No background jobs. Use the Exec tab or CLI to run detached
                  commands.
                </p>
              ) : (
                detachedJobs.map((job: DetachedCommand) => (
                  <div
                    key={job.id}
                    className={`cursor-pointer rounded-md border p-2 text-xs ${
                      selectedJob === job.id
                        ? "border-primary bg-primary/10"
                        : "hover:bg-muted"
                    }`}
                    onClick={() => setSelectedJob(job.id)}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-mono truncate">
                        {job.command.join(" ")}
                      </span>
                      <span
                        className={`ml-2 shrink-0 rounded px-1 py-0.5 text-[10px] ${
                          job.status === "running"
                            ? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300"
                            : "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400"
                        }`}
                      >
                        {job.status}
                      </span>
                    </div>
                    {selectedJob === job.id && job.status === "running" && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="mt-1 h-6 text-xs text-destructive"
                        onClick={(e) => {
                          e.stopPropagation();
                          killJobMutation.mutate(job.id);
                        }}
                      >
                        Kill
                      </Button>
                    )}
                  </div>
                ))
              )}
            </div>
            <div className="flex-1 h-[400px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-sm text-neutral-200">
              {!selectedJob ? (
                <p className="text-neutral-500">
                  Select a background job to view its output.
                </p>
              ) : jobLogs ? (
                <pre className="whitespace-pre-wrap">
                  {jobLogs.stdout || ""}
                  {jobLogs.stderr && (
                    <span className="text-red-400">{jobLogs.stderr}</span>
                  )}
                  {!jobLogs.stdout && !jobLogs.stderr && (
                    <span className="text-neutral-500">No output yet.</span>
                  )}
                </pre>
              ) : (
                <p className="text-neutral-500">Loading...</p>
              )}
            </div>
          </div>
        </TabsContent>

        <TabsContent value="files" className="space-y-4">
          <FileBrowser sandboxName={name ?? ""} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
