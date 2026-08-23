import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useHealth() {
  const query = useQuery({
    queryKey: ["health"],
    queryFn: api.checkConnection,
    refetchInterval: 5000,
    // Allow the app-owned sidecar time to bind during application launch.
    retry: 5,
    retryDelay: 1000,
  });

  return {
    ...query,
    isConnected: query.data === "ok",
  };
}
