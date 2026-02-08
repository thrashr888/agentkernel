// Sandbox types matching Rust backend / agentkernel HTTP API
export interface SandboxInfo {
  name: string;
  status: string;
  backend: string;
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  created_at?: string;
  ip?: string;
  ports?: string[];
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
  profile?: string;
  source_url?: string;
  source_ref?: string;
  volumes?: string[];
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
}

// Secrets
export interface SecretEntry {
  name: string;
  created_at?: string;
}

// Settings
export interface Settings {
  api_url: string;
  api_key: string;
  theme: "light" | "dark" | "system";
  poll_interval_ms: number;
}
