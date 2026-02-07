import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useSnapshots } from "@/lib/hooks/use-snapshots";
import { api } from "@/lib/api";
import { Trash2, RotateCcw } from "lucide-react";
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
} from "@/components/ui/dialog";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeDate } from "@/lib/utils";

type ConfirmAction =
  | { type: "restore"; name: string }
  | { type: "delete"; name: string }
  | null;

export function Snapshots() {
  const { data: snapshots, isLoading, error } = useSnapshots();
  const queryClient = useQueryClient();
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);

  const restoreMutation = useMutation({
    mutationFn: (name: string) => api.restoreSnapshot(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      queryClient.invalidateQueries({ queryKey: ["snapshots"] });
      setConfirmAction(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (name: string) => api.deleteSnapshot(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["snapshots"] });
      setConfirmAction(null);
    },
  });

  const isPending = restoreMutation.isPending || deleteMutation.isPending;
  const actionError = restoreMutation.error || deleteMutation.error;

  function handleConfirm() {
    if (!confirmAction) return;
    if (confirmAction.type === "restore") {
      restoreMutation.mutate(confirmAction.name);
    } else {
      deleteMutation.mutate(confirmAction.name);
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Snapshots</h1>
        <p className="text-muted-foreground">
          Saved sandbox states that can be restored
        </p>
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
              Failed to load snapshots: {error.message}
            </p>
          </CardContent>
        </Card>
      ) : !snapshots || snapshots.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <p className="text-muted-foreground">
              No snapshots found. Take a snapshot from a sandbox to save its
              state.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Source Sandbox</TableHead>
                <TableHead>Backend</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[120px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {snapshots.map((snapshot) => (
                <TableRow key={snapshot.name}>
                  <TableCell className="font-medium">
                    {snapshot.name}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {snapshot.sandbox}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {snapshot.backend}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatRelativeDate(snapshot.created_at)}
                  </TableCell>
                  <TableCell>
                    <div className="flex gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Restore snapshot"
                        onClick={() =>
                          setConfirmAction({
                            type: "restore",
                            name: snapshot.name,
                          })
                        }
                      >
                        <RotateCcw className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Delete snapshot"
                        onClick={() =>
                          setConfirmAction({
                            type: "delete",
                            name: snapshot.name,
                          })
                        }
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <Dialog
        open={!!confirmAction}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirmAction?.type === "restore"
                ? "Restore Snapshot"
                : "Delete Snapshot"}
            </DialogTitle>
            <DialogDescription>
              {confirmAction?.type === "restore"
                ? `This will create a new sandbox from snapshot "${confirmAction.name}". Continue?`
                : `This will permanently delete snapshot "${confirmAction?.name}". This action cannot be undone.`}
            </DialogDescription>
          </DialogHeader>
          {actionError && (
            <p className="text-sm text-destructive">{actionError.message}</p>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setConfirmAction(null)}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button
              variant={
                confirmAction?.type === "delete" ? "destructive" : "default"
              }
              onClick={handleConfirm}
              disabled={isPending}
            >
              {isPending
                ? confirmAction?.type === "restore"
                  ? "Restoring..."
                  : "Deleting..."
                : confirmAction?.type === "restore"
                  ? "Restore"
                  : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
