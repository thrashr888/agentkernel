import { invoke } from "@tauri-apps/api/core";
import type {
  SandboxInfo,
  RunOutput,
  BatchCommand,
  BatchRunResponse,
  SnapshotMeta,
  ExtendTtlResponse,
  DetachedCommand,
  DetachedLogsResponse,
  CreateSandboxRequest,
  TemplateInfo,
  Settings,
  AuditLogEntry,
  StatusInfo,
  BackendDiscovery,
  DoctorResult,
  GcResult,
  FileReadResponse,
  SecretEntry,
  AgentInfo,
  AgentIntegrationResult,
  PolicyStatus,
  PolicyCheckResult,
  PolicyReloadResult,
  PolicyActivationRequest,
  PolicyActivationResult,
  PolicyAuditEntry,
  LlmUsageEntry,
  LlmSpendReport,
  DurableObjectInfo,
  CreateObjectRequest,
  ScheduleInfo,
  CreateScheduleRequest,
  DurableStoreInfo,
  CreateStoreRequest,
  StoreQueryResult,
  StoreExecuteResult,
  StoreCommandResult,
  TunnelStatus,
  DockerImage,
  DockerImageDiskUsage,
  BenchmarkResult,
  SessionSummary,
  SessionRecording,
  PermissionGrant,
  GrantPermissionRequest,
  PermissionCheckResult as PermissionCheckResultType,
} from "./types";

export const api = {
  // Health — returns "ok" string on success
  checkConnection: () => invoke<string>("check_connection"),

  // Diagnostics
  getStatus: () => invoke<StatusInfo>("get_status"),
  getDoctor: () => invoke<DoctorResult>("get_doctor"),
  getBackends: () => invoke<BackendDiscovery>("get_backends"),
  prepareBackend: () => invoke<string>("prepare_backend"),
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
  updateSandbox: (name: string, labels?: Record<string, string>, description?: string) =>
    invoke<SandboxInfo>("update_sandbox", { name, labels, description }),

  // Files
  listFiles: (name: string, path: string) =>
    invoke<RunOutput>("list_files", { name, path }),
  readFile: (name: string, path: string) =>
    invoke<FileReadResponse>("read_file", { name, path }),

  // Quick Run — temporary sandbox, execute, clean up
  quickRun: (command: string[], image?: string, profile?: string) =>
    invoke<RunOutput>("quick_run", { command, image, profile }),

  // Parallel execution — run the same API command in multiple ephemeral sandboxes
  batchRun: (commands: BatchCommand[]) =>
    invoke<BatchRunResponse>("batch_run", { commands }),

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
  installAgent: (name: string, scope: "project" | "global", confirm: boolean) =>
    invoke<AgentIntegrationResult>("install_agent", { name, scope, confirm }),

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
  getLlmSpend: () => invoke<LlmSpendReport>("get_llm_spend"),

  // Durable Objects
  listObjects: () => invoke<DurableObjectInfo[]>("list_objects"),
  getObject: (id: string) => invoke<DurableObjectInfo>("get_object", { id }),
  createObject: (req: CreateObjectRequest) =>
    invoke<DurableObjectInfo>("create_object", { req }),
  deleteObject: (id: string) => invoke<void>("delete_object", { id }),
  patchObject: (id: string, storage?: unknown, status?: string) =>
    invoke<DurableObjectInfo>("patch_object", { id, storage: storage ?? null, status: status ?? null }),
  callObject: (objectClass: string, objectId: string, method: string, args: unknown) =>
    invoke<unknown>("call_object", { class: objectClass, object_id: objectId, method, args }),

  // Schedules
  listSchedules: () => invoke<ScheduleInfo[]>("list_schedules"),
  getSchedule: (id: string) => invoke<ScheduleInfo>("get_schedule", { id }),
  createSchedule: (req: CreateScheduleRequest) =>
    invoke<ScheduleInfo>("create_schedule", { req }),
  deleteSchedule: (id: string) => invoke<void>("delete_schedule", { id }),
  triggerSchedule: (id: string) => invoke<ScheduleInfo>("trigger_schedule", { id }),

  // Durable Stores
  listStores: () => invoke<DurableStoreInfo[]>("list_stores"),
  getStore: (id: string) => invoke<DurableStoreInfo>("get_store", { id }),
  createStore: (req: CreateStoreRequest) =>
    invoke<DurableStoreInfo>("create_store", { req }),
  deleteStore: (id: string) => invoke<void>("delete_store", { id }),
  queryStore: (id: string, sql: string, params?: unknown[]) =>
    invoke<StoreQueryResult>("query_store", { id, sql, params: params ?? [] }),
  executeStore: (id: string, sql: string, params?: unknown[]) =>
    invoke<StoreExecuteResult>("execute_store", { id, sql, params: params ?? [] }),
  commandStore: (id: string, command: string[]) =>
    invoke<StoreCommandResult>("command_store", { id, command }),

  // Server management
  startServer: () => invoke<string>("start_server"),
  stopServer: () => invoke<string>("stop_server"),
  serverStatus: () => invoke<boolean>("server_status"),
  startTunnel: () => invoke<TunnelStatus>("start_tunnel"),
  stopTunnel: () => invoke<TunnelStatus>("stop_tunnel"),
  tunnelStatus: () => invoke<TunnelStatus>("tunnel_status"),

  // Docker Images
  listImages: () => invoke<DockerImage[]>("list_images"),
  removeImage: (id: string) => invoke<void>("remove_image", { id }),
  imageDiskUsage: () => invoke<DockerImageDiskUsage[]>("image_disk_usage"),
  pullImage: (image: string) => invoke<string>("pull_image", { image }),
  pruneImages: (agentkernelOnly = true) =>
    invoke<string>("prune_images", { agentkernel_only: agentkernelOnly }),

  // Benchmark
  runBenchmark: () => invoke<BenchmarkResult>("run_benchmark"),

  // Session Recording
  listSessions: () => invoke<SessionSummary[]>("list_sessions"),
  getSession: (id: string) =>
    invoke<SessionRecording>("get_session", { id }),
  getSessionCast: (id: string) =>
    invoke<string>("get_session_cast", { id }),

  // Export/Import Config
  exportSandboxConfig: (name: string) =>
    invoke<string>("export_sandbox_config", { name }),
  importSandboxConfig: (config: string, name?: string) =>
    invoke<SandboxInfo>("import_sandbox_config", { name, config }),

  // Policy (enterprise)
  getPolicyStatus: () => invoke<PolicyStatus>("get_policy_status"),
  checkPolicy: (action: string, sandbox: string) =>
    invoke<PolicyCheckResult>("check_policy", { action, sandbox }),
  reloadPolicy: () => invoke<PolicyReloadResult>("reload_policy"),
  activateLocalPolicy: (request: PolicyActivationRequest) =>
    invoke<PolicyActivationResult>("activate_local_policy", { request }),
  getPolicyAudit: (last?: number) =>
    invoke<PolicyAuditEntry[]>("get_policy_audit", { last }),

  // Interactive Permissions
  listPermissions: () => invoke<PermissionGrant[]>("list_permissions"),
  grantPermission: (req: GrantPermissionRequest) =>
    invoke<{ grant_id: string }>("grant_permission", { req }),
  revokePermission: (id: string) =>
    invoke<void>("revoke_permission", { id }),
  checkPermission: (kind: string, sandbox?: string) =>
    invoke<PermissionCheckResultType>("check_permission", { kind, sandbox }),
};
