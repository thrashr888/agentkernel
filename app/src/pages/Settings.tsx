import { useState, useEffect, useRef, useCallback } from "react";
import { Eye, EyeOff, CheckCircle, XCircle, Trash2 } from "lucide-react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useSettings } from "@/lib/hooks/use-settings";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { Settings as SettingsType } from "@/lib/types";

export function Settings() {
  const [showSaved, setShowSaved] = useState(false);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const onSaved = useCallback(() => {
    setShowSaved(true);
    clearTimeout(savedTimerRef.current);
    savedTimerRef.current = setTimeout(() => setShowSaved(false), 2000);
  }, []);

  useEffect(() => {
    return () => clearTimeout(savedTimerRef.current);
  }, []);

  const { settings, isLoading, error, saveSettings } = useSettings({
    onSaved,
  });

  const [formApiUrl, setFormApiUrl] = useState("http://localhost:18888");
  const [formApiKey, setFormApiKey] = useState("");
  const [formTheme, setFormTheme] = useState<SettingsType["theme"]>("system");
  const [formPollInterval, setFormPollInterval] = useState(3);
  const [showApiKey, setShowApiKey] = useState(false);
  const [connectionResult, setConnectionResult] = useState<
    "success" | "failed" | null
  >(null);
  const [testingConnection, setTestingConnection] = useState(false);

  useEffect(() => {
    if (settings) {
      setFormApiUrl(settings.api_url);
      setFormApiKey(settings.api_key);
      setFormTheme(settings.theme);
      setFormPollInterval(Math.round(settings.poll_interval_ms / 1000));
    }
  }, [settings]);

  async function handleTestConnection() {
    setTestingConnection(true);
    setConnectionResult(null);
    try {
      const result = await api.checkConnection();
      setConnectionResult(result ? "success" : "failed");
    } catch {
      setConnectionResult("failed");
    } finally {
      setTestingConnection(false);
    }
  }

  const saveCurrentSettings = useCallback(() => {
    saveSettings({
      api_url: formApiUrl,
      api_key: formApiKey,
      theme: formTheme,
      poll_interval_ms: formPollInterval * 1000,
    });
  }, [saveSettings, formApiUrl, formApiKey, formTheme, formPollInterval]);

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[400px] rounded-lg" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-6">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Settings</h1>
        </div>
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load settings: {error.message}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Settings</h1>
        <p className="text-muted-foreground">
          Configure your AgentKernel desktop application
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Connection</CardTitle>
          <CardDescription>
            Configure the connection to your AgentKernel API server
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-2">
            <Label htmlFor="api-url">API URL</Label>
            <Input
              id="api-url"
              value={formApiUrl}
              onChange={(e) => setFormApiUrl(e.target.value)}
              onBlur={saveCurrentSettings}
              placeholder="http://localhost:18888"
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="api-key">API Key</Label>
            <div className="relative">
              <Input
                id="api-key"
                type={showApiKey ? "text" : "password"}
                value={formApiKey}
                onChange={(e) => setFormApiKey(e.target.value)}
                onBlur={saveCurrentSettings}
                placeholder="Enter API key"
                className="pr-10"
              />
              <button
                type="button"
                className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                onClick={() => setShowApiKey(!showApiKey)}
              >
                {showApiKey ? (
                  <EyeOff className="h-4 w-4" />
                ) : (
                  <Eye className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <Button
              variant="outline"
              onClick={handleTestConnection}
              disabled={testingConnection}
            >
              {testingConnection ? "Testing..." : "Test Connection"}
            </Button>
            {connectionResult === "success" && (
              <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
                <CheckCircle className="h-4 w-4" />
                Connection successful
              </span>
            )}
            {connectionResult === "failed" && (
              <span className="flex items-center gap-1 text-sm text-destructive">
                <XCircle className="h-4 w-4" />
                Connection failed
              </span>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
          <CardDescription>
            Customize the look and feel of the application
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-2">
            <Label>Theme</Label>
            <Select
              value={formTheme}
              onValueChange={(v) => {
                const newTheme = v as SettingsType["theme"];
                setFormTheme(newTheme);
                saveSettings({
                  api_url: formApiUrl,
                  api_key: formApiKey,
                  theme: newTheme,
                  poll_interval_ms: formPollInterval * 1000,
                });
              }}
            >
              <SelectTrigger className="w-[200px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="light">Light</SelectItem>
                <SelectItem value="dark">Dark</SelectItem>
                <SelectItem value="system">System</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="grid gap-2">
            <Label htmlFor="poll-interval">Poll Interval (seconds)</Label>
            <Input
              id="poll-interval"
              type="number"
              min={1}
              max={60}
              value={formPollInterval}
              onChange={(e) => setFormPollInterval(Number(e.target.value))}
              onBlur={saveCurrentSettings}
              className="w-[200px]"
            />
            <p className="text-xs text-muted-foreground">
              How often to refresh sandbox status data
            </p>
          </div>
        </CardContent>
      </Card>

      <ApiKeysCard />

      {showSaved && (
        <div className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-400 animate-in fade-in duration-300">
          <CheckCircle className="h-4 w-4" />
          Saved
        </div>
      )}
    </div>
  );
}

function ApiKeysCard() {
  const queryClient = useQueryClient();
  const [newName, setNewName] = useState("");
  const [newValue, setNewValue] = useState("");

  const {
    data: secrets,
    isLoading: secretsLoading,
    error: secretsError,
  } = useQuery({
    queryKey: ["secrets"],
    queryFn: () => api.listSecrets(),
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
    <Card>
      <CardHeader>
        <CardTitle>API Keys</CardTitle>
        <CardDescription>
          Manage secrets passed to sandbox environments
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
          <p className="text-sm text-destructive">
            Failed to load secrets:{" "}
            {secretsError instanceof Error
              ? secretsError.message
              : String(secretsError)}
          </p>
        )}

        {secrets && secrets.length > 0 && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead className="w-[100px] text-right">Actions</TableHead>
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
          <p className="text-sm text-muted-foreground">
            No secrets stored yet.
          </p>
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
            disabled={!newName.trim() || !newValue || createMutation.isPending}
          >
            {createMutation.isPending ? "Adding..." : "Add Secret"}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}
