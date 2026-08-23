import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Download, HardDrive, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
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
} from "@/components/ui/dialog";

export function Images() {
  const queryClient = useQueryClient();
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [pullTarget, setPullTarget] = useState("alpine:3.24");
  const [pruneTarget, setPruneTarget] = useState<"agentkernel" | "all" | null>(null);

  const {
    data: images,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["images"],
    queryFn: () => api.listImages(),
    refetchInterval: 15000,
  });

  const {
    data: diskUsage,
    error: diskUsageError,
    refetch: refetchDiskUsage,
  } = useQuery({
    queryKey: ["image-disk-usage"],
    queryFn: () => api.imageDiskUsage(),
    refetchInterval: 15000,
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.removeImage(id),
    onMutate: () => {
      return { toastId: toast("Removing image...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Image removed!", "success");
      queryClient.invalidateQueries({ queryKey: ["images"] });
      queryClient.invalidateQueries({ queryKey: ["image-disk-usage"] });
      setDeleteTarget(null);
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

  const pullMutation = useMutation({
    mutationFn: () => api.pullImage(pullTarget.trim()),
    onMutate: () => ({ toastId: toast(`Pulling ${pullTarget.trim()}...`) }),
    onSuccess: (_message, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Image pulled successfully!", "success");
      queryClient.invalidateQueries({ queryKey: ["images"] });
      queryClient.invalidateQueries({ queryKey: ["image-disk-usage"] });
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) {
        toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
      }
    },
  });

  const pruneMutation = useMutation({
    mutationFn: (agentkernelOnly: boolean) => api.pruneImages(agentkernelOnly),
    onMutate: (agentkernelOnly) => ({
      toastId: toast(agentkernelOnly ? "Pruning AgentKernel images..." : "Pruning dangling images..."),
    }),
    onSuccess: (message, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, message || "Image cleanup complete!", "success");
      queryClient.invalidateQueries({ queryKey: ["images"] });
      queryClient.invalidateQueries({ queryKey: ["image-disk-usage"] });
      setPruneTarget(null);
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) {
        toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
      }
    },
  });

  function handleRefresh() {
    refetch();
    refetchDiskUsage();
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[400px] rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Images</h1>
          <p className="text-muted-foreground">
            Manage cached Docker images used by sandboxes
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={handleRefresh}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Pull an image</CardTitle>
          <CardDescription>Download an image into the local runtime cache before creating a sandbox.</CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="flex flex-col gap-3 sm:flex-row"
            onSubmit={(event) => {
              event.preventDefault();
              if (pullTarget.trim()) pullMutation.mutate();
            }}
          >
            <Input
              value={pullTarget}
              onChange={(event) => setPullTarget(event.target.value)}
              placeholder="e.g. python:3.12-alpine"
              aria-label="Docker image to pull"
              className="font-mono"
            />
            <Button type="submit" disabled={!pullTarget.trim() || pullMutation.isPending}>
              {pullMutation.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Download className="mr-2 h-4 w-4" />
              )}
              {pullMutation.isPending ? "Pulling..." : "Pull"}
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-lg">
            <HardDrive className="h-4 w-4" />
            Disk usage
          </CardTitle>
          <CardDescription>Space currently used by the container runtime.</CardDescription>
        </CardHeader>
        <CardContent>
          {diskUsageError ? (
            <p className="text-sm text-muted-foreground">
              Disk usage unavailable: {diskUsageError instanceof Error ? diskUsageError.message : String(diskUsageError)}
            </p>
          ) : diskUsage && diskUsage.length > 0 ? (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {diskUsage.map((entry) => (
                <div key={entry.type} className="rounded-md border p-3">
                  <p className="text-sm font-medium">{entry.type}</p>
                  <p className="mt-1 text-xl font-semibold tabular-nums">{entry.size}</p>
                  <p className="text-xs text-muted-foreground">
                    {entry.active} active of {entry.total}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {entry.reclaimable} reclaimable
                  </p>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">No disk usage reported.</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Clean up unused images</CardTitle>
          <CardDescription>Remove images that are no longer needed to free local disk space.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row">
          <Button variant="outline" onClick={() => setPruneTarget("agentkernel")}>
            <Trash2 className="mr-2 h-4 w-4" />
            Prune AgentKernel images
          </Button>
          <Button variant="outline" onClick={() => setPruneTarget("all")}>
            <Trash2 className="mr-2 h-4 w-4 text-destructive" />
            Prune all dangling images
          </Button>
        </CardContent>
      </Card>

      {error ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load images: {error instanceof Error ? error.message : String(error)}
            </p>
          </CardContent>
        </Card>
      ) : !images || images.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <p className="text-muted-foreground">
              No cached images found.
            </p>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Repository</TableHead>
                  <TableHead>Tag</TableHead>
                  <TableHead>Size</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead className="w-[60px]">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {images.map((image) => (
                  <TableRow key={image.id}>
                    <TableCell className="font-mono text-sm">{image.repository}</TableCell>
                    <TableCell className="font-mono text-sm text-muted-foreground">
                      {image.tag}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">{image.size}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">{image.created}</TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Remove image"
                        onClick={() => setDeleteTarget(image.id)}
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <Dialog open={!!deleteTarget} onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove Image</DialogTitle>
            <DialogDescription>
              This will remove the image from the local cache. It will be re-downloaded
              when next needed.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={deleteMutation.isPending}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteTarget && deleteMutation.mutate(deleteTarget)}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending ? "Removing..." : "Remove"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!pruneTarget} onOpenChange={(open) => { if (!open) setPruneTarget(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {pruneTarget === "all" ? "Prune all dangling images?" : "Prune AgentKernel images?"}
            </DialogTitle>
            <DialogDescription>
              {pruneTarget === "all"
                ? "This removes every dangling image in the container runtime. Images used by running containers are kept."
                : "This removes unused images created for AgentKernel sandboxes. Images referenced by saved sandboxes are kept."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPruneTarget(null)} disabled={pruneMutation.isPending}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => pruneTarget && pruneMutation.mutate(pruneTarget === "agentkernel")}
              disabled={pruneMutation.isPending}
            >
              {pruneMutation.isPending ? "Pruning..." : "Prune"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
