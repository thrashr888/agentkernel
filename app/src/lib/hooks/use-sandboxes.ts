import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useSandboxes() {
  return useQuery({
    queryKey: ["sandboxes"],
    queryFn: api.listSandboxes,
    refetchInterval: 3000,
  });
}
