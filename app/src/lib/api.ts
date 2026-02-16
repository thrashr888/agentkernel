import { invoke } from "@tauri-apps/api/core";
import type {
  SandboxInfo,
  RunOutput,
  SnapshotMeta,
  ExtendTtlResponse,
  DetachedCommand,
  DetachedLogsResponse,
  CreateSandboxRequest,
  TemplateInfo,
  Settings,
  AuditLogEntry,
  StatusInfo,
  DoctorResult,
  GcResult,
  FileReadResponse,
  SecretEntry,
  AgentInfo,
  PolicyStatus,
  PolicyCheckResult,
  PolicyReloadResult,
  PolicyAuditEntry,
  LlmUsageEntry,
} from "./types";

export const api = {
  // Health — returns "ok" string on success
  checkConnection: () => invoke<string>("check_connection"),

  // Diagnostics
  getStatus: () => invoke<StatusInfo>("get_status"),
  getDoctor: () => invoke<DoctorResult>("get_doctor"),
  runGc: () => invoke<GcResult>("run_gc"),

  // Sandboxes
  listSandboxes: () => invoke<SandboxInfo[]>("list_sandboxes"),
  getSandbox: (name: string) => invoke<SandboxInfo>("get_sandbox", { name }),
  createSandbox: (req: CreateSandboxRequest) =>
    invoke<SandboxInfo>("create_sandbox", { req }),
  removeSandbox: (name: string) => invoke<void>("remove_sandbox", { name }),
  startSandbox: (name: string) => invoke<void>("start_sandbox", { name }),
  stopSandbox: (name: string) => invoke<void>("stop_sandbox", { name }),
  extendTtl: (name: string, by: string) =>
    invoke<ExtendTtlResponse>("extend_ttl", { name, by }),
  getSandboxLogs: (name: string) =>
    invoke<AuditLogEntry[]>("get_sandbox_logs", { name }),
  openTerminal: (name: string) => invoke<void>("open_terminal", { name }),
  quickstartAgent: (agent: string, name: string) =>
    invoke<string>("quickstart_agent", { agent, name }),
  exportSandbox: (name: string) =>
    invoke<string>("export_sandbox", { name }),
  resizeSandbox: (name: string, vcpus?: number, memoryMb?: number) =>
    invoke<SandboxInfo>("resize_sandbox", { name, vcpus, memory_mb: memoryMb }),

  // Files
  listFiles: (name: string, path: string) =>
    invoke<RunOutput>("list_files", { name, path }),
  readFile: (name: string, path: string) =>
    invoke<FileReadResponse>("read_file", { name, path }),

  // Quick Run — temporary sandbox, execute, clean up
  quickRun: (command: string[], image?: string, profile?: string) =>
    invoke<RunOutput>("quick_run", { command, image, profile }),

  // Execution — params are flat, not a nested struct
  execCommand: (name: string, command: string[], env?: string[], workdir?: string) =>
    invoke<RunOutput>("exec_command", { name, command, env: env ?? [], workdir }),
  execDetached: (name: string, command: string[]) =>
    invoke<DetachedCommand>("exec_detached", { name, command }),
  listDetached: (name: string) =>
    invoke<DetachedCommand[]>("list_detached", { name }),
  getDetachedLogs: (name: string, cmdId: string) =>
    invoke<DetachedLogsResponse>("get_detached_logs", { name, cmd_id: cmdId }),
  killDetached: (name: string, cmdId: string) =>
    invoke<void>("kill_detached", { name, cmd_id: cmdId }),

  // Snapshots
  listSnapshots: () => invoke<SnapshotMeta[]>("list_snapshots"),
  takeSnapshot: (sandbox: string, name?: string) =>
    invoke<SnapshotMeta>("take_snapshot", { sandbox, name }),
  deleteSnapshot: (name: string) => invoke<void>("delete_snapshot", { name }),
  restoreSnapshot: (name: string, asName?: string) =>
    invoke<SandboxInfo>("restore_snapshot", { name, as_name: asName }),

  // Templates
  listTemplates: () => invoke<TemplateInfo[]>("list_templates"),

  // Secrets
  listSecrets: () => invoke<SecretEntry[]>("list_secrets"),
  createSecret: (name: string, value: string) =>
    invoke<void>("create_secret", { name, value }),
  deleteSecret: (name: string) => invoke<void>("delete_secret", { name }),

  // Agents/Plugins
  listAgents: () => invoke<AgentInfo[]>("list_agents"),
  installAgent: (name: string) => invoke<string>("install_agent", { name }),

  // Settings
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<void>("save_settings", { new_settings: settings }),

  // Audit
  getAuditLog: (last?: number) =>
    invoke<AuditLogEntry[]>("get_audit_log", { last }),

  // LLM Usage
  getLlmUsage: () =>
    invoke<Record<string, LlmUsageEntry[]>>("get_llm_usage"),
  getLlmUsageBySandbox: (sandbox: string) =>
    invoke<LlmUsageEntry[]>("get_llm_usage_by_sandbox", { sandbox }),

  // Policy (enterprise)
  getPolicyStatus: () => invoke<PolicyStatus>("get_policy_status"),
  checkPolicy: (action: string, sandbox: string) =>
    invoke<PolicyCheckResult>("check_policy", { action, sandbox }),
  reloadPolicy: () => invoke<PolicyReloadResult>("reload_policy"),
  getPolicyAudit: (last?: number) =>
    invoke<PolicyAuditEntry[]>("get_policy_audit", { last }),
};
