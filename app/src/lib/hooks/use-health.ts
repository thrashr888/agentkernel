import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useHealth() {
  const query = useQuery({
    queryKey: ["health"],
    queryFn: api.checkConnection,
    refetchInterval: 5000,
    retry: false,
  });

  return {
    ...query,
    isConnected: query.data === "ok",
  };
}
