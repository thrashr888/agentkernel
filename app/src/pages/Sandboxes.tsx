import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { Plus, MoreHorizontal, Trash2, Camera, Square, Play, ChevronLeft, ChevronRight, Terminal, Copy, ArrowUpDown, ArrowUp, ArrowDown, Search, X } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useSandboxes } from "@/lib/hooks/use-sandboxes";
import { api } from "@/lib/api";
import type { CreateSandboxRequest } from "@/lib/types";
import { SandboxStatusBadge } from "@/components/sandbox/sandbox-status-badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Card, CardContent } from "@/components/ui/card";
import { toast } from "@/components/ui/use-toast";
import { formatRelativeDate } from "@/lib/utils";

export function Sandboxes() {
  const { data: sandboxes, isLoading, error } = useSandboxes();
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const [dialogOpen, setDialogOpen] = useState(false);

  // Open create dialog when navigated with ?action=create (e.g. from tray menu)
  useEffect(() => {
    if (searchParams.get("action") === "create") {
      setDialogOpen(true);
      searchParams.delete("action");
      setSearchParams(searchParams, { replace: true });
    }
  }, [searchParams, setSearchParams]);

  const [formName, setFormName] = useState("");
  const [formImage, setFormImage] = useState("alpine:3.20");
  const [formVcpus, setFormVcpus] = useState(1);
  const [formMemory, setFormMemory] = useState(512);
  const [formProfile, setFormProfile] = useState("restrictive");
  const [formAgent, setFormAgent] = useState("");
  const [formLabels, setFormLabels] = useState("");
  const [formDescription, setFormDescription] = useState("");
  const [page, setPage] = useState(0);
  const PAGE_SIZE = 24;

  // Sort, filter, search state
  type SortColumn = "name" | "status" | "image" | "ip" | "created";
  const [sortColumn, setSortColumn] = useState<SortColumn>("name");
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [search, setSearch] = useState("");

  const createMutation = useMutation({
    mutationFn: (req: CreateSandboxRequest) => api.createSandbox(req),
    onMutate: () => {
      return { toastId: toast("Creating sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox created!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      setDialogOpen(false);
      resetForm();
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => api.removeSandbox(name),
    onMutate: () => {
      return { toastId: toast("Removing sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox removed!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const startMutation = useMutation({
    mutationFn: (name: string) => api.startSandbox(name),
    onMutate: () => {
      return { toastId: toast("Starting sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox started!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const stopMutation = useMutation({
    mutationFn: (name: string) => api.stopSandbox(name),
    onMutate: () => {
      return { toastId: toast("Stopping sandbox...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Sandbox stopped!", "success");
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

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

  const snapshotMutation = useMutation({
    mutationFn: ({ sandboxName, snapshotName }: { sandboxName: string; snapshotName: string }) =>
      api.takeSnapshot(sandboxName, snapshotName),
    onMutate: () => {
      return { toastId: toast("Taking snapshot...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Snapshot created!", "success");
      queryClient.invalidateQueries({ queryKey: ["snapshots"] });
    },
    onError: (err: unknown, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  function resetForm() {
    setFormName("");
    setFormImage("alpine:3.20");
    setFormVcpus(1);
    setFormMemory(512);
    setFormProfile("restrictive");
    setFormAgent("");
    setFormLabels("");
    setFormDescription("");
  }

  function handleCreate() {
    if (!formName.trim()) return;
    const labels: Record<string, string> = {};
    if (formLabels.trim()) {
      for (const part of formLabels.split(",")) {
        const [k, v] = part.trim().split("=");
        if (k && v) labels[k.trim()] = v.trim();
      }
    }
    createMutation.mutate({
      name: formName.trim(),
      image: formImage,
      vcpus: formVcpus,
      memory_mb: formMemory,
      profile: formProfile,
      ...(formAgent && formAgent !== "none" ? { agent: formAgent } : {}),
      ...(Object.keys(labels).length > 0 ? { labels } : {}),
      ...(formDescription.trim() ? { description: formDescription.trim() } : {}),
    });
  }

  function toggleSort(col: SortColumn) {
    if (sortColumn === col) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortColumn(col);
      setSortDir("asc");
    }
    setPage(0);
  }

  function SortIcon({ col }: { col: SortColumn }) {
    if (sortColumn !== col) return <ArrowUpDown className="ml-1 h-3 w-3 opacity-40" />;
    return sortDir === "asc"
      ? <ArrowUp className="ml-1 h-3 w-3" />
      : <ArrowDown className="ml-1 h-3 w-3" />;
  }

  // Apply search + status filter + sort
  const filtered = (sandboxes ?? []).filter((s) => {
    if (statusFilter !== "all" && s.status.toLowerCase() !== statusFilter) return false;
    if (search) {
      const q = search.toLowerCase();
      const labelStr = Object.entries(s.labels ?? {}).map(([k, v]) => `${k}=${v}`).join(" ");
      const haystack = [s.name, s.image ?? "", s.ip ?? "", labelStr].join(" ").toLowerCase();
      if (!haystack.includes(q)) return false;
    }
    return true;
  });

  const sorted = [...filtered].sort((a, b) => {
    const dir = sortDir === "asc" ? 1 : -1;
    switch (sortColumn) {
      case "name": return dir * a.name.localeCompare(b.name);
      case "status": return dir * a.status.localeCompare(b.status);
      case "image": return dir * (a.image ?? "").localeCompare(b.image ?? "");
      case "ip": return dir * (a.ip ?? "").localeCompare(b.ip ?? "");
      case "created": return dir * (a.created_at ?? "").localeCompare(b.created_at ?? "");
      default: return 0;
    }
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Sandboxes</h1>
          <p className="text-muted-foreground">
            Manage your isolated sandbox environments
          </p>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button>
              <Plus className="mr-2 h-4 w-4" />
              Create Sandbox
            </Button>
          </DialogTrigger>
          <DialogContent className="max-h-[85vh] flex flex-col">
            <DialogHeader>
              <DialogTitle>Create Sandbox</DialogTitle>
              <DialogDescription>
                Configure a new isolated sandbox environment.
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-4 py-4 overflow-y-auto pr-1">
              <div className="grid gap-2">
                <Label htmlFor="name">Name</Label>
                <Input
                  id="name"
                  placeholder="my-sandbox"
                  value={formName}
                  onChange={(e) => setFormName(e.target.value)}
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="description">Description (optional)</Label>
                <Input
                  id="description"
                  placeholder="What this sandbox is for"
                  value={formDescription}
                  onChange={(e) => setFormDescription(e.target.value)}
                />
              </div>
              <div className="grid gap-2">
                <Label>Agent (optional)</Label>
                <Select value={formAgent} onValueChange={(val) => {
                  setFormAgent(val);
                  if (val && val !== "none") {
                    setFormImage("node:22-alpine");
                    setFormVcpus(2);
                    setFormMemory(1024);
                    setFormProfile("moderate");
                  }
                }}>
                  <SelectTrigger>
                    <SelectValue placeholder="None — plain sandbox" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">None</SelectItem>
                    <SelectItem value="claude">Claude Code</SelectItem>
                    <SelectItem value="gemini">Gemini CLI</SelectItem>
                    <SelectItem value="codex">Codex</SelectItem>
                    <SelectItem value="opencode">OpenCode</SelectItem>
                    <SelectItem value="amp">Amp</SelectItem>
                    <SelectItem value="pi">Pi</SelectItem>
                    <SelectItem value="copilot">Copilot CLI</SelectItem>
                  </SelectContent>
                </Select>
                {formAgent && formAgent !== "none" && (
                  <p className="text-xs text-muted-foreground">
                    The {formAgent} CLI will be installed automatically on start
                  </p>
                )}
              </div>
              <div className="grid gap-2">
                <Label htmlFor="image">Image</Label>
                <Input
                  id="image"
                  value={formImage}
                  onChange={(e) => setFormImage(e.target.value)}
                />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="grid gap-2">
                  <Label htmlFor="vcpus">vCPUs</Label>
                  <Input
                    id="vcpus"
                    type="number"
                    min={1}
                    value={formVcpus}
                    onChange={(e) => setFormVcpus(Number(e.target.value))}
                  />
                </div>
                <div className="grid gap-2">
                  <Label htmlFor="memory">Memory (MB)</Label>
                  <Input
                    id="memory"
                    type="number"
                    min={128}
                    step={128}
                    value={formMemory}
                    onChange={(e) => setFormMemory(Number(e.target.value))}
                  />
                </div>
              </div>
              <div className="grid gap-2">
                <Label>Security Profile</Label>
                <Select value={formProfile} onValueChange={setFormProfile}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="permissive">Permissive</SelectItem>
                    <SelectItem value="moderate">Moderate</SelectItem>
                    <SelectItem value="restrictive">Restrictive</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {formProfile === "permissive" && "Network + mount cwd + mount home + pass env vars"}
                  {formProfile === "moderate" && "Network only — no mounts, no env pass-through"}
                  {formProfile === "restrictive" && "No network, no mounts, read-only filesystem"}
                </p>
              </div>
              <div className="grid gap-2">
                <Label htmlFor="labels">Labels (optional)</Label>
                <Input
                  id="labels"
                  placeholder="env=prod, team=infra"
                  value={formLabels}
                  onChange={(e) => setFormLabels(e.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  Comma-separated key=value pairs for fleet management
                </p>
              </div>
            </div>
            {!!createMutation.error && (
              <p className="text-sm text-destructive">
                {createMutation.error instanceof Error ? createMutation.error.message : String(createMutation.error)}
              </p>
            )}
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => setDialogOpen(false)}
              >
                Cancel
              </Button>
              <Button
                onClick={handleCreate}
                disabled={!formName.trim() || createMutation.isPending}
              >
                {createMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {[1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-16 rounded-lg" />
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
      ) : !sandboxes || sandboxes.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <p className="mb-4 text-muted-foreground">
              No sandboxes found. Create your first sandbox to get started.
            </p>
            <Button onClick={() => setDialogOpen(true)}>
              <Plus className="mr-2 h-4 w-4" />
              Create Sandbox
            </Button>
          </CardContent>
        </Card>
      ) : (
        <>
        {/* Toolbar: count filter buttons + search */}
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1 text-sm">
            <button
              onClick={() => { setStatusFilter("all"); setPage(0); }}
              className={`rounded-md px-2.5 py-1 font-medium transition-colors ${statusFilter === "all" ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:text-foreground hover:bg-accent/50"}`}
            >
              {sandboxes.length} total
            </button>
            <button
              onClick={() => { setStatusFilter(statusFilter === "running" ? "all" : "running"); setPage(0); }}
              className={`rounded-md px-2.5 py-1 font-medium transition-colors ${statusFilter === "running" ? "bg-green-100 text-green-700 dark:bg-green-950 dark:text-green-400" : "text-muted-foreground hover:text-foreground hover:bg-accent/50"}`}
            >
              {sandboxes.filter((s) => s.status.toLowerCase() === "running").length} running
            </button>
            <button
              onClick={() => { setStatusFilter(statusFilter === "stopped" ? "all" : "stopped"); setPage(0); }}
              className={`rounded-md px-2.5 py-1 font-medium transition-colors ${statusFilter === "stopped" ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:text-foreground hover:bg-accent/50"}`}
            >
              {sandboxes.filter((s) => s.status.toLowerCase() === "stopped").length} stopped
            </button>
          </div>
          <div className="h-4 w-px bg-border" />
          <div className="relative flex-1 max-w-sm">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search by name, image, or IP..."
              value={search}
              onChange={(e) => { setSearch(e.target.value); setPage(0); }}
              className="pl-9 pr-8"
            />
            {search && (
              <button
                onClick={() => { setSearch(""); setPage(0); }}
                className="absolute right-2.5 top-2.5 text-muted-foreground hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </button>
            )}
          </div>
        </div>

        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>
                  <button className="inline-flex items-center hover:text-foreground" onClick={() => toggleSort("name")}>
                    Name <SortIcon col="name" />
                  </button>
                </TableHead>
                <TableHead>
                  <button className="inline-flex items-center hover:text-foreground" onClick={() => toggleSort("status")}>
                    Status <SortIcon col="status" />
                  </button>
                </TableHead>
                <TableHead>
                  <button className="inline-flex items-center hover:text-foreground" onClick={() => toggleSort("image")}>
                    Image <SortIcon col="image" />
                  </button>
                </TableHead>
                <TableHead>
                  <button className="inline-flex items-center hover:text-foreground" onClick={() => toggleSort("ip")}>
                    IP <SortIcon col="ip" />
                  </button>
                </TableHead>
                <TableHead>
                  <button className="inline-flex items-center hover:text-foreground" onClick={() => toggleSort("created")}>
                    Created <SortIcon col="created" />
                  </button>
                </TableHead>
                <TableHead className="w-[70px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {(() => {
                const totalPages = Math.ceil(sorted.length / PAGE_SIZE);
                const safePage = Math.min(page, Math.max(totalPages - 1, 0));
                return sorted.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE);
              })().map((sandbox) => (
                <TableRow key={sandbox.name}>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <Link
                        to={`/sandboxes/${sandbox.name}`}
                        className="font-medium hover:underline"
                      >
                        {sandbox.name}
                      </Link>
                      {sandbox.labels && Object.keys(sandbox.labels).length > 0 && (
                        <div className="flex gap-1 flex-wrap">
                          {Object.entries(sandbox.labels).map(([k, v]) => (
                            <span
                              key={k}
                              className="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
                              title={`${k}=${v}`}
                            >
                              {k}={v}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <SandboxStatusBadge status={sandbox.status} />
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {sandbox.image ?? sandbox.backend}
                  </TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {sandbox.ip ?? "—"}
                    {sandbox.ports && sandbox.ports.length > 0 && (
                      <span className="ml-1 text-xs">({sandbox.ports.join(", ")})</span>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatRelativeDate(sandbox.created_at)}
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon">
                          <MoreHorizontal className="h-4 w-4" />
                          <span className="sr-only">Actions</span>
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        {sandbox.status.toLowerCase() !== "running" && (
                          <>
                            <DropdownMenuItem
                              onClick={() => startMutation.mutate(sandbox.name)}
                            >
                              <Play className="mr-2 h-4 w-4" />
                              Start
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                          </>
                        )}
                        {sandbox.status.toLowerCase() === "running" && (
                          <>
                            <DropdownMenuItem
                              onClick={() => stopMutation.mutate(sandbox.name)}
                            >
                              <Square className="mr-2 h-4 w-4" />
                              Stop
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                          </>
                        )}
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
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          onClick={() =>
                            snapshotMutation.mutate({
                              sandboxName: sandbox.name,
                              snapshotName: `${sandbox.name}-snap-${Date.now()}`,
                            })
                          }
                        >
                          <Camera className="mr-2 h-4 w-4" />
                          Take Snapshot
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          className="text-destructive focus:text-destructive"
                          onClick={() => removeMutation.mutate(sandbox.name)}
                        >
                          <Trash2 className="mr-2 h-4 w-4" />
                          Remove
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
        {sorted.length === 0 && (search || statusFilter !== "all") && (
          <div className="flex flex-col items-center justify-center py-8 text-center">
            <Search className="mb-3 h-8 w-8 text-muted-foreground/50" />
            <p className="text-sm text-muted-foreground">
              No sandboxes match your filters.
            </p>
            <Button
              variant="link"
              size="sm"
              onClick={() => { setSearch(""); setStatusFilter("all"); setPage(0); }}
            >
              Clear filters
            </Button>
          </div>
        )}
        {sorted.length > PAGE_SIZE && (() => {
          const totalPages = Math.ceil(sorted.length / PAGE_SIZE);
          return (
            <div className="flex items-center justify-between pt-2">
              <span className="text-xs text-muted-foreground">
                {page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, sorted.length)} of {sorted.length}
              </span>
              <div className="flex items-center gap-1">
                <Button
                  variant="outline"
                  size="icon"
                  className="h-8 w-8"
                  disabled={page === 0}
                  onClick={() => setPage(page - 1)}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-xs text-muted-foreground px-2">
                  {page + 1} / {totalPages}
                </span>
                <Button
                  variant="outline"
                  size="icon"
                  className="h-8 w-8"
                  disabled={page >= totalPages - 1}
                  onClick={() => setPage(page + 1)}
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          );
        })()}
        </>
      )}
    </div>
  );
}
