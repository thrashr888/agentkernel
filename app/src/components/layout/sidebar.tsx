import { NavLink } from "react-router-dom";
import {
  LayoutDashboard,
  Box,
  FileCode,
  Camera,
  ScrollText,
  Activity,
  Shield,
  Puzzle,
  KeyRound,
  Settings,
} from "lucide-react";
import { cn } from "@/lib/utils";
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

const navItems = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard },
  { to: "/sandboxes", label: "Sandboxes", icon: Box },
  { to: "/templates", label: "Templates", icon: FileCode },
  { to: "/snapshots", label: "Snapshots", icon: Camera },
  { to: "/plugins", label: "Plugins", icon: Puzzle },
  { to: "/audit", label: "Audit Log", icon: ScrollText },
  { to: "/diagnostics", label: "Diagnostics", icon: Activity },
  { to: "/policy", label: "Policy", icon: Shield },
  { to: "/secrets", label: "Secrets", icon: KeyRound },
  { to: "/settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  return (
    <aside className="flex h-full w-[240px] flex-col border-r bg-muted/40">
      <div className="flex h-12 items-center gap-2 px-4">
        <AKLogo className="h-5 w-5 shrink-0" />
        <span className="text-lg font-semibold tracking-tight">
          AgentKernel
        </span>
      </div>
      <Separator />
      <nav className="flex flex-1 flex-col gap-1 p-2">
        {navItems.map((item) => (
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
      </nav>
    </aside>
  );
}
