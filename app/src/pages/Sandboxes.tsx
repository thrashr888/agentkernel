import { useState } from "react";
import { Link } from "react-router-dom";
import { Plus, MoreHorizontal, Trash2, Camera, Square, Play } from "lucide-react";
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
import { formatRelativeDate } from "@/lib/utils";

export function Sandboxes() {
  const { data: sandboxes, isLoading, error } = useSandboxes();
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);

  const [formName, setFormName] = useState("");
  const [formImage, setFormImage] = useState("alpine:3.20");
  const [formVcpus, setFormVcpus] = useState(1);
  const [formMemory, setFormMemory] = useState(512);
  const [formProfile, setFormProfile] = useState("restrictive");

  const createMutation = useMutation({
    mutationFn: (req: CreateSandboxRequest) => api.createSandbox(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      setDialogOpen(false);
      resetForm();
    },
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => api.removeSandbox(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
  });

  const startMutation = useMutation({
    mutationFn: (name: string) => api.startSandbox(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
    },
  });

  const snapshotMutation = useMutation({
    mutationFn: ({ sandboxName, snapshotName }: { sandboxName: string; snapshotName: string }) =>
      api.takeSnapshot(sandboxName, snapshotName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["snapshots"] });
    },
  });

  function resetForm() {
    setFormName("");
    setFormImage("alpine:3.20");
    setFormVcpus(1);
    setFormMemory(512);
    setFormProfile("restrictive");
  }

  function handleCreate() {
    if (!formName.trim()) return;
    createMutation.mutate({
      name: formName.trim(),
      image: formImage,
      vcpus: formVcpus,
      memory_mb: formMemory,
    });
  }

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
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Sandbox</DialogTitle>
              <DialogDescription>
                Configure a new isolated sandbox environment.
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-4 py-4">
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
              </div>
            </div>
            {createMutation.error && (
              <p className="text-sm text-destructive">
                {createMutation.error.message}
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
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Image</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[70px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {[...sandboxes].sort((a, b) => a.name.localeCompare(b.name)).map((sandbox) => (
                <TableRow key={sandbox.name}>
                  <TableCell>
                    <Link
                      to={`/sandboxes/${sandbox.name}`}
                      className="font-medium hover:underline"
                    >
                      {sandbox.name}
                    </Link>
                  </TableCell>
                  <TableCell>
                    <SandboxStatusBadge status={sandbox.status} />
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {sandbox.image ?? sandbox.backend}
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
                              onClick={() => removeMutation.mutate(sandbox.name)}
                            >
                              <Square className="mr-2 h-4 w-4" />
                              Stop
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                          </>
                        )}
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
      )}
    </div>
  );
}
