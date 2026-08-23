export interface Sandbox {
  name: string;
  uuid: string;
  status: string;
  backend: string;
  ip?: string;
  image?: string;
  description?: string;
  created_at?: string;
  labels?: Record<string, string>;
}

export interface SandboxListResponse {
  success: boolean;
  data: Sandbox[];
}
