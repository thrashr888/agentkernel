import { useState, useRef, useEffect } from "react";
import { useParams, Link, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  Trash2,
  Play,
  Clock,
  Cpu,
  HardDrive,
  Image,
  Server,
  Calendar,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSandbox } from "@/lib/hooks/use-sandbox";
import { useExec } from "@/lib/hooks/use-exec";
import { api } from "@/lib/api";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
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
import { formatDate } from "@/lib/utils";
import type { RunOutput, DetachedCommand } from "@/lib/types";

interface CommandEntry {
  command: string;
  output: RunOutput;
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
  const [selectedJob, setSelectedJob] = useState<string | null>(null);
  const outputRef = useRef<HTMLDivElement>(null);

  const removeMutation = useMutation({
    mutationFn: (sandboxName: string) => api.removeSandbox(sandboxName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      navigate("/sandboxes");
    },
  });

  const extendMutation = useMutation({
    mutationFn: ({ sandboxName, seconds }: { sandboxName: string; seconds: number }) =>
      api.extendTtl(sandboxName, `${seconds}s`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      setExtendDialogOpen(false);
    },
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
    },
  });

  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [history]);

  function handleExec() {
    if (!commandInput.trim() || !name) return;
    const cmd = commandInput.trim();
    setCommandInput("");
    execMutation.mutate(
      {
        name,
        command: ["sh", "-c", cmd],
      },
      {
        onSuccess: (output) => {
          setHistory((prev) => [...prev, { command: cmd, output }]);
        },
      }
    );
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
        <Button
          variant="destructive"
          onClick={() => removeMutation.mutate(sandbox.name)}
          disabled={removeMutation.isPending}
        >
          <Trash2 className="mr-2 h-4 w-4" />
          {removeMutation.isPending ? "Removing..." : "Remove"}
        </Button>
      </div>

      <Tabs defaultValue="info">
        <TabsList>
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="exec">Exec</TabsTrigger>
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
                <p className="text-sm font-mono">{sandbox.image ?? "—"}</p>
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
                  <CardTitle className="text-sm font-medium">Created At</CardTitle>
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
                  <CardTitle className="text-sm font-medium">IP Address</CardTitle>
                  <Server className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-sm font-mono">{sandbox.ip}</p>
                </CardContent>
              </Card>
            )}

            {sandbox.ports && sandbox.ports.length > 0 && (
              <Card>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">Ports</CardTitle>
                  <Server className="h-4 w-4 text-muted-foreground" />
                </CardHeader>
                <CardContent>
                  <p className="text-sm font-mono">{sandbox.ports.join(", ")}</p>
                </CardContent>
              </Card>
            )}

            <Card className="flex items-center justify-center">
              <CardContent className="pt-6">
                <Dialog open={extendDialogOpen} onOpenChange={setExtendDialogOpen}>
                  <DialogTrigger asChild>
                    <Button variant="outline" size="sm">
                      <Clock className="mr-2 h-4 w-4" />
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
                      <Label htmlFor="extend-seconds">Seconds to add</Label>
                      <Input
                        id="extend-seconds"
                        type="number"
                        min={60}
                        step={60}
                        value={extendSeconds}
                        onChange={(e) => setExtendSeconds(Number(e.target.value))}
                      />
                    </div>
                    {extendMutation.error && (
                      <p className="text-sm text-destructive">
                        {extendMutation.error.message}
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
              disabled={execMutation.isPending}
            />
            <Button
              onClick={handleExec}
              disabled={!commandInput.trim() || execMutation.isPending}
            >
              <Play className="mr-2 h-4 w-4" />
              {execMutation.isPending ? "Running..." : "Run"}
            </Button>
          </div>

          <div
            ref={outputRef}
            className="h-[400px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-sm text-neutral-200"
          >
            {history.length === 0 ? (
              <p className="text-neutral-500">
                Run a command to see output here.
              </p>
            ) : (
              history.map((entry, i) => (
                <div key={i} className="mb-4">
                  <div className="text-green-400">$ {entry.command}</div>
                  {entry.output.output && (
                    <pre className="whitespace-pre-wrap text-neutral-200">
                      {entry.output.output}
                    </pre>
                  )}
                  {i < history.length - 1 && (
                    <Separator className="my-2 bg-neutral-800" />
                  )}
                </div>
              ))
            )}
          </div>
        </TabsContent>

        <TabsContent value="logs" className="space-y-4">
          <div className="flex gap-4">
            <div className="w-64 space-y-2">
              <p className="text-sm font-medium">Background Jobs</p>
              {!detachedJobs || detachedJobs.length === 0 ? (
                <p className="text-xs text-muted-foreground">
                  No background jobs. Use the Exec tab or CLI to run detached commands.
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
                      <span className="font-mono truncate">{job.command.join(" ")}</span>
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
      </Tabs>
    </div>
  );
}
