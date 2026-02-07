import { useState, useEffect } from "react";
import { Eye, EyeOff, CheckCircle, XCircle } from "lucide-react";
import { useSettings } from "@/lib/hooks/use-settings";
import { api } from "@/lib/api";
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
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import type { Settings as SettingsType } from "@/lib/types";

export function Settings() {
  const { settings, isLoading, error, saveSettings, isSaving, saveError } =
    useSettings();

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

  function handleSave() {
    saveSettings({
      api_url: formApiUrl,
      api_key: formApiKey,
      theme: formTheme,
      poll_interval_ms: formPollInterval * 1000,
    });
  }

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
              onValueChange={(v) =>
                setFormTheme(v as SettingsType["theme"])
              }
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
              className="w-[200px]"
            />
            <p className="text-xs text-muted-foreground">
              How often to refresh sandbox status data
            </p>
          </div>
        </CardContent>
      </Card>

      <Separator />

      <div className="flex items-center gap-3">
        <Button onClick={handleSave} disabled={isSaving}>
          {isSaving ? "Saving..." : "Save Settings"}
        </Button>
        {saveError && (
          <p className="text-sm text-destructive">{saveError.message}</p>
        )}
      </div>
    </div>
  );
}
