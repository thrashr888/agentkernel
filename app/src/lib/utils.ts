import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import { format, formatDistanceToNow } from "date-fns";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatDate(date: string | Date): string {
  const d = typeof date === "string" ? new Date(date) : date;
  return format(d, "MMM d, yyyy HH:mm:ss");
}

export function formatRelativeDate(date: string | Date): string {
  const d = typeof date === "string" ? new Date(date) : date;
  return formatDistanceToNow(d, { addSuffix: true });
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function statusColor(status: string): string {
  switch (status.toLowerCase()) {
    case "running":
      return "text-green-500";
    case "stopped":
      return "text-neutral-500";
    case "paused":
      return "text-yellow-500";
    case "error":
    case "failed":
      return "text-red-500";
    case "creating":
    case "starting":
      return "text-blue-500";
    default:
      return "text-muted-foreground";
  }
}
