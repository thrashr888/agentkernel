import { useState, useEffect, useRef, useCallback } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Eye,
  EyeOff,
  CheckCircle,
  XCircle,
  Download,
  RefreshCw,
  Loader2,
  Wifi,
  WifiOff,
  Server,
  AlertTriangle,
  Plus,
  Trash2,
  Check,
  Network,
  Power,
} from "lucide-react";
import { useSettings } from "@/lib/hooks/use-settings";
import { useHealth } from "@/lib/hooks/use-health";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Badge } from "@/components/ui/badge";
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
import type { Settings as SettingsType, ServerEntry, SshTunnelConfig } from "@/lib/types";
import {
  classifyLocalServerVersion,
  isLocalServer,
  localServerVersionMessage,
} from "@/lib/server-version";

function isRemoteServerUrl(value: string): boolean {
  try {
    const url = new URL(value);
    const host = url.hostname.toLowerCase();
    return !["localhost", "127.0.0.1", "::1", "[::1]"].includes(host);
  } catch {
    return false;
  }
}

export function Settings() {
  const queryClient = useQueryClient();
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

  const { isConnected } = useHealth();

  const { data: tunnelStatus } = useQuery({
    queryKey: ["tunnel-status"],
    queryFn: () => api.tunnelStatus(),
    refetchInterval: 2000,
  });

  const { data: status } = useQuery({
    queryKey: ["status"],
    queryFn: () => api.getStatus(),
    enabled: isConnected,
    refetchInterval: 10000,
  });

  const { data: doctor } = useQuery({
    queryKey: ["doctor"],
    queryFn: () => api.getDoctor(),
    enabled: isConnected,
    refetchInterval: 30000,
  });

  const backendCheck = doctor?.checks.find((check) => check.name === "backend");

  // Local form state for servers
  const [servers, setServers] = useState<ServerEntry[]>([]);
  const [activeServer, setActiveServer] = useState<string>("");
  const [editingServerIndex, setEditingServerIndex] = useState<number | null>(null);
  const [showApiKeys, setShowApiKeys] = useState<Record<string, boolean>>({});

  // Local form state for appearance
  const [formTheme, setFormTheme] = useState<SettingsType["theme"]>("system");
  const [formPollInterval, setFormPollInterval] = useState(3);

  const [connectionResult, setConnectionResult] = useState<
    "success" | "failed" | null
  >(null);
  const [testingConnection, setTestingConnection] = useState(false);

  const [serverStarting, setServerStarting] = useState(false);
  const [backendPreparing, setBackendPreparing] = useState(false);
  const [appVersion, setAppVersion] = useState("");

  const activeServerEntry = servers.find((server) => server.name === activeServer);
  const appManagedLocalServer = Boolean(
    activeServerEntry &&
      activeServerEntry.managed !== false &&
      isLocalServer(activeServerEntry.url),
  );
  const { data: managedServerRunning = false } = useQuery({
    queryKey: ["managed-server-status"],
    queryFn: api.serverStatus,
    enabled: appManagedLocalServer,
    refetchInterval: 1000,
  });
  const versionStatus = classifyLocalServerVersion(
    appVersion,
    status?.version ?? "",
    activeServerEntry?.url ?? settings?.api_url ?? "",
  );
  const versionMessage = localServerVersionMessage(
    versionStatus,
    appVersion,
    status?.version ?? "",
  );

  const [updateStatus, setUpdateStatus] = useState<
    "idle" | "checking" | "available" | "downloading" | "ready" | "up-to-date" | "error"
  >("idle");
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);

  // Sync from loaded settings
  useEffect(() => {
    if (settings) {
      setServers(settings.servers?.length ? settings.servers : [{
        name: "Local",
        url: settings.api_url || "http://localhost:18888",
        api_key: settings.api_key || undefined,
        managed: true,
      }]);
      setActiveServer(settings.active_server || settings.servers?.[0]?.name || "Local");
      setFormTheme(settings.theme);
      setFormPollInterval(Math.round(settings.poll_interval_ms / 1000));
    }
  }, [settings]);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  function buildSettingsPayload(
    overrides?: Partial<{ servers: ServerEntry[]; activeServer: string; theme: SettingsType["theme"]; pollInterval: number }>
  ): SettingsType {
    const s = overrides?.servers ?? servers;
    const a = overrides?.activeServer ?? activeServer;
    const active = s.find((srv) => srv.name === a);
    return {
      active_server: a,
      servers: s,
      theme: overrides?.theme ?? formTheme,
      poll_interval_ms: (overrides?.pollInterval ?? formPollInterval) * 1000,
      api_url: active?.url ?? "",
      api_key: active?.api_key ?? "",
    };
  }

  function saveCurrentSettings(overrides?: Parameters<typeof buildSettingsPayload>[0]) {
    const payload = buildSettingsPayload(overrides);
    saveSettings(payload);
    queryClient.invalidateQueries();
  }

  function handleAddServer() {
    const name = `Server ${servers.length + 1}`;
    const updated = [...servers, { name, url: "http://localhost:18888", managed: false }];
    setServers(updated);
    setEditingServerIndex(updated.length - 1);
  }

  function handleRemoveServer(name: string) {
    if (servers.length <= 1) return;
    const updated = servers.filter((s) => s.name !== name);
    const newActive = name === activeServer ? updated[0]?.name ?? "" : activeServer;
    setServers(updated);
    setActiveServer(newActive);
    saveCurrentSettings({ servers: updated, activeServer: newActive });
  }

  function handleSwitchServer(name: string) {
    setActiveServer(name);
    setConnectionResult(null);
    saveCurrentSettings({ activeServer: name });
  }

  function handleUpdateServer(index: number, field: keyof ServerEntry, value: string) {
    const updated = [...servers];
    const oldName = updated[index].name;
    updated[index] = { ...updated[index], [field]: value || undefined };
    setServers(updated);
    // Keep activeServer in sync if we renamed the active entry
    if (field === "name" && oldName === activeServer && value) {
      setActiveServer(value);
    }
  }

  function handleUpdateTunnel(index: number, changes: Partial<SshTunnelConfig>) {
    const updated = [...servers];
    const current: SshTunnelConfig = updated[index].ssh_tunnel ?? {
      enabled: false,
      ssh_host: "",
    };
    updated[index] = { ...updated[index], ssh_tunnel: { ...current, ...changes } };
    setServers(updated);
  }

  function handleTunnelPort(index: number, field: "ssh_port" | "remote_port" | "local_port", value: string) {
    handleUpdateTunnel(index, { [field]: value ? Number(value) : undefined });
  }

  async function handleStartTunnel() {
    try {
      await api.startTunnel();
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
      queryClient.invalidateQueries();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleStopTunnel() {
    try {
      await api.stopTunnel();
      await queryClient.invalidateQueries({ queryKey: ["tunnel-status"] });
      queryClient.invalidateQueries();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  }

  function handleServerBlur() {
    setEditingServerIndex(null);
    saveCurrentSettings();
  }

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

  async function handlePrepareBackend() {
    setBackendPreparing(true);
    try {
      const message = await api.prepareBackend();
      toast.success(message);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["doctor"] }),
        queryClient.invalidateQueries({ queryKey: ["status"] }),
        queryClient.invalidateQueries({ queryKey: ["sandboxes"] }),
      ]);
    } catch (prepareError) {
      toast.error(prepareError instanceof Error ? prepareError.message : String(prepareError));
    } finally {
      setBackendPreparing(false);
    }
  }

  async function handleServerToggle() {
    setServerStarting(true);
    try {
      if (managedServerRunning) {
        await api.stopServer();
      } else {
        await api.startServer();
        // Give the child a short window to bind before refreshing health.
        for (let attempt = 0; attempt < 10; attempt += 1) {
          if ((await api.checkConnection().catch(() => "")) === "ok") break;
          await new Promise((resolve) => setTimeout(resolve, 250));
        }
      }
      await queryClient.invalidateQueries({ queryKey: ["managed-server-status"] });
      await queryClient.invalidateQueries({ queryKey: ["health"] });
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["status"] }),
        queryClient.invalidateQueries({ queryKey: ["doctor"] }),
        queryClient.invalidateQueries({ queryKey: ["sandboxes"] }),
      ]);
    } catch (serverError) {
      toast.error(serverError instanceof Error ? serverError.message : String(serverError));
    } finally {
      setServerStarting(false);
    }
  }

  async function handleCheckForUpdates() {
    setUpdateStatus("checking");
    setUpdateError(null);
    setUpdateVersion(null);
    setDownloadProgress(null);
    try {
      const update = await check();
      if (update) {
        setUpdateVersion(update.version);
        setUpdateStatus("available");
      } else {
        setUpdateStatus("up-to-date");
      }
    } catch (e) {
      setUpdateError(e instanceof Error ? e.message : String(e));
      setUpdateStatus("error");
    }
  }

  async function handleDownloadAndInstall() {
    setUpdateStatus("downloading");
    setDownloadProgress(0);
    try {
      const update = await check();
      if (!update) return;

      let totalBytes = 0;
      let downloadedBytes = 0;

      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            totalBytes = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloadedBytes += event.data.chunkLength;
            if (totalBytes > 0) {
              setDownloadProgress(Math.round((downloadedBytes / totalBytes) * 100));
            }
            break;
          case "Finished":
            setDownloadProgress(100);
            break;
        }
      });

      setUpdateStatus("ready");
    } catch (e) {
      setUpdateError(e instanceof Error ? e.message : String(e));
      setUpdateStatus("error");
    }
  }

  async function handleRelaunch() {
    await relaunch();
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
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Servers</CardTitle>
              <CardDescription>
                Manage your AgentKernel server connections
              </CardDescription>
            </div>
            <Button variant="outline" size="sm" onClick={handleAddServer}>
              <Plus className="mr-1 h-4 w-4" />
              Add Server
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {servers.map((server, index) => {
            const isActive = server.name === activeServer;
            const isEditing = editingServerIndex === index;
            const showKey = showApiKeys[server.name] ?? false;

            return (
              <div
                key={index}
                className={`rounded-lg border p-3 space-y-3 transition-colors ${
                  isActive ? "border-primary bg-primary/5" : "border-border"
                }`}
              >
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleSwitchServer(server.name)}
                    className={`flex items-center justify-center h-5 w-5 rounded-full border-2 shrink-0 transition-colors ${
                      isActive
                        ? "border-primary bg-primary text-primary-foreground"
                        : "border-muted-foreground/30 hover:border-primary"
                    }`}
                  >
                    {isActive && <Check className="h-3 w-3" />}
                  </button>

                  {isEditing ? (
                    <Input
                      value={server.name}
                      onChange={(e) => handleUpdateServer(index, "name", e.target.value)}
                      onBlur={handleServerBlur}
                      onKeyDown={(e) => e.key === "Enter" && handleServerBlur()}
                      className="h-7 text-sm font-medium"
                      autoFocus
                    />
                  ) : (
                    <button
                      onClick={() => setEditingServerIndex(index)}
                      className="text-sm font-medium hover:underline"
                    >
                      {server.name}
                    </button>
                  )}

                  {isActive && (
                    <Badge variant="success" className="text-xs">Active</Badge>
                  )}

                  <div className="flex-1" />

                  {servers.length > 1 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive"
                      onClick={() => handleRemoveServer(server.name)}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>

                <div className="grid gap-2">
                  <div className="grid gap-1">
                    <Label className="text-xs text-muted-foreground">URL</Label>
                    <Input
                      value={server.url}
                      onChange={(e) => handleUpdateServer(index, "url", e.target.value)}
                      onBlur={handleServerBlur}
                      placeholder="http://localhost:18888"
                      className="h-8 text-sm font-mono"
                    />
                  </div>
                  <div className="grid gap-1">
                    <Label className="text-xs text-muted-foreground">API Key</Label>
                    <div className="relative">
                      <Input
                        type={showKey ? "text" : "password"}
                        value={server.api_key ?? ""}
                        onChange={(e) => handleUpdateServer(index, "api_key", e.target.value)}
                        onBlur={handleServerBlur}
                        placeholder="Optional"
                        className="h-8 text-sm pr-10"
                      />
                      <button
                        type="button"
                        className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                        onClick={() => setShowApiKeys({ ...showApiKeys, [server.name]: !showKey })}
                      >
                        {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                      </button>
                    </div>
                  </div>
                </div>

                {isRemoteServerUrl(server.url) && (
                  <div className="space-y-3 rounded-md border border-primary/30 bg-primary/5 p-3">
                    <div className="flex items-start gap-2">
                      <Network className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                      <div className="space-y-1">
                        <Label className="text-sm">Private SSH tunnel</Label>
                        <p className="text-xs text-muted-foreground">
                          Keep AgentKernel private on the remote machine. The app uses your OpenSSH config and only forwards a loopback port.
                        </p>
                      </div>
                      <label className="ml-auto flex items-center gap-2 text-xs font-medium">
                        <input
                          type="checkbox"
                          checked={server.ssh_tunnel?.enabled ?? false}
                          onChange={(event) => {
                            const updatedServers = servers.map((entry, entryIndex) =>
                              entryIndex === index
                                ? {
                                    ...entry,
                                    ssh_tunnel: {
                                      ...(entry.ssh_tunnel ?? { ssh_host: "" }),
                                      enabled: event.target.checked,
                                    },
                                  }
                                : entry,
                            );
                            handleUpdateTunnel(index, { enabled: event.target.checked });
                            saveCurrentSettings({ servers: updatedServers });
                          }}
                          className="h-4 w-4 accent-primary"
                        />
                        Manage tunnel
                      </label>
                    </div>

                    {server.ssh_tunnel?.enabled && (
                      <div className="grid gap-2 sm:grid-cols-2">
                        <div className="grid gap-1 sm:col-span-2">
                          <Label className="text-xs text-muted-foreground">SSH host or config alias</Label>
                          <Input
                            value={server.ssh_tunnel.ssh_host}
                            onChange={(event) => handleUpdateTunnel(index, { ssh_host: event.target.value })}
                            onBlur={handleServerBlur}
                            placeholder="rookery"
                            className="h-8 text-sm font-mono"
                          />
                        </div>
                        <div className="grid gap-1">
                          <Label className="text-xs text-muted-foreground">SSH user (optional)</Label>
                          <Input
                            value={server.ssh_tunnel.ssh_user ?? ""}
                            onChange={(event) => handleUpdateTunnel(index, { ssh_user: event.target.value || undefined })}
                            onBlur={handleServerBlur}
                            placeholder="From SSH config"
                            className="h-8 text-sm"
                          />
                        </div>
                        <div className="grid gap-1">
                          <Label className="text-xs text-muted-foreground">SSH port (optional)</Label>
                          <Input
                            type="number"
                            min={1}
                            max={65535}
                            value={server.ssh_tunnel.ssh_port ?? ""}
                            onChange={(event) => handleTunnelPort(index, "ssh_port", event.target.value)}
                            onBlur={handleServerBlur}
                            placeholder="22"
                            className="h-8 text-sm"
                          />
                        </div>
                        <div className="grid gap-1">
                          <Label className="text-xs text-muted-foreground">Remote bind (loopback only)</Label>
                          <Input
                            value={server.ssh_tunnel.remote_host ?? ""}
                            onChange={(event) => handleUpdateTunnel(index, { remote_host: event.target.value || undefined })}
                            onBlur={handleServerBlur}
                            placeholder="127.0.0.1"
                            className="h-8 text-sm font-mono"
                          />
                        </div>
                        <div className="grid gap-1">
                          <Label className="text-xs text-muted-foreground">Remote AgentKernel port</Label>
                          <Input
                            type="number"
                            min={1}
                            max={65535}
                            value={server.ssh_tunnel.remote_port ?? ""}
                            onChange={(event) => handleTunnelPort(index, "remote_port", event.target.value)}
                            onBlur={handleServerBlur}
                            placeholder="From URL or 18888"
                            className="h-8 text-sm"
                          />
                        </div>
                        <div className="grid gap-1">
                          <Label className="text-xs text-muted-foreground">Local port (optional)</Label>
                          <Input
                            type="number"
                            min={1}
                            max={65535}
                            value={server.ssh_tunnel.local_port ?? ""}
                            onChange={(event) => handleTunnelPort(index, "local_port", event.target.value)}
                            onBlur={handleServerBlur}
                            placeholder="Choose automatically"
                            className="h-8 text-sm"
                          />
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}

          <div className="flex items-center gap-3 pt-2">
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
          <CardTitle className="flex items-center gap-2">
            <Server className="h-5 w-5" />
            Server Status
          </CardTitle>
          <CardDescription>
            Status of the active AgentKernel server
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-3">
            <div className="flex items-center justify-between border-b pb-2">
              <span className="text-sm font-medium">Status</span>
              <div className="flex items-center gap-2">
                {isConnected ? (
                  <>
                    <Wifi className="h-4 w-4 text-green-500" />
                    <span className="text-sm text-green-600 dark:text-green-400">Connected</span>
                  </>
                ) : (
                  <>
                    <WifiOff className="h-4 w-4 text-destructive" />
                    <span className="text-sm text-destructive">Disconnected</span>
                  </>
                )}
              </div>
            </div>
            {activeServerEntry?.ssh_tunnel?.enabled && (
              <div className="space-y-2 border-b pb-2">
                <div className="flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm font-medium">
                    <Network className="h-4 w-4" />
                    SSH tunnel
                  </span>
                  <Badge
                    variant={
                      tunnelStatus?.state === "connected"
                        ? "success"
                        : tunnelStatus?.state === "error"
                          ? "destructive"
                          : "secondary"
                    }
                  >
                    {tunnelStatus?.state ?? "starting"}
                  </Badge>
                </div>
                {tunnelStatus?.local_url && (
                  <p className="text-xs text-muted-foreground">
                    Forwarding through {tunnelStatus.local_url}
                  </p>
                )}
                {tunnelStatus?.error && (
                  <p className="text-xs text-destructive">{tunnelStatus.error}</p>
                )}
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleStartTunnel}
                    disabled={tunnelStatus?.state === "starting"}
                  >
                    <Power className="mr-2 h-3.5 w-3.5" />
                    Retry tunnel
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleStopTunnel}
                    disabled={tunnelStatus?.state === "disabled"}
                  >
                    Stop tunnel
                  </Button>
                </div>
              </div>
            )}
            {status && (
              <>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Server Version</span>
                  <span className="font-mono text-sm text-muted-foreground">{status.version}</span>
                </div>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">Backend</span>
                  <span className="font-mono text-sm text-muted-foreground">{status.backend}</span>
                </div>
                <div className="flex items-center justify-between border-b pb-2">
                  <span className="text-sm font-medium">API Key</span>
                  <Badge variant={status.api_key_configured ? "success" : "secondary"}>
                    {status.api_key_configured ? "Configured" : "Not configured"}
                  </Badge>
                </div>
              </>
            )}
            {versionMessage && (
              <div
                role="alert"
                className="flex items-start gap-2 rounded-md border border-yellow-500/40 bg-yellow-500/10 p-3 text-yellow-900 dark:text-yellow-200"
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <div>
                  <p className="text-sm font-medium">Local server version mismatch</p>
                  <p className="mt-1 text-xs">{versionMessage}</p>
                </div>
              </div>
            )}
            {doctor && (
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium">Health</span>
                <Badge variant={doctor.healthy ? "success" : "destructive"}>
                  {doctor.healthy
                    ? `${doctor.checks.filter((c) => c.status === "ok").length}/${doctor.checks.length} checks passing`
                    : "Unhealthy"}
                </Badge>
              </div>
            )}
            <div className="flex items-center justify-between border-t pt-3">
              <div>
                <span className="text-sm font-medium">Desktop lifecycle</span>
                <p className="text-xs text-muted-foreground">
                  {appManagedLocalServer
                    ? "The app owns this local server and stops it when it quits."
                    : "This server is managed outside the desktop app."}
                </p>
              </div>
              {appManagedLocalServer ? (
                <Button
                  variant={managedServerRunning ? "outline" : "default"}
                  size="sm"
                  onClick={() => void handleServerToggle()}
                  disabled={serverStarting}
                >
                  {serverStarting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                  {serverStarting
                    ? managedServerRunning
                      ? "Stopping..."
                      : "Starting..."
                    : managedServerRunning
                      ? "Stop Server"
                      : "Start Server"}
                </Button>
              ) : (
                <Badge variant="secondary">External</Badge>
              )}
            </div>
            {isConnected && backendCheck && backendCheck.status !== "ok" && (
              <div className="space-y-3 rounded-md border border-yellow-500/40 bg-yellow-500/10 p-3">
                <div>
                  <p className="text-sm font-medium">Sandbox backend unavailable</p>
                  <p className="mt-1 text-xs text-muted-foreground">{backendCheck.message}</p>
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="default"
                    size="sm"
                    onClick={handlePrepareBackend}
                    disabled={backendPreparing}
                  >
                    {backendPreparing && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                    {backendPreparing ? "Preparing..." : "Prepare Backend"}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      void queryClient.invalidateQueries({ queryKey: ["doctor"] });
                    }}
                    disabled={backendPreparing}
                  >
                    <RefreshCw className="mr-2 h-4 w-4" />
                    Check Again
                  </Button>
                </div>
              </div>
            )}
            {!isConnected && !appManagedLocalServer && (
              <div className="space-y-2 pt-1">
                <p className="text-xs text-muted-foreground">
                  The external server is not reachable. Start it outside the desktop app or check its connection settings.
                </p>
              </div>
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
                saveCurrentSettings({ theme: newTheme });
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
              onBlur={() => saveCurrentSettings()}
              className="w-[200px]"
            />
            <p className="text-xs text-muted-foreground">
              How often to refresh sandbox status data
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Updates</CardTitle>
          <CardDescription>
            Check for and install application updates
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-3">
            {updateStatus === "idle" && (
              <Button variant="outline" onClick={handleCheckForUpdates}>
                <RefreshCw className="mr-2 h-4 w-4" />
                Check for Updates
              </Button>
            )}
            {updateStatus === "checking" && (
              <Button variant="outline" disabled>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Checking...
              </Button>
            )}
            {updateStatus === "up-to-date" && (
              <>
                <Button variant="outline" onClick={handleCheckForUpdates}>
                  <RefreshCw className="mr-2 h-4 w-4" />
                  Check Again
                </Button>
                <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
                  <CheckCircle className="h-4 w-4" />
                  Up to date
                </span>
              </>
            )}
            {updateStatus === "available" && (
              <>
                <Button onClick={handleDownloadAndInstall}>
                  <Download className="mr-2 h-4 w-4" />
                  Install v{updateVersion}
                </Button>
                <span className="text-sm text-muted-foreground">
                  New version available
                </span>
              </>
            )}
            {updateStatus === "downloading" && (
              <>
                <Button disabled>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Downloading...
                </Button>
                {downloadProgress !== null && (
                  <span className="text-sm text-muted-foreground">
                    {downloadProgress}%
                  </span>
                )}
              </>
            )}
            {updateStatus === "ready" && (
              <Button onClick={handleRelaunch}>
                <RefreshCw className="mr-2 h-4 w-4" />
                Restart to Apply
              </Button>
            )}
            {updateStatus === "error" && (
              <>
                <Button variant="outline" onClick={handleCheckForUpdates}>
                  <RefreshCw className="mr-2 h-4 w-4" />
                  Retry
                </Button>
                <span className="flex items-center gap-1 text-sm text-destructive">
                  <XCircle className="h-4 w-4" />
                  {updateError ?? "Update check failed"}
                </span>
              </>
            )}
          </div>
        </CardContent>
      </Card>

      {showSaved && (
        <div className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-400 animate-in fade-in duration-300">
          <CheckCircle className="h-4 w-4" />
          Saved
        </div>
      )}
    </div>
  );
}
