import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle,
  XCircle,
  AlertTriangle,
  Info,
  Loader2,
  ArrowRight,
  ArrowLeft,
  Wifi,
  Box,
  Stethoscope,
} from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const SETUP_KEY = "agentkernel_setup_complete";

export function isSetupComplete(): boolean {
  return localStorage.getItem(SETUP_KEY) === "true";
}

export function markSetupComplete(): void {
  localStorage.setItem(SETUP_KEY, "true");
}

interface SetupWizardProps {
  open: boolean;
  onComplete: () => void;
}

export function SetupWizard({ open, onComplete }: SetupWizardProps) {
  const [step, setStep] = useState(0);
  const [apiUrl, setApiUrl] = useState("http://localhost:18888");
  const [sandboxName, setSandboxName] = useState("my-first-sandbox");
  const [startingServer, setStartingServer] = useState(false);
  const queryClient = useQueryClient();

  // Step 1: Check Docker / health
  const {
    data: doctor,
    isLoading: doctorLoading,
    refetch: refetchDoctor,
  } = useQuery({
    queryKey: ["setup-doctor"],
    queryFn: () => api.getDoctor(),
    enabled: step === 0,
    retry: false,
  });

  const backendReady = doctor?.checks.some(
    (check) => check.name === "backend" && check.status === "ok",
  ) ?? false;

  async function refreshDoctorUntilReady() {
    for (let attempt = 0; attempt < 6; attempt += 1) {
      const result = await refetchDoctor();
      const ready = result.data?.checks.some(
        (check) => check.name === "backend" && check.status === "ok",
      );
      if (ready || attempt === 5) return;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }

  // Step 2: Test connection
  const [connectionOk, setConnectionOk] = useState<boolean | null>(null);
  const [testingConnection, setTestingConnection] = useState(false);

  async function testConnection() {
    setTestingConnection(true);
    setConnectionOk(null);
    try {
      const settings = await api.getSettings();
      const activeServer = settings.active_server ?? settings.servers[0]?.name ?? "Local";
      const servers = settings.servers.length > 0 ? [...settings.servers] : [
        { name: activeServer, url: apiUrl },
      ];
      const serverIndex = servers.findIndex((server) => server.name === activeServer);
      if (serverIndex >= 0) {
        servers[serverIndex] = { ...servers[serverIndex], url: apiUrl };
      }
      await api.saveSettings({
        ...settings,
        active_server: activeServer,
        servers,
        api_url: apiUrl,
      });
      const result = await api.checkConnection();
      setConnectionOk(result === "ok");
    } catch {
      setConnectionOk(false);
    } finally {
      setTestingConnection(false);
    }
  }

  const prepareBackendMutation = useMutation({
    mutationFn: () => api.prepareBackend(),
    onSuccess: async (message) => {
      toast.success(message);
      await refreshDoctorUntilReady();
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  async function startServerAndRetry() {
    setStartingServer(true);
    try {
      await api.startServer();
      await refreshDoctorUntilReady();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    } finally {
      setStartingServer(false);
    }
  }

  // Step 3: Create sandbox
  const createMutation = useMutation({
    mutationFn: () =>
      api.createSandbox({
        name: sandboxName,
        image: "alpine:3.24",
        vcpus: 2,
        memory_mb: 512,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      toast.success("Sandbox created!");
      handleFinish();
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  function handleFinish() {
    markSetupComplete();
    onComplete();
  }

  const steps = [
    { label: "Health Check", icon: Stethoscope },
    { label: "Connection", icon: Wifi },
    { label: "First Sandbox", icon: Box },
  ];

  return (
    <Dialog open={open} onOpenChange={() => {}}>
      <DialogContent className="sm:max-w-[500px]" onPointerDownOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>Welcome to AgentKernel</DialogTitle>
          <DialogDescription>
            Let's get you set up in a few quick steps.
          </DialogDescription>
        </DialogHeader>

        {/* Step indicator */}
        <div className="flex items-center justify-center gap-2 py-2">
          {steps.map((s, i) => (
            <div
              key={i}
              className={`flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium ${
                i === step
                  ? "bg-primary text-primary-foreground"
                  : i < step
                    ? "bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300"
                    : "bg-muted text-muted-foreground"
              }`}
            >
              <s.icon className="h-3 w-3" />
              {s.label}
            </div>
          ))}
        </div>

        <div className="min-h-[160px] py-4">
          {/* Step 0: Health Check */}
          {step === 0 && (
            <div className="space-y-4">
              <p className="text-sm text-muted-foreground">
                Checking the AgentKernel server and a local sandbox runtime.
              </p>
              {doctorLoading ? (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Running health checks...
                </div>
              ) : doctor ? (
                <div className="space-y-2">
                  {doctor.checks.map((check) => (
                    <div key={check.name} className="flex items-center gap-2 text-sm">
                      {check.status === "ok" ? (
                        <CheckCircle className="h-4 w-4 text-green-500" />
                      ) : check.status === "warning" ? (
                        <AlertTriangle className="h-4 w-4 text-yellow-500" />
                      ) : check.status === "info" ? (
                        <Info className="h-4 w-4 text-muted-foreground" />
                      ) : (
                        <XCircle className="h-4 w-4 text-red-500" />
                      )}
                      <span className="font-medium">{check.name}</span>
                      <span className="text-muted-foreground">{check.message}</span>
                    </div>
                  ))}
                  {!backendReady && (
                    <div
                      className="mt-4 space-y-3 rounded-md border border-yellow-500/40 bg-yellow-500/10 p-3"
                      role="alert"
                    >
                      <div>
                        <p className="text-sm font-medium">No sandbox backend is ready</p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          AgentKernel can prepare Apple Containers on macOS or help start Docker
                          Desktop. The first sandbox cannot be created until one is ready.
                        </p>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          size="sm"
                          onClick={() => prepareBackendMutation.mutate()}
                          disabled={prepareBackendMutation.isPending}
                        >
                          {prepareBackendMutation.isPending ? (
                            <>
                              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                              Preparing...
                            </>
                          ) : (
                            "Prepare Backend"
                          )}
                        </Button>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => refetchDoctor()}
                          disabled={doctorLoading}
                        >
                          Check Again
                        </Button>
                      </div>
                    </div>
                  )}
                </div>
              ) : (
                <div className="space-y-3">
                  <div className="flex items-center gap-2 text-sm text-destructive">
                    <XCircle className="h-4 w-4" />
                    Could not connect to the AgentKernel server.
                  </div>
                  <p className="text-xs text-muted-foreground">
                    The server needs to be running before health checks can proceed.
                  </p>
                  <div className="flex items-center gap-2">
                    <Button
                      variant="default"
                      size="sm"
                      onClick={startServerAndRetry}
                      disabled={startingServer}
                    >
                      {startingServer ? "Starting..." : "Start Server"}
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => refetchDoctor()}>
                      Retry
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Step 1: Connection */}
          {step === 1 && (
            <div className="space-y-4">
              <p className="text-sm text-muted-foreground">
                Confirm the AgentKernel server URL. The default works for local development.
              </p>
              <div className="grid gap-2">
                <Label htmlFor="setup-api-url">API URL</Label>
                <Input
                  id="setup-api-url"
                  value={apiUrl}
                  onChange={(e) => setApiUrl(e.target.value)}
                  placeholder="http://localhost:18888"
                />
              </div>
              <div className="flex items-center gap-3">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={testConnection}
                  disabled={testingConnection}
                >
                  {testingConnection ? "Testing..." : "Test Connection"}
                </Button>
                {connectionOk === true && (
                  <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
                    <CheckCircle className="h-4 w-4" />
                    Connected
                  </span>
                )}
                {connectionOk === false && (
                  <span className="flex items-center gap-1 text-sm text-destructive">
                    <XCircle className="h-4 w-4" />
                    Failed
                  </span>
                )}
              </div>
            </div>
          )}

          {/* Step 2: Create first sandbox */}
          {step === 2 && (
            <div className="space-y-4">
              <p className="text-sm text-muted-foreground">
                Create your first sandbox to verify everything works. You can skip this step.
              </p>
              <div className="grid gap-2">
                <Label htmlFor="setup-sandbox-name">Sandbox Name</Label>
                <Input
                  id="setup-sandbox-name"
                  value={sandboxName}
                  onChange={(e) => setSandboxName(e.target.value)}
                  placeholder="my-first-sandbox"
                />
              </div>
            </div>
          )}
        </div>

        <DialogFooter className="flex items-center justify-between sm:justify-between">
          <div>
            {step > 0 && (
              <Button variant="ghost" onClick={() => setStep(step - 1)}>
                <ArrowLeft className="mr-1 h-4 w-4" />
                Back
              </Button>
            )}
          </div>
          <div className="flex items-center gap-2">
            {step === 2 && (
              <Button variant="ghost" onClick={handleFinish}>
                Skip
              </Button>
            )}
            {step < 2 ? (
              <Button
                onClick={() => setStep(step + 1)}
                disabled={step === 0 ? !backendReady : connectionOk !== true}
              >
                Next
                <ArrowRight className="ml-1 h-4 w-4" />
              </Button>
            ) : (
              <Button
                onClick={() => createMutation.mutate()}
                disabled={createMutation.isPending || !sandboxName.trim()}
              >
                {createMutation.isPending ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Creating...
                  </>
                ) : (
                  "Create Sandbox"
                )}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
