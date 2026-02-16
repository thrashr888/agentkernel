import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useLlmUsage() {
  return useQuery({
    queryKey: ["llm-usage"],
    queryFn: api.getLlmUsage,
    refetchInterval: 5000,
  });
}

export function useLlmUsageBySandbox(sandbox: string) {
  return useQuery({
    queryKey: ["llm-usage", sandbox],
    queryFn: () => api.getLlmUsageBySandbox(sandbox),
    enabled: !!sandbox,
    refetchInterval: 5000,
  });
}
