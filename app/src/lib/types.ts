// Sandbox types matching Rust backend / agentkernel HTTP API
export interface SandboxInfo {
  name: string;
  uuid?: string;
  status: string;
  backend: string;
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  created_at?: string;
  created_from_template?: string;
  template_help_text?: string;
  ip?: string;
  ports?: string[];
  secret_files?: string[];
  secret_mappings?: Record<string, string>;
  labels?: Record<string, string>;
  description?: string;
}

export interface RunOutput {
  output: string;
}

/** A command submitted to the parallel batch-run endpoint. */
export interface BatchCommand {
  command: string[];
}

/** Result for one command submitted to the parallel batch-run endpoint. */
export interface BatchResult {
  output?: string;
  error?: string;
}

export interface BatchRunResponse {
  results: BatchResult[];
}

export interface FileReadResponse {
  content: string;
  encoding: string;
  size: number;
}

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

export interface ExtendTtlResponse {
  expires_at?: string;
}

// Detached command types
export interface DetachedCommand {
  id: string;
  sandbox: string;
  command: string[];
  pid: number;
  status: string;
  exit_code?: number;
  started_at: string;
}

export interface DetachedLogsResponse {
  stdout?: string;
  stderr?: string;
}

// Audit log entry from GET /sandboxes/:name/logs
export interface AuditLogEntry {
  timestamp: string;
  pid?: number;
  user?: string;
  type?: string;
  [key: string]: unknown;
}

// Diagnostics types
export interface StatusInfo {
  version: string;
  backend: string;
  api_key_configured: boolean;
}

export interface BackendCapabilities {
  mount_cwd: boolean;
  mount_home: boolean;
  attach: boolean;
  host_volumes: boolean;
  ssh: boolean;
  proxy_secret_bindings: boolean;
  secret_files: boolean;
  snapshots: boolean;
  resume: boolean;
  endpoints: boolean;
}

export interface BackendDescriptor {
  backend: string;
  configured: boolean;
  usable: boolean;
  readiness_reason: string;
  capabilities: BackendCapabilities;
}

export interface BackendDiscovery {
  default_backend?: string;
  backends: BackendDescriptor[];
}

export interface HealthCheck {
  name: string;
  status: string;
  message: string;
}

export interface DoctorResult {
  checks: HealthCheck[];
  healthy: boolean;
}

export interface GcResult {
  removed: string[];
  count: number;
}

// Request types
export interface CreateSandboxRequest {
  name: string;
  /** Backend identifier; omit or use automatic for server selection. */
  backend?: string;
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  ports?: string[];
  secret_files?: string[];
  profile?: string;
  source_url?: string;
  source_ref?: string;
  volumes?: string[];
  agent?: string;
  secrets?: string[];
  init_script?: string;
  secret_mappings?: Record<string, string>;
  created_from_template?: string;
  template_help_text?: string;
  labels?: Record<string, string>;
  description?: string;
}

export interface ExecRequest {
  command: string[];
  env?: string[];
  workdir?: string;
  sudo?: boolean;
}

export interface TakeSnapshotRequest {
  sandbox: string;
  name?: string;
}

// Template types
export interface TemplateInfo {
  name: string;
  description: string;
  package?: string;
  category: string;
  base_image: string;
  vcpus: number;
  memory_mb: number;
  ports?: string[];
  secret_files?: string[];
  init_script?: string;
  help_text?: string;
  secrets?: Record<string, string>;
}

// Secrets
export interface SecretEntry {
  name: string;
  created_at?: string;
}

// Agents/Plugins
export interface AgentInfo {
  name: string;
  display_name: string;
  enabled: boolean;
  description: string;
  cli_installed: boolean;
  cli_version?: string;
  tested_version: string;
  compatibility_status: "not_installed" | "tested" | "untested_version";
  install_command: string;
  integration_supported: boolean;
  integration_project_installed: boolean;
  integration_global_installed: boolean;
  integration_global_supported: boolean;
}

export interface AgentIntegrationResult {
  target: string;
  scope: "project" | "global";
  confirmed: boolean;
  files: string[];
}

// LLM Usage
export interface LlmUsageEntry {
  provider: string;
  model: string;
  request_count: number;
  streaming_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tokens: number;
  last_request: string;
}

export interface LlmSpendMetric {
  bucket: string;
  tenant: string;
  agent: string;
  user: string;
  project: string;
  provider: string;
  model: string;
  request_count: number;
  streaming_count: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  last_request: string;
}

export interface MonetaryCostStatus {
  available: boolean;
  currency?: string;
  reason: string;
}

