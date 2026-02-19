import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useObjects() {
  return useQuery({
    queryKey: ["objects"],
    queryFn: api.listObjects,
    refetchInterval: 5000,
  });
}
