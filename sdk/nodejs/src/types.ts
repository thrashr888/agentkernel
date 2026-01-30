/** Security profile for sandbox execution. */
export type SecurityProfile = "permissive" | "moderate" | "restrictive";

/** Sandbox status. */
export type SandboxStatus = "running" | "stopped";

/** SSE event types emitted by /run/stream. */
export type StreamEventType = "started" | "progress" | "output" | "done" | "error";

/** Configuration options for the AgentKernel client. */
export interface AgentKernelOptions {
  /** Base URL of the agentkernel HTTP API. Default: AGENTKERNEL_BASE_URL or http://localhost:18888 */
  baseUrl?: string;
  /** API key for Bearer authentication. Default: AGENTKERNEL_API_KEY env var */
  apiKey?: string;
  /** Request timeout in milliseconds. Default: 30000 */
  timeout?: number;
}

/** Options for the run command. */
export interface RunOptions {
  /** Docker image to use (auto-detected if not specified). */
  image?: string;
  /** Security profile. Default: moderate */
  profile?: SecurityProfile;
  /** Use container pool for faster execution. Default: true */
  fast?: boolean;
}

/** Options for creating a sandbox. */
export interface CreateSandboxOptions {
  /** Docker image to use. Default: alpine:3.20 */
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  profile?: SecurityProfile;
}

/** Output from a command execution. */
export interface RunOutput {
  output: string;
}

/** Information about a sandbox. */
export interface SandboxInfo {
  name: string;
  status: SandboxStatus;
  backend: string;
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  created_at?: string;
}

/** SSE stream event. */
export interface StreamEvent {
  type: StreamEventType;
  data: Record<string, unknown>;
}

/** Options for writing a file. */
export interface FileWriteOptions {
  /** Content encoding: "utf8" (default) or "base64". */
  encoding?: "utf8" | "base64";
}

/** Response from reading a file. */
export interface FileReadResponse {
  content: string;
  encoding: "utf8" | "base64";
  size: number;
}

/** A command for batch execution. */
export interface BatchCommand {
  command: string[];
}

/** Result of a single batch command. */
export interface BatchResult {
  output: string | null;
  error: string | null;
}

/** Response from batch execution. */
export interface BatchRunResponse {
  results: BatchResult[];
}

/** API response wrapper. */
export interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}
