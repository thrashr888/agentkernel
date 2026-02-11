import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useSnapshots() {
  return useQuery({
    queryKey: ["snapshots"],
    queryFn: api.listSnapshots,
    refetchInterval: 5000,
  });
}