export interface LlmSpendReport {
  metrics: LlmSpendMetric[];
  next_offset?: number;
  retention_days: number;
  monetary_cost: MonetaryCostStatus;
}

// Durable Objects
export interface DurableObjectInfo {
  id: string;
  class: string;
  object_id: string;
  status: string;
  sandbox?: string;
  storage: unknown;
  idle_timeout_seconds: number;
  created_at: string;
  updated_at: string;
}

export interface CreateObjectRequest {
  class: string;
  object_id: string;
  sandbox?: string;
  storage?: unknown;
  idle_timeout_seconds?: number;
}

// Schedules
export interface ScheduleInfo {
  id: string;
  name: string;
  cron?: string;
  fire_at?: string;
  method: string;
  args: unknown;
  target_class?: string;
  target_object_id?: string;
  target_orchestration?: string;
  status: string;
  last_fired_at?: string;
  created_at: string;
  updated_at: string;
}

export interface CreateScheduleRequest {
  name: string;
  cron?: string;
  fire_at?: string;
  method: string;
  args?: unknown;
  target_class?: string;
  target_object_id?: string;
  target_orchestration?: string;
}

// Durable Stores
export interface DurableStoreInfo {
  id: string;
  name: string;
  kind: string;
  sandbox?: string;
  config: unknown;
  created_at: string;
  updated_at: string;
}

export interface CreateStoreRequest {
  name: string;
  kind: string;
  sandbox?: string;
  config?: unknown;
}

export interface StoreQueryResult {
  columns: string[];
  rows: unknown[];
  row_count: number;
}

export interface StoreExecuteResult {
  rows_affected: number;
  last_insert_rowid?: number;
}

export interface StoreCommandResult {
  result: unknown;
}

// Server entry for multi-server support
export interface ServerEntry {
  name: string;
  url: string;
  api_key?: string;
  /** True when the desktop app owns and starts this local server. */
  managed?: boolean;
}

// Settings
export interface Settings {
  active_server?: string;
  servers: ServerEntry[];
  theme: "light" | "dark" | "system";
  poll_interval_ms: number;
  // Legacy fields (kept for migration)
  api_url: string;
  api_key: string;
}

// Docker Images
export interface DockerImage {
  id: string;
  repository: string;
  tag: string;
  size: string;
  created: string;
}

export interface DockerImageDiskUsage {
  type: string;
  total: string;
  active: string;
  size: string;
  reclaimable: string;
}

// Benchmark
export interface BenchmarkResult {
  create_ms: number;
  exec_ms: number;
  destroy_ms: number;
  total_ms: number;
  image: string;
  backend: string;
  started_at?: string;
  finished_at?: string;
  timestamp: string;
}

// Session Recording
export interface SessionSummary {
  id: string;
  filename: string;
  title?: string;
  command?: string;
  timestamp?: number;
  duration?: number;
  width: number;
  height: number;
  event_count: number;
  size_bytes: number;
}

export interface SessionEvent {
  time: number;
  event_type: "output" | "input";
  data: string;
}

export interface SessionRecording {
  id: string;
  filename: string;
  title?: string;
  command?: string;
  timestamp?: number;
  duration?: number;
  width: number;
  height: number;
  event_count: number;
  size_bytes: number;
  header: {
    version: number;
    width: number;
    height: number;
    duration?: number;
    title?: string;
    command?: string;
  };
  events: SessionEvent[];
}

// Policy (enterprise)
export interface PolicyStatus {
  enabled: boolean;
  version: number;
  org_id?: string;
  offline_mode?: string;
  policy_server?: string;
}

export interface PolicyCheckResult {
  decision: string;
  reason: string;
  matched_policies: string[];
  evaluation_time_us: number;
}

export interface PolicyReloadResult {
  reloaded: boolean;
  version: number;
}

export interface PolicyAuditEntry {
  timestamp: string;
  principal: string;
  action: string;
  resource: string;
  decision: string;
  matched_policies: string[];
  evaluation_time_us: number;
  reason?: string;
}

// Interactive Permissions
export type PermissionKind =
  | "sandbox_remove"
  | "sandbox_create"
  | "network_access"
  | "mount_directory"
  | "sudo_exec"
  | "file_delete";

export type GrantScope = "once" | "session" | "always";

export interface PermissionGrant {
  id: string;
  kind: PermissionKind;
  scope: GrantScope;
  sandbox?: string;
  granted_at: string;
  granted_by: string;
}

export interface GrantPermissionRequest {
  kind: PermissionKind;
  scope?: GrantScope;
  sandbox?: string;
}

export interface PermissionCheckResult {
  permitted: boolean;
  kind: string;
}
