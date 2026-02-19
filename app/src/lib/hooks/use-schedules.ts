import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useSchedules() {
  return useQuery({
    queryKey: ["schedules"],
    queryFn: api.listSchedules,
    refetchInterval: 5000,
  });
}
