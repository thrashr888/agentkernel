import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api";

interface ExecParams {
  name: string;
  command: string[];
  env?: string[];
  workdir?: string;
}

export function useExec() {
  return useMutation({
    mutationFn: ({ name, command, env, workdir }: ExecParams) =>
      api.execCommand(name, command, env, workdir),
  });
}
