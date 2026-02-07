import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { ExecRequest } from "@/lib/types";

export function useExec() {
  return useMutation({
    mutationFn: (request: ExecRequest) => api.execCommand(request),
  });
}
