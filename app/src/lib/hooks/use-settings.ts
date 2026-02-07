import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { Settings } from "@/lib/types";

export function useSettings() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["settings"],
    queryFn: api.getSettings,
  });

  const mutation = useMutation({
    mutationFn: (settings: Settings) => api.saveSettings(settings),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings"] });
    },
  });

  return {
    settings: query.data,
    isLoading: query.isLoading,
    error: query.error,
    saveSettings: mutation.mutate,
    isSaving: mutation.isPending,
    saveError: mutation.error,
  };
}
