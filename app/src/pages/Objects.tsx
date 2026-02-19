import { useState, useEffect } from "react";
import {
  Trash2,
  AlertTriangle,
  Blocks,
  RefreshCw,
  Plus,
  ArrowLeft,
  Save,
  Copy,
  Terminal,
} from "lucide-react";
import { useMutation, useQueryClient, useQuery } from "@tanstack/react-query";
import { useObjects } from "@/lib/hooks/use-objects";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
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
import { Link } from "react-router-dom";
import { SandboxPicker } from "@/components/sandbox-picker";

function statusBadge(status: string) {
  switch (status) {
    case "active":
      return <Badge variant="success">Active</Badge>;
    case "hibernating":
      return <Badge variant="warning">Hibernating</Badge>;
    case "deleted":
      return <Badge variant="secondary">Deleted</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function ObjectDetail({
  objectId,
  onBack,
}: {
  objectId: string;
  onBack: () => void;
}) {
  const queryClient = useQueryClient();
  const {
    data: obj,
    isLoading,
    error,
  } = useQuery({
    queryKey: ["objects", objectId],
    queryFn: () => api.getObject(objectId),
    refetchInterval: 5000,
  });

  const [storageEditing, setStorageEditing] = useState(false);
  const [storageText, setStorageText] = useState("");
  const [storageError, setStorageError] = useState<string | null>(null);

  // Sync editor text when object data changes (and not currently editing)
  useEffect(() => {
    if (obj && !storageEditing) {
      setStorageText(JSON.stringify(obj.storage ?? {}, null, 2));
    }
  }, [obj, storageEditing]);

  const patchMutation = useMutation({
    mutationFn: (storage: unknown) => api.patchObject(objectId, storage),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["objects", objectId] });
      setStorageEditing(false);
      setStorageError(null);
      toast.success("Storage updated");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => api.deleteObject(objectId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["objects"] });
      toast.success("Object deleted");
      onBack();
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  if (error || !obj) {
    return (
      <div className="space-y-6">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
          <p className="text-sm text-destructive">
            {error instanceof Error ? error.message : "Object not found"}
          </p>
        </div>
      </div>
    );
  }

  const storageJson = JSON.stringify(obj.storage ?? {}, null, 2);
  const hasStorage = storageJson !== "{}" && storageJson !== "null";

  function handleSaveStorage() {
    try {
      const parsed = JSON.parse(storageText);
      setStorageError(null);
      patchMutation.mutate(parsed);
    } catch {
      setStorageError("Invalid JSON");
    }
  }

  function copySnippet(text: string) {
    navigator.clipboard.writeText(text);
    toast.success("Copied to clipboard");
  }

  const baseUrl = "http://localhost:18888";

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Button variant="ghost" size="sm" onClick={onBack}>
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back
          </Button>
          <div>
            <h1 className="text-3xl font-bold tracking-tight">
              {obj.class}/{obj.object_id}
            </h1>
            <p className="text-muted-foreground flex items-center gap-2">
              {statusBadge(obj.status)}
              <span className="text-sm">
                Created {new Date(obj.created_at).toLocaleString()}
              </span>
            </p>
          </div>
        </div>
        <Button
          variant="destructive"
          size="sm"
          onClick={() => deleteMutation.mutate()}
          disabled={deleteMutation.isPending}
        >
          <Trash2 className="h-4 w-4 mr-1" />
          {deleteMutation.isPending ? "Deleting..." : "Delete"}
        </Button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <Card>
          <CardHeader>
            <CardTitle>Details</CardTitle>
          </CardHeader>
          <CardContent>
            <dl className="space-y-3 text-sm">
              <div className="flex justify-between">
                <dt className="text-muted-foreground">ID</dt>
                <dd className="font-mono text-xs">{obj.id}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Class</dt>
                <dd className="font-mono">{obj.class}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Object ID</dt>
                <dd className="font-mono">{obj.object_id}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Status</dt>
                <dd>{statusBadge(obj.status)}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Sandbox</dt>
                <dd>
                  {obj.sandbox ? (
                    <Link
                      to={`/sandboxes/${obj.sandbox}`}
                      className="text-primary hover:underline font-mono"
                    >
                      {obj.sandbox}
                    </Link>
                  ) : (
                    <span className="text-muted-foreground">—</span>
                  )}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Idle Timeout</dt>
                <dd className="font-mono">{obj.idle_timeout_seconds}s</dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Updated</dt>
                <dd>{new Date(obj.updated_at).toLocaleString()}</dd>
              </div>
            </dl>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-start justify-between space-y-0">
            <div>
              <CardTitle>Storage</CardTitle>
              <CardDescription>
                {hasStorage
                  ? "Persisted storage state for this object"
                  : "No storage data yet"}
              </CardDescription>
            </div>
            <div className="flex gap-1">
              {storageEditing ? (
                <>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setStorageEditing(false);
                      setStorageError(null);
                      setStorageText(storageJson);
                    }}
                  >
                    Cancel
                  </Button>
                  <Button
                    size="sm"
                    onClick={handleSaveStorage}
                    disabled={patchMutation.isPending}
                  >
                    <Save className="h-3.5 w-3.5 mr-1" />
                    {patchMutation.isPending ? "Saving..." : "Save"}
                  </Button>
                </>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setStorageEditing(true)}
                >
                  Edit
                </Button>
              )}
            </div>
          </CardHeader>
          <CardContent>
            {storageEditing ? (
              <div className="space-y-2">
                <textarea
                  value={storageText}
                  onChange={(e) => {
                    setStorageText(e.target.value);
                    setStorageError(null);
                  }}
                  className="w-full rounded-md border bg-muted p-3 text-xs font-mono min-h-[200px] max-h-[400px] resize-y focus:outline-none focus:ring-2 focus:ring-ring"
                  spellCheck={false}
                />
                {storageError && (
                  <p className="text-xs text-destructive">{storageError}</p>
                )}
              </div>
            ) : (
              <pre className="overflow-auto rounded-md bg-muted p-3 text-xs font-mono max-h-[300px]">
                {storageJson}
              </pre>
            )}
          </CardContent>
        </Card>
      </div>

      {/* API reference */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Terminal className="h-4 w-4" />
            API Reference
          </CardTitle>
          <CardDescription>
            Interact with this object via the REST API or CLI
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {[
            {
              label: "Get object",
              cmd: `curl ${baseUrl}/objects/${obj.id}`,
            },
            {
              label: "Update storage",
              cmd: `curl -X PATCH ${baseUrl}/objects/${obj.id} \\\n  -H "Content-Type: application/json" \\\n  -d '{"storage": {"key": "value"}}'`,
            },
            {
              label: "Update status",
              cmd: `curl -X PATCH ${baseUrl}/objects/${obj.id} \\\n  -H "Content-Type: application/json" \\\n  -d '{"status": "active"}'`,
            },
            {
              label: "Call method",
              cmd: `curl -X POST ${baseUrl}/objects/${obj.class}/${obj.object_id}/call/myMethod \\\n  -H "Content-Type: application/json" \\\n  -d '{"arg1": "value"}'`,
            },
            {
              label: "Delete object",
              cmd: `curl -X DELETE ${baseUrl}/objects/${obj.id}`,
            },
          ].map((item) => (
            <div key={item.label} className="space-y-1">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-muted-foreground">
                  {item.label}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 px-2"
                  onClick={() => copySnippet(item.cmd)}
                >
                  <Copy className="h-3 w-3" />
                </Button>
              </div>
              <pre className="overflow-auto rounded-md bg-muted px-3 py-2 text-xs font-mono whitespace-pre-wrap">
                {item.cmd}
              </pre>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

export function Objects() {
  const queryClient = useQueryClient();
  const {
    data: objects,
    isLoading,
    error,
    refetch,
    isRefetching,
  } = useObjects();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [newClass, setNewClass] = useState("");
  const [newObjectId, setNewObjectId] = useState("");
  const [newSandbox, setNewSandbox] = useState("");

  const [selectedObjectId, setSelectedObjectId] = useState<string | null>(null);

  const createMutation = useMutation({
    mutationFn: () =>
      api.createObject({
        class: newClass.trim(),
        object_id: newObjectId.trim(),
        sandbox: newSandbox.trim() || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["objects"] });
      setNewClass("");
      setNewObjectId("");
      setNewSandbox("");
      setDialogOpen(false);
      toast.success("Object created");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.deleteObject(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["objects"] });
      toast.success("Object deleted");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  if (selectedObjectId) {
    return (
      <ObjectDetail
        objectId={selectedObjectId}
        onBack={() => setSelectedObjectId(null)}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Durable Objects</h1>
          <p className="text-muted-foreground">
            Manage stateful durable objects across sandboxes
          </p>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button>
              <Plus className="h-4 w-4 mr-2" />
              New Object
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Durable Object</DialogTitle>
              <DialogDescription>
                Register a new durable object instance.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <div className="grid gap-2">
                <Label htmlFor="obj-class">Class</Label>
                <Input
                  id="obj-class"
                  value={newClass}
                  onChange={(e) => setNewClass(e.target.value)}
                  placeholder="Counter"
                  className="font-mono"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="obj-id">Object ID</Label>
                <Input
                  id="obj-id"
                  value={newObjectId}
                  onChange={(e) => setNewObjectId(e.target.value)}
                  placeholder="my-counter-1"
                  className="font-mono"
                />
              </div>
              <div className="grid gap-2">
                <Label>Sandbox (optional)</Label>
                <SandboxPicker
                  value={newSandbox}
                  onChange={setNewSandbox}
                />
              </div>
            </div>
            <DialogFooter>
              <Button
                onClick={() => createMutation.mutate()}
                disabled={
                  !newClass.trim() ||
                  !newObjectId.trim() ||
                  createMutation.isPending
                }
              >
                {createMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Objects</CardTitle>
          <CardDescription>
            Durable objects provide stateful, single-instance actors
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          )}

          {error && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
                <div className="flex-1 space-y-2">
                  <p className="text-sm font-medium text-destructive">
                    Failed to load objects
                  </p>
                  <p className="text-xs text-destructive/80">
                    {error instanceof Error ? error.message : String(error)}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => refetch()}
                    disabled={isRefetching}
                    className="mt-1"
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 mr-1.5 ${isRefetching ? "animate-spin" : ""}`}
                    />
                    {isRefetching ? "Retrying..." : "Retry"}
                  </Button>
                </div>
              </div>
            </div>
          )}

          {objects && objects.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Class</TableHead>
                  <TableHead>Object ID</TableHead>
                  <TableHead>Sandbox</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Updated</TableHead>
                  <TableHead className="w-[80px] text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {objects.map((obj) => (
                  <TableRow key={obj.id}>
                    <TableCell>
                      <button
                        onClick={() => setSelectedObjectId(obj.id)}
                        className="font-mono text-sm text-primary hover:underline text-left"
                      >
                        {obj.class}
                      </button>
                    </TableCell>
                    <TableCell>
                      <button
                        onClick={() => setSelectedObjectId(obj.id)}
                        className="font-mono text-sm text-primary hover:underline text-left"
                      >
                        {obj.object_id}
                      </button>
                    </TableCell>
                    <TableCell>
                      {obj.sandbox ? (
                        <Link
                          to={`/sandboxes/${obj.sandbox}`}
                          className="text-primary hover:underline text-sm"
                        >
                          {obj.sandbox}
                        </Link>
                      ) : (
                        <span className="text-muted-foreground text-sm">—</span>
                      )}
                    </TableCell>
                    <TableCell>{statusBadge(obj.status)}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {new Date(obj.updated_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => deleteMutation.mutate(obj.id)}
                        disabled={deleteMutation.isPending}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}

          {objects && objects.length === 0 && !isLoading && (
            <div className="flex flex-col items-center gap-2 py-4 text-center">
              <Blocks className="h-8 w-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">
                No durable objects yet
              </p>
              <p className="text-xs text-muted-foreground/70">
                Create one using the button above or via the API.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
