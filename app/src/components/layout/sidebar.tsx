import { useEffect, useState } from "react";
import { NavLink, Link } from "react-router-dom";
import { MarkGithubIcon } from "@primer/octicons-react/MarkGithubIcon";
import {
  LayoutDashboard,
  Box,
  FileCode,
  Camera,
  FileCheck2,
  ScrollText,
  Activity,
  Shield,
  ClipboardList,
  Puzzle,
  KeyRound,
  Settings,
  Wifi,
  WifiOff,
  Blocks,
  Timer,
  Database,
  BookOpen,
  ExternalLink,
  ShieldCheck,
  ShieldOff,
  ShieldAlert,
  Layers,
  Zap,
  Terminal,
  HardDrive,
  ChevronsUpDown,
  Check,
  Server,
  AlertTriangle,
} from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-shell";
import { useQueryClient } from "@tanstack/react-query";
import { cn } from "@/lib/utils";
import { useHealth } from "@/lib/hooks/use-health";
import { useSettings } from "@/lib/hooks/use-settings";
import { api } from "@/lib/api";
import { classifyLocalServerVersion, localServerVersionMessage } from "@/lib/server-version";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

function AKLogo({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 1024 1024"
      className={className}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="420" y="160" width="80" height="704" />
      <polygon points="120,864 420,160 500,160 200,864" />
      <rect x="225" y="548" width="275" height="72" />
      <polygon points="500,465 860,160 860,240 500,545" />
      <polygon points="500,479 500,559 860,864 860,784" />
    </svg>
  );
}

const navSections = [
  {
    items: [
      { to: "/", label: "Dashboard", icon: LayoutDashboard },
    ],
  },
  {
    label: "Workflow",
    items: [
      { to: "/sandboxes", label: "Sandboxes", icon: Box },
      { to: "/templates", label: "Templates", icon: FileCode },
      { to: "/snapshots", label: "Snapshots", icon: Camera },
      { to: "/receipts", label: "Receipts", icon: FileCheck2 },
      { to: "/secrets", label: "Secrets", icon: KeyRound },
    ],
  },
  {
    label: "Durable",
    items: [
      { to: "/objects", label: "Objects", icon: Blocks },
      { to: "/schedules", label: "Schedules", icon: Timer },
      { to: "/stores", label: "Stores", icon: Database },
    ],
  },
  {
    label: "Extensions",
    items: [
      { to: "/plugins", label: "Plugins", icon: Puzzle },
      { to: "/policy", label: "Policy", icon: Shield },
      { to: "/policy/log", label: "Policy Log", icon: ClipboardList },
    ],
  },
  {
    label: "Resources",
    items: [
      { to: "/jobs", label: "Jobs", icon: Layers },
      { to: "/images", label: "Images", icon: HardDrive },
      { to: "/sessions", label: "Sessions", icon: Terminal },
    ],
  },
  {
    label: "System",
    items: [
      { to: "/permissions", label: "Permissions", icon: ShieldAlert },
      { to: "/audit", label: "Audit Log", icon: ScrollText },
      { to: "/diagnostics", label: "Diagnostics", icon: Activity },
      { to: "/benchmark", label: "Benchmark", icon: Zap },
      { to: "/settings", label: "Settings", icon: Settings },
    ],
  },
];

