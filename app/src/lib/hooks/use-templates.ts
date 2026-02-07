import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function useTemplates() {
  return useQuery({
    queryKey: ["templates"],
    queryFn: api.listTemplates,
  });
}
