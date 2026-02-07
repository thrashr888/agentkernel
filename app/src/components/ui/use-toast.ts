import { useCallback, useEffect, useState } from "react";

export type ToastType = "default" | "success" | "error";

export interface Toast {
  id: string;
  message: string;
  type: ToastType;
}

let toastListeners: Array<(toasts: Toast[]) => void> = [];
let toasts: Toast[] = [];
let toastCount = 0;

function notifyListeners() {
  for (const listener of toastListeners) {
    listener([...toasts]);
  }
}

function addToast(message: string, type: ToastType = "default") {
  const id = `toast-${++toastCount}`;
  const toast: Toast = { id, message, type };
  toasts = [...toasts, toast];
  notifyListeners();

  setTimeout(() => {
    dismissToast(id);
  }, 3000);

  return id;
}

function dismissToast(id: string) {
  toasts = toasts.filter((t) => t.id !== id);
  notifyListeners();
}

export function toast(message: string, type: ToastType = "default") {
  return addToast(message, type);
}

toast.success = (message: string) => addToast(message, "success");
toast.error = (message: string) => addToast(message, "error");

export function useToast() {
  const [currentToasts, setCurrentToasts] = useState<Toast[]>(toasts);

  useEffect(() => {
    toastListeners.push(setCurrentToasts);
    return () => {
      toastListeners = toastListeners.filter((l) => l !== setCurrentToasts);
    };
  }, []);

  const dismiss = useCallback((id: string) => {
    dismissToast(id);
  }, []);

  return {
    toasts: currentToasts,
    toast,
    dismiss,
  };
}