export function Sidebar() {
  const { isConnected } = useHealth();
  const { settings, saveSettings } = useSettings();
  const queryClient = useQueryClient();
  const [appVersion, setAppVersion] = useState<string>("");
  const [serverVersion, setServerVersion] = useState<string>("");
  const [backend, setBackend] = useState<string>("");
  const [policyEnabled, setPolicyEnabled] = useState<boolean | null>(null);

  const servers = settings?.servers ?? [];
  const activeServer = settings?.active_server;
  const activeEntry = servers.find((s) => s.name === activeServer);
  const apiUrl = activeEntry?.url ?? settings?.api_url ?? "";
  const versionStatus = classifyLocalServerVersion(appVersion, serverVersion, apiUrl);
  const versionMessage = localServerVersionMessage(versionStatus, appVersion, serverVersion);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  useEffect(() => {
    if (isConnected) {
      api.getStatus().then((s) => { setServerVersion(s.version); setBackend(s.backend); }).catch(() => {});
      api.getPolicyStatus().then((s) => setPolicyEnabled(s.enabled)).catch(() => setPolicyEnabled(null));
    }
  }, [isConnected, activeServer]);

  function switchServer(name: string) {
    if (!settings || name === activeServer) return;
    const updated = { ...settings, active_server: name };
    // Also update legacy fields from the selected server
    const entry = servers.find((s) => s.name === name);
    if (entry) {
      updated.api_url = entry.url;
      updated.api_key = entry.api_key ?? "";
    }
    saveSettings(updated);
    // Reset connection-dependent queries so they refetch against the new server
    queryClient.invalidateQueries();
  }

  return (
    <aside className="flex h-full w-[240px] flex-col border-r bg-muted/40">
      <div className="flex h-12 items-center gap-2 px-4">
        <AKLogo className="h-5 w-5 shrink-0" />
        <span className="text-lg font-semibold tracking-tight">
          AgentKernel
        </span>
      </div>

      {/* Server switcher */}
      {servers.length > 1 && (
        <div className="px-2 pb-1">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="flex w-full items-center gap-2 rounded-md border px-3 py-1.5 text-sm hover:bg-accent transition-colors">
                <Server className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate flex-1 text-left">{activeServer ?? "Select server"}</span>
                <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-[216px]">
              {servers.map((s) => (
                <DropdownMenuItem key={s.name} onClick={() => switchServer(s.name)}>
                  <Check className={cn("mr-2 h-4 w-4", s.name === activeServer ? "opacity-100" : "opacity-0")} />
                  <div className="flex flex-col">
                    <span>{s.name}</span>
                    <span className="text-xs text-muted-foreground font-mono">{s.url.replace(/^https?:\/\//, "")}</span>
                  </div>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      )}
      <Separator />
      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto p-2">
        {navSections.map((section, i) => (
          <div key={i}>
            {i > 0 && <Separator className="my-2" />}
            {section.label && (
              <span className="px-3 py-1 text-xs font-medium uppercase tracking-wider text-muted-foreground/60">
                {section.label}
              </span>
            )}
            {section.items.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                end={item.to === "/"}
                className={({ isActive }) =>
                  cn(
                    "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                    "hover:bg-accent hover:text-accent-foreground",
                    isActive
                      ? "bg-primary/10 text-primary"
                      : "text-muted-foreground"
                  )
                }
              >
                <item.icon className="h-4 w-4" />
                {item.label}
              </NavLink>
            ))}
          </div>
        ))}
      </nav>
      <Separator />
      <div className="px-4 py-3 space-y-2">
        <Link to="/settings" className="flex items-center gap-2 hover:opacity-80 transition-opacity">
          {isConnected ? (
            <Wifi className="h-3.5 w-3.5 text-green-500" />
          ) : (
            <WifiOff className="h-3.5 w-3.5 text-destructive" />
          )}
          <span className="text-xs">
            {isConnected ? (
              <span className="text-green-600 dark:text-green-400">Connected</span>
            ) : (
              <span className="text-destructive">Disconnected</span>
            )}
          </span>
          {apiUrl && (
            <span className="text-xs text-muted-foreground font-mono truncate">
              {apiUrl.replace(/^https?:\/\//, "")}
            </span>
          )}
        </Link>
        <div className="flex items-center gap-3 text-xs text-muted-foreground font-mono flex-wrap">
          {backend && <span>{backend}</span>}
          {policyEnabled !== null && (
            <Link to="/policy" className={`flex items-center gap-1 hover:opacity-80 transition-opacity ${policyEnabled ? "text-green-600 dark:text-green-400" : "text-muted-foreground"}`}>
              {policyEnabled ? <ShieldCheck className="h-3 w-3" /> : <ShieldOff className="h-3 w-3" />}
              {policyEnabled ? "policy" : "no policy"}
            </Link>
          )}
        </div>
        {versionMessage && (
          <Link
            to="/settings"
            className="flex items-start gap-1.5 text-xs text-yellow-700 hover:text-yellow-800 dark:text-yellow-400 dark:hover:text-yellow-300"
            title={versionMessage}
          >
            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
            <span>Local server version mismatch</span>
          </Link>
        )}
        <div className="flex items-center gap-3 text-xs text-muted-foreground font-mono flex-wrap">
          {appVersion && <span>app v{appVersion}</span>}
          {serverVersion && <span>server v{serverVersion}</span>}
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={() => open("https://thrashr888.github.io/agentkernel/")}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <BookOpen className="h-3 w-3" />
            Docs
            <ExternalLink className="h-2.5 w-2.5" />
          </button>
          <button
            type="button"
            aria-label="Open AgentKernel on GitHub"
            onClick={() => open("https://github.com/thrashr888/agentkernel")}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <MarkGithubIcon className="h-3 w-3" size={12} />
            GitHub
            <ExternalLink className="h-2.5 w-2.5" />
          </button>
        </div>
      </div>
    </aside>
  );
}
