import type { AgentKernelOptions } from "./types.js";

const DEFAULT_BASE_URL = "http://localhost:8880";
const DEFAULT_TIMEOUT = 30_000;

/** Resolve configuration from constructor args, env vars, and defaults. */
export function resolveConfig(opts?: AgentKernelOptions) {
  const baseUrl =
    opts?.baseUrl ??
    process.env.AGENTKERNEL_BASE_URL ??
    DEFAULT_BASE_URL;

  const apiKey =
    opts?.apiKey ??
    process.env.AGENTKERNEL_API_KEY ??
    undefined;

  const timeout = opts?.timeout ?? DEFAULT_TIMEOUT;

  return {
    baseUrl: baseUrl.replace(/\/+$/, ""),
    apiKey,
    timeout,
  };
}
