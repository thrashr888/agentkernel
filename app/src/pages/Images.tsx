import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.removeImage(id),
    onMutate: () => {
      return { toastId: toast("Removing image...") };
    },
    onSuccess: (_data, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, "Image removed!", "success");
      queryClient.invalidateQueries({ queryKey: ["images"] });
      setDeleteTarget(null);
    },
    onError: (err, _vars, context) => {
      if (context?.toastId) toast.update(context.toastId, err instanceof Error ? err.message : String(err), "error");
    },
  });

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
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

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
    </div>
  );
}
