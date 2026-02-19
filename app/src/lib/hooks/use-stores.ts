import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useStores() {
  return useQuery({
    queryKey: ["stores"],
    queryFn: api.listStores,
    refetchInterval: 5000,
  });
}
