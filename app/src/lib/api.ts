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
} from "./types";

export const api = {
  // Health — returns "ok" string on success
  checkConnection: () => invoke<string>("check_connection"),

  // Diagnostics
  getStatus: () => invoke<StatusInfo>("get_status"),
  getDoctor: () => invoke<DoctorResult>("get_doctor"),

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
  restoreSnapshot: (name: string) =>
    invoke<SandboxInfo>("restore_snapshot", { name }),

  // Templates
  listTemplates: () => invoke<TemplateInfo[]>("list_templates"),

  // Settings
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<void>("save_settings", { new_settings: settings }),
};
