import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useSandbox(name: string) {
  return useQuery({
    queryKey: ["sandbox", name],
    queryFn: () => api.getSandbox(name),
    enabled: !!name,
    refetchInterval: 3000,
    retry: true,
    retryDelay: 1000,
  });
}
