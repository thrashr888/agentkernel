import { invoke } from "@tauri-apps/api/core";
import type {
  SandboxInfo,
  RunOutput,
  SnapshotMeta,
  ExtendTtlResponse,
  DetachedCommand,
  DetachedLogsResponse,
  CreateSandboxRequest,
  ExecRequest,
  TakeSnapshotRequest,
  TemplateInfo,
  Settings,
} from "./types";

export const api = {
  // Health
  checkConnection: () => invoke<boolean>("check_connection"),

  // Sandboxes
  listSandboxes: () => invoke<SandboxInfo[]>("list_sandboxes"),
  getSandbox: (name: string) => invoke<SandboxInfo>("get_sandbox", { name }),
  createSandbox: (request: CreateSandboxRequest) =>
    invoke<SandboxInfo>("create_sandbox", { request }),
  removeSandbox: (name: string) => invoke<void>("remove_sandbox", { name }),
  extendTtl: (name: string, seconds: number) =>
    invoke<ExtendTtlResponse>("extend_ttl", { name, seconds }),

  // Execution
  execCommand: (request: ExecRequest) =>
    invoke<RunOutput>("exec_command", { request }),
  execDetached: (name: string, command: string[]) =>
    invoke<DetachedCommand>("exec_detached", { name, command }),
  listDetached: (name: string) =>
    invoke<DetachedCommand[]>("list_detached", { name }),
  getDetachedLogs: (name: string, id: string) =>
    invoke<DetachedLogsResponse>("get_detached_logs", { name, id }),
  killDetached: (name: string, id: string) =>
    invoke<void>("kill_detached", { name, id }),

  // Snapshots
  listSnapshots: () => invoke<SnapshotMeta[]>("list_snapshots"),
  takeSnapshot: (request: TakeSnapshotRequest) =>
    invoke<SnapshotMeta>("take_snapshot", { request }),
  deleteSnapshot: (name: string) => invoke<void>("delete_snapshot", { name }),
  restoreSnapshot: (name: string) =>
    invoke<SandboxInfo>("restore_snapshot", { name }),

  // Templates
  listTemplates: () => invoke<TemplateInfo[]>("list_templates"),

  // Settings
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<void>("save_settings", { settings }),
};
