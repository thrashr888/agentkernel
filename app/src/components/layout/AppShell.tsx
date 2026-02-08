import { useEffect } from "react";
import { useSettings } from "@/lib/hooks/use-settings";
import { Sidebar } from "./sidebar";
import { Header } from "./header";
import { Toaster } from "@/components/ui/toast";

interface AppShellProps {
  children: React.ReactNode;
}

export function AppShell({ children }: AppShellProps) {
  const { settings } = useSettings();

  useEffect(() => {
    const root = document.documentElement;
    if (!settings) return;

    if (settings.theme === "dark") {
      root.classList.add("dark");
    } else if (settings.theme === "light") {
      root.classList.remove("dark");
    } else {
      // system
      const prefersDark = window.matchMedia(
        "(prefers-color-scheme: dark)"
      ).matches;
      root.classList.toggle("dark", prefersDark);
    }
  }, [settings?.theme]);

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Header />
        <main className="flex-1 overflow-y-auto p-6">{children}</main>
      </div>
      <Toaster />
    </div>
  );
}
