import { useHealth } from "@/lib/hooks/use-health";
import { cn } from "@/lib/utils";

export function Header() {
  const { isConnected } = useHealth();

  return (
    <header className="flex h-12 items-center justify-between border-b px-4">
      <div>{/* Spacer / page title slot */}</div>
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span
            className={cn(
              "inline-block h-2 w-2 rounded-full",
              isConnected ? "bg-green-500" : "bg-red-500"
            )}
          />
          {isConnected ? "Connected" : "Disconnected"}
        </div>
      </div>
    </header>
  );
}
