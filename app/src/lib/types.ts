// Sandbox types matching Rust backend
export interface SandboxInfo {
  name: string;
  status: string;
  image: string;
  created_at: string;
  vcpus: number;
  memory_mb: number;
  ttl_seconds: number;
  expires_at: string | null;
  pid: number | null;
}

export interface RunOutput {
  exit_code: number;
  stdout: string;
  stderr: string;
}

export interface SnapshotMeta {
  name: string;
  sandbox_name: string;
  created_at: string;
  size_bytes: number;
}

export interface ExtendTtlResponse {
  name: string;
  new_expires_at: string;
  ttl_seconds: number;
}

// Detached command types
export interface DetachedCommand {
  id: string;
  sandbox_name: string;
  command: string[];
  started_at: string;
  status: string;
  exit_code: number | null;
}

export interface DetachedLogsResponse {
  id: string;
  stdout: string;
  stderr: string;
  status: string;
  exit_code: number | null;
}

// Request types
export interface CreateSandboxRequest {
  name: string;
  image?: string;
  vcpus?: number;
  memory_mb?: number;
  ttl_seconds?: number;
  env?: Record<string, string>;
  network?: boolean;
}

export interface ExecRequest {
  name: string;
  command: string[];
  env?: Record<string, string>;
  workdir?: string;
}

export interface TakeSnapshotRequest {
  sandbox_name: string;
  snapshot_name: string;
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

// Settings
export interface Settings {
  api_url: string;
  api_key: string;
  theme: "light" | "dark" | "system";
  poll_interval_ms: number;
}
