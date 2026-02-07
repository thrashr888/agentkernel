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
  Timer,
  Calendar,
} from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import type { RunOutput } from "@/lib/types";

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
      api.extendTtl(sandboxName, seconds),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandbox", name] });
      setExtendDialogOpen(false);
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
                <CardTitle className="text-sm font-medium">Image</CardTitle>
                <Image className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-sm font-mono">{sandbox.image}</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">vCPUs</CardTitle>
                <Cpu className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">{sandbox.vcpus}</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Memory</CardTitle>
                <HardDrive className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">{sandbox.memory_mb} MB</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Created At</CardTitle>
                <Calendar className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-sm">{formatDate(sandbox.created_at)}</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">TTL</CardTitle>
                <Timer className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-sm">{sandbox.ttl_seconds}s</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Expires At</CardTitle>
                <Clock className="h-4 w-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                <p className="text-sm">
                  {sandbox.expires_at
                    ? formatDate(sandbox.expires_at)
                    : "No expiration"}
                </p>
              </CardContent>
            </Card>

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
                  {entry.output.stdout && (
                    <pre className="whitespace-pre-wrap text-neutral-200">
                      {entry.output.stdout}
                    </pre>
                  )}
                  {entry.output.stderr && (
                    <pre className="whitespace-pre-wrap text-red-400">
                      {entry.output.stderr}
                    </pre>
                  )}
                  <div className="text-neutral-500">
                    exit code: {entry.output.exit_code}
                  </div>
                  {i < history.length - 1 && (
                    <Separator className="my-2 bg-neutral-800" />
                  )}
                </div>
              ))
            )}
          </div>
        </TabsContent>

        <TabsContent value="logs" className="space-y-4">
          <div className="h-[400px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-sm text-neutral-200">
            <p className="text-neutral-500">
              Logs are streamed from the sandbox. Use the Exec tab to run
              commands and view output.
            </p>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
