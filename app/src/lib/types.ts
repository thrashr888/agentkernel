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
}

export interface RunOutput {
  output: string;
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

// Settings
export interface Settings {
  api_url: string;
  api_key: string;
  theme: "light" | "dark" | "system";
  poll_interval_ms: number;
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
