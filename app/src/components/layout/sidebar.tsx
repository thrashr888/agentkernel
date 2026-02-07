import { NavLink } from "react-router-dom";
import {
  LayoutDashboard,
  Box,
  FileCode,
  Camera,
  Settings,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Separator } from "@/components/ui/separator";

const navItems = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard },
  { to: "/sandboxes", label: "Sandboxes", icon: Box },
  { to: "/templates", label: "Templates", icon: FileCode },
  { to: "/snapshots", label: "Snapshots", icon: Camera },
  { to: "/settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  return (
    <aside className="flex h-full w-[240px] flex-col border-r bg-muted/40">
      <div className="flex h-12 items-center px-4">
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
