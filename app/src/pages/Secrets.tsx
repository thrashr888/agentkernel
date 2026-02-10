import { useState } from "react";
import { Trash2, AlertTriangle, KeyRound, RefreshCw } from "lucide-react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
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

export function Secrets() {
  const queryClient = useQueryClient();
  const [newName, setNewName] = useState("");
  const [newValue, setNewValue] = useState("");

  const {
    data: secrets,
    isLoading: secretsLoading,
    error: secretsError,
    refetch: refetchSecrets,
    isRefetching: secretsRefetching,
  } = useQuery({
    queryKey: ["secrets"],
    queryFn: () => api.listSecrets(),
    retry: false,
  });

  const createMutation = useMutation({
    mutationFn: ({ name, value }: { name: string; value: string }) =>
      api.createSecret(name, value),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["secrets"] });
      setNewName("");
      setNewValue("");
      toast.success("Secret stored");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (name: string) => api.deleteSecret(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["secrets"] });
      toast.success("Secret deleted");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  function handleAddSecret(e: React.FormEvent) {
    e.preventDefault();
    const trimmedName = newName.trim();
    if (!trimmedName || !newValue) return;
    createMutation.mutate({ name: trimmedName, value: newValue });
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Secrets</h1>
        <p className="text-muted-foreground">
          Manage API keys and credentials injected into sandboxes
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Stored Secrets</CardTitle>
          <CardDescription>
            Secrets are passed as environment variables to sandbox environments
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {secretsLoading && (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          )}

          {secretsError && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
                <div className="flex-1 space-y-2">
                  <p className="text-sm font-medium text-destructive">
                    Failed to load secrets
                  </p>
                  <p className="text-xs text-destructive/80">
                    {secretsError instanceof Error
                      ? secretsError.message
                      : String(secretsError)}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Make sure the AgentKernel server is running and the API URL
                    is correct in Settings.
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => refetchSecrets()}
                    disabled={secretsRefetching}
                    className="mt-1"
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 mr-1.5 ${secretsRefetching ? "animate-spin" : ""}`}
                    />
                    {secretsRefetching ? "Retrying..." : "Retry"}
                  </Button>
                </div>
              </div>
            </div>
          )}

          {secrets && secrets.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead className="w-[100px] text-right">
                    Actions
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {secrets.map((secret) => (
                  <TableRow key={secret.name}>
                    <TableCell className="font-mono text-sm">
                      {secret.name}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => deleteMutation.mutate(secret.name)}
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

          {secrets && secrets.length === 0 && !secretsLoading && (
            <div className="flex flex-col items-center gap-2 py-4 text-center">
              <KeyRound className="h-8 w-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">
                No secrets stored yet
              </p>
              <p className="text-xs text-muted-foreground/70">
                Add API keys below to inject them as environment variables into
                your sandboxes.
              </p>
            </div>
          )}

          <form onSubmit={handleAddSecret} className="space-y-3 pt-2">
            <div className="grid gap-2">
              <Label htmlFor="secret-name">Name</Label>
              <Input
                id="secret-name"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="ANTHROPIC_API_KEY"
                className="font-mono"
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="secret-value">Value</Label>
              <Input
                id="secret-value"
                type="password"
                value={newValue}
                onChange={(e) => setNewValue(e.target.value)}
                placeholder="sk-..."
              />
            </div>
            <Button
              type="submit"
              variant="outline"
              disabled={
                !newName.trim() || !newValue || createMutation.isPending
              }
            >
              {createMutation.isPending ? "Adding..." : "Add Secret"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
