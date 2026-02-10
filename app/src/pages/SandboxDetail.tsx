import { useState, useRef, useEffect } from "react";
import { useParams, Link, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  Trash2,
  Play,
  Square,
  Clock,
  Cpu,
  HardDrive,
  Image,
  Server,
  Calendar,
  Copy,
  Check,
  Terminal,
  Loader2,
  X,
  Download,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSandbox } from "@/lib/hooks/use-sandbox";
import { useExec } from "@/lib/hooks/use-exec";
import { api } from "@/lib/api";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import { FileBrowser } from "@/components/sandbox/file-browser";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
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
import type { RunOutput, DetachedCommand, AuditLogEntry } from "@/lib/types";

interface CommandEntry {
  command: string;
  output: RunOutput;
  /** When set, this entry shows streaming output from a detached job */
  detachedJobId?: string;
}

export function SandboxDetail() {
  const { name } = useParams<{ name: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data: sandbox, isLoading, error } = useSandbox(name ?? "");
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
  const outputRef = useRef<HTMLDivElement>(null);

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

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <Link
            to="/sandboxes"
            className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="h-4 w-4" />
          </Link>
          <div>
            <div className="flex items-center gap-3">
              <h1 className="text-3xl font-bold tracking-tight">
                {sandbox.name}
              </h1>
              <SandboxStatusBadge status={sandbox.status} />
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {sandbox.status.toLowerCase() === "running" && (
            <Button
              variant="outline"
              onClick={() => openTerminalMutation.mutate()}
              disabled={openTerminalMutation.isPending}
            >
              <Terminal className="mr-2 h-4 w-4" />
              {openTerminalMutation.isPending ? "Opening..." : "Open Terminal"}
            </Button>
          )}
          {sandbox.status.toLowerCase() === "running" && (
            <Button
              variant="outline"
              onClick={() => exportMutation.mutate()}
              disabled={exportMutation.isPending}
            >
              <Download className="mr-2 h-4 w-4" />
              {exportMutation.isPending ? "Exporting..." : "Export"}
            </Button>
          )}
          {sandbox.status.toLowerCase() === "running" ? (
            <Button
              variant="outline"
              onClick={() => stopMutation.mutate()}
              disabled={stopMutation.isPending}
            >
              <Square className="mr-2 h-4 w-4" />
              {stopMutation.isPending ? "Stopping..." : "Stop"}
            </Button>
          ) : (
            <Button
              variant="outline"
              onClick={() => startMutation.mutate()}
              disabled={startMutation.isPending}
            >
              <Play className="mr-2 h-4 w-4" />
              {startMutation.isPending ? "Starting..." : "Start"}
            </Button>
          )}
          <Button
            variant="destructive"
            onClick={() => removeMutation.mutate(sandbox.name)}
            disabled={removeMutation.isPending}
          >
            <Trash2 className="mr-2 h-4 w-4" />
            {removeMutation.isPending ? "Removing..." : "Remove"}
          </Button>
        </div>
      </div>

      <Tabs defaultValue="info">
        <TabsList>
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="exec">Exec</TabsTrigger>
          <TabsTrigger value="files">Files</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
        </TabsList>

        <TabsContent value="info" className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Status</CardTitle>
                <Server className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <SandboxStatusBadge status={sandbox.status} />
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Backend</CardTitle>
                <Server className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-sm font-mono">{sandbox.backend}</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Image</CardTitle>
                <Image className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                {sandbox.image ? (
                  <button
                    type="button"
                    className="group flex items-center gap-1.5 text-sm font-mono hover:text-foreground"
                    onClick={() => copyToClipboard(sandbox.image!, "image")}
                    title="Copy to clipboard"
                  >
                    <span className="truncate">{sandbox.image}</span>
                    {copiedField === "image" ? (
                      <Check className="h-3 w-3 shrink-0 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-50" />
                    )}
                  </button>
                ) : (
                  <p className="text-sm font-mono">—</p>
                )}
              </CardContent>
            </Card>

            {sandbox.vcpus != null && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">vCPUs</CardTitle>
                  <Cpu className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-2xl font-bold">{sandbox.vcpus}</p>
                </CardContent>
              </Card>
            )}

            {sandbox.memory_mb != null && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">Memory</CardTitle>
                  <HardDrive className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-2xl font-bold">{sandbox.memory_mb} MB</p>
                </CardContent>
              </Card>
            )}

            {sandbox.created_at && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">
                    Created At
                  </CardTitle>
                  <Calendar className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-sm">{formatDate(sandbox.created_at)}</p>
                </CardContent>
              </Card>
            )}

            {sandbox.ip && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">
                    IP Address
                  </CardTitle>
                  <Server className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <button
                    type="button"
                    className="group flex items-center gap-1.5 text-sm font-mono hover:text-foreground"
                    onClick={() => copyToClipboard(sandbox.ip!, "ip")}
                    title="Copy to clipboard"
                  >
                    <span>{sandbox.ip}</span>
                    {copiedField === "ip" ? (
                      <Check className="h-3 w-3 shrink-0 text-green-500" />
                    ) : (
                      <Copy className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-50" />
                    )}
                  </button>
                </CardContent>
              </Card>
            )}

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Connect</CardTitle>
                <Terminal className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <button
                  type="button"
                  className="group flex items-center gap-1.5 text-sm font-mono hover:text-foreground min-w-0"
                  onClick={() =>
                    copyToClipboard(
                      `agentkernel attach ${sandbox.name}`,
                      "connect",
                    )
                  }
                  title="Copy to clipboard"
                >
                  <span className="break-all">
                    agentkernel attach {sandbox.name}
                  </span>
                  {copiedField === "connect" ? (
                    <Check className="h-3 w-3 shrink-0 text-green-500" />
                  ) : (
                    <Copy className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-50" />
                  )}
                </button>
              </CardContent>
            </Card>

            {sandbox.ports && sandbox.ports.length > 0 && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">Ports</CardTitle>
                  <Server className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-sm font-mono">
                    {sandbox.ports.join(", ")}
                  </p>
                </CardContent>
              </Card>
            )}

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">
                  Extend TTL
                </CardTitle>
                <Clock className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <Dialog
                  open={extendDialogOpen}
                  onOpenChange={setExtendDialogOpen}
                >
                  <DialogTrigger asChild>
                    <Button variant="outline" size="sm">
                      <Clock className="mr-2 h-4 w-4" />
                      Extend
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
                      <Label htmlFor="extend-seconds">Seconds to add</Label>
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
                        {extendMutation.isPending ? "Extending..." : "Extend"}
                      </Button>
                    </DialogFooter>
                  </DialogContent>
                </Dialog>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Resize</CardTitle>
                <Cpu className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <div className="flex items-center gap-3 mb-2">
                  <span className="text-xs text-muted-foreground">
                    {sandbox.vcpus ?? "?"} vCPU
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {sandbox.memory_mb ?? "?"} MB
                  </span>
                </div>
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
                    <Button variant="outline" size="sm">
                      <Cpu className="mr-2 h-4 w-4" />
                      Resize
                    </Button>
                  </DialogTrigger>
                  <DialogContent>
                    <DialogHeader>
                      <DialogTitle>Resize Sandbox</DialogTitle>
                      <DialogDescription>
                        Change CPU and memory allocation. The sandbox will be
                        stopped and recreated with the new resources.
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
                        {resizeMutation.isPending ? "Resizing..." : "Resize"}
                      </Button>
                    </DialogFooter>
                  </DialogContent>
                </Dialog>
              </CardContent>
            </Card>
          </div>
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
