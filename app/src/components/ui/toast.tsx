import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useToast, type Toast } from "./use-toast";

function ToastItem({
  toast,
  onDismiss,
}: {
  toast: Toast;
  onDismiss: (id: string) => void;
}) {
  return (
    <div
      className={cn(
        "pointer-events-auto relative flex w-full items-center justify-between gap-2 overflow-hidden rounded-md border p-4 shadow-lg transition-all",
        "animate-in slide-in-from-bottom-5 fade-in-0",
        toast.type === "default" && "bg-background text-foreground border-border",
        toast.type === "success" &&
          "border-green-600/30 bg-green-50 text-green-800 dark:bg-green-950 dark:text-green-200 dark:border-green-800/50",
        toast.type === "error" &&
          "border-red-600/30 bg-red-50 text-red-800 dark:bg-red-950 dark:text-red-200 dark:border-red-800/50"
      )}
    >
      <p className="text-sm font-medium">{toast.message}</p>
      <button
        onClick={() => onDismiss(toast.id)}
        className="rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
      >
        <X className="h-4 w-4" />
        <span className="sr-only">Dismiss</span>
      </button>
    </div>
  );
}

export function Toaster() {
  const { toasts, dismiss } = useToast();

  if (toasts.length === 0) return null;

  return (
    <div className="fixed bottom-4 right-4 z-[100] flex max-w-[420px] flex-col gap-2">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} onDismiss={dismiss} />
      ))}
    </div>
  );
}
