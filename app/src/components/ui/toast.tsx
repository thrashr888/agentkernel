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
        toast.type === "default" && "bg-background text-foreground",
        toast.type === "success" &&
          "border-green-500/50 bg-green-500/10 text-green-700 dark:text-green-400",
        toast.type === "error" &&
          "border-destructive/50 bg-destructive/10 text-destructive"
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
