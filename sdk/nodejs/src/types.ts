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
  /** Git repository URL to clone into the sandbox. */
  source_url?: string;
  /** Git ref to checkout after cloning. */
  source_ref?: string;
  /** Volume mounts (slug:/path or slug:/path:ro). Create volumes via CLI first. */
  volumes?: string[];
  /**
   * Secret bindings for proxy-based injection (Gondolin pattern).
   * Secrets are injected as HTTP headers by the host proxy — they never enter the VM.
   * Formats: "KEY=value:host", "KEY:host", "KEY:host:header"
   */
  secrets?: string[];
  /**
   * Secret keys to inject as files at /run/agentkernel/secrets/KEY.
   * Values are resolved from the secret vault.
   */
  secret_files?: string[];
}

/** Options for executing a command in a sandbox. */
export interface ExecOptions {
  /** Environment variables (KEY=VALUE). */
  env?: string[];
  /** Working directory inside the container. */
  workdir?: string;
  /** Run as root. */
  sudo?: boolean;
}

/** Output from a command execution. */
export interface RunOutput {
  output: string;
}

/** Information about a sandbox. */
export interface SandboxInfo {
  name: string;
  uuid?: string;
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

/** Result of a batch file write. */
export interface BatchFileWriteResponse {
  written: number;
}

/** Opaque orchestration payload/response body. */
export type Orchestration = Record<string, unknown>;
export type OrchestrationDefinition = Record<string, unknown>;

/** Opaque object payload/response body. */
export type DurableObject = Record<string, unknown>;

/** Opaque schedule payload/response body. */
export type Schedule = Record<string, unknown>;

/** Status of a detached command. */
export type DetachedStatus = "running" | "completed" | "failed";

/** A detached (background) command running in a sandbox. */
export interface DetachedCommand {
  id: string;
  sandbox: string;
  command: string[];
  pid: number;
  status: DetachedStatus;
  exit_code: number | null;
  started_at: string;
}

/** Response from detached command logs. */
export interface DetachedLogsResponse {
  stdout?: string;
  stderr?: string;
}

/** Opaque payload used when creating orchestrations. */
export type CreateOrchestrationRequest = Record<string, unknown>;

/** Opaque payload used when creating objects. */
export type CreateObjectRequest = Record<string, unknown>;

/** Opaque payload used when creating schedules. */
export type CreateScheduleRequest = Record<string, unknown>;

/** API response wrapper. */
export interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

/** Response from extending a sandbox's TTL. */
export interface ExtendTtlResponse {
  expires_at?: string;
}

/** Options for extending TTL. */
export interface ExtendTtlOptions {
  /** Duration string like "1h", "30m", "2d". */
  by: string;
}

/** Metadata for a sandbox snapshot. */
export interface SnapshotMeta {
  name: string;
  sandbox: string;
  image_tag: string;
  backend: string;
  base_image?: string;
  vcpus?: number;
  memory_mb?: number;
  created_at: string;
}

/** Options for taking a snapshot. */
export interface TakeSnapshotOptions {
  /** Name of the sandbox to snapshot. */
  sandbox: string;
  /** Optional snapshot name (auto-generated if not provided). */
  name?: string;
}

/** A link extracted from a web page. */
export interface PageLink {
  text: string;
  href: string;
}

/** Result of navigating to a web page. */
export interface PageResult {
  /** Page title. */
  title: string;
  /** Final URL after redirects. */
  url: string;
  /** Page body text (truncated to ~8KB). */
  text: string;
  /** First 50 links on the page. */
  links: PageLink[];
}

/** ARIA accessibility tree snapshot of a web page (v2). */
export interface AriaSnapshot {
  /** ARIA tree as YAML. */
  snapshot: string;
  /** Current page URL. */
  url: string;
  /** Page title. */
  title: string;
  /** Available ref IDs (e.g. ["e1", "e2", ...]). */
  refs: string[];
}

/** A browser interaction event for debugging and context recovery. */
export interface BrowserEvent {
  /** Monotonic sequence number. */
  seq: number;
  /** Event type (e.g. "page.navigated", "page.clicked"). */
  type: string;
  /** Page name. */
  page: string;
  /** ISO 8601 timestamp. */
  ts: string;
}
