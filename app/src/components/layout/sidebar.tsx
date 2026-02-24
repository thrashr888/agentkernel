import { useEffect, useState } from "react";
import { NavLink, Link } from "react-router-dom";
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
  Github,
  ExternalLink,
  ShieldCheck,
  ShieldOff,
} from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-shell";
import { cn } from "@/lib/utils";
import { useHealth } from "@/lib/hooks/use-health";
import { api } from "@/lib/api";
import { Separator } from "@/components/ui/separator";

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
    label: "System",
    items: [
      { to: "/audit", label: "Audit Log", icon: ScrollText },
      { to: "/diagnostics", label: "Diagnostics", icon: Activity },
      { to: "/settings", label: "Settings", icon: Settings },
    ],
  },
];

export function Sidebar() {
  const { isConnected } = useHealth();
  const [appVersion, setAppVersion] = useState<string>("");
  const [serverVersion, setServerVersion] = useState<string>("");
  const [backend, setBackend] = useState<string>("");
  const [apiUrl, setApiUrl] = useState<string>("");
  const [policyEnabled, setPolicyEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
    api.getSettings().then((s) => setApiUrl(s.api_url)).catch(() => {});
  }, []);

  useEffect(() => {
    if (isConnected) {
      api.getStatus().then((s) => { setServerVersion(s.version); setBackend(s.backend); }).catch(() => {});
      api.getPolicyStatus().then((s) => setPolicyEnabled(s.enabled)).catch(() => setPolicyEnabled(null));
    }
  }, [isConnected]);

  return (
    <aside className="flex h-full w-[240px] flex-col border-r bg-muted/40">
      <div className="flex h-12 items-center gap-2 px-4">
        <AKLogo className="h-5 w-5 shrink-0" />
        <span className="text-lg font-semibold tracking-tight">
          AgentKernel
        </span>
      </div>
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
            onClick={() => open("https://github.com/thrashr888/agentkernel")}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <Github className="h-3 w-3" />
            GitHub
            <ExternalLink className="h-2.5 w-2.5" />
          </button>
        </div>
      </div>
    </aside>
  );
}
