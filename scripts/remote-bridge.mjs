#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";

const [, , provider, payload] = process.argv;

if (!provider || !payload) {
  console.error("usage: remote-bridge.mjs <provider> <base64-request>");
  process.exit(1);
}

const request = JSON.parse(Buffer.from(payload, "base64").toString("utf8"));
const mode = process.env.AGENTKERNEL_REMOTE_BRIDGE_MODE ?? "";

if (mode !== "mock") {
  respond({
    success: false,
    error:
      `Remote provider bridge for '${provider}' is not configured. ` +
      `Set AGENTKERNEL_REMOTE_BRIDGE_MODE=mock for local testing or point ` +
      `AGENTKERNEL_REMOTE_BRIDGE at a provider-aware bridge.`,
  });
}

const rootDir = path.join(os.tmpdir(), "agentkernel-remote-bridge", provider);
await fs.mkdir(rootDir, { recursive: true });

try {
  switch (request.operation) {
    case "create":
      respond(await createSandbox(provider, request));
      break;
    case "resume":
      respond(await resumeSandbox(request));
      break;
    case "status":
      respond(await statusSandbox(request));
      break;
    case "stop":
      respond(await stopSandbox(request));
      break;
    case "destroy":
      respond(await destroySandbox(request));
      break;
    case "exec":
      respond(await execSandbox(request));
      break;
    case "attach":
      process.exit(await attachSandbox(request));
      break;
    case "write_file":
      respond(await writeSandboxFile(request));
      break;
    case "read_file":
      respond(await readSandboxFile(request));
      break;
    case "remove_file":
      respond(await removeSandboxFile(request));
      break;
    case "mkdir":
      respond(await mkdirSandbox(request));
      break;
    case "sync_push":
      respond(await syncPush(request));
      break;
    case "sync_pull":
      respond(await syncPull(request));
      break;
    case "snapshot":
      respond(await takeSnapshot(request));
      break;
    case "restore":
      respond(await restoreSnapshot(request));
      break;
    default:
      respond({
        success: false,
        error: `unsupported remote bridge operation: ${request.operation}`,
      });
  }
} catch (error) {
  respond({
    success: false,
    error: error instanceof Error ? error.message : String(error),
  });
}

function respond(body) {
  process.stdout.write(`${JSON.stringify(body)}\n`);
  process.exit(body.success === false ? 1 : 0);
}

function sandboxesDir() {
  return path.join(rootDir, "sandboxes");
}

function sandboxStatePath(remoteId) {
  return path.join(sandboxesDir(), `${remoteId}.json`);
}

function nameMapPath(name) {
  return path.join(rootDir, "names", `${name}.json`);
}

function snapshotDir(remoteId, snapshotName) {
  return path.join(rootDir, "snapshots", remoteId, snapshotName);
}

async function loadSandbox(request) {
  const remoteId = request.remote_id ?? (await lookupRemoteId(request.sandbox_name));
  if (!remoteId) {
    throw new Error(`remote sandbox '${request.sandbox_name}' not found`);
  }

  const file = sandboxStatePath(remoteId);
  const raw = await fs.readFile(file, "utf8");
  return JSON.parse(raw);
}

async function saveSandbox(state) {
  await fs.mkdir(sandboxesDir(), { recursive: true });
  await fs.mkdir(path.dirname(nameMapPath(state.name)), { recursive: true });
  await fs.writeFile(sandboxStatePath(state.remoteId), JSON.stringify(state, null, 2));
  await fs.writeFile(nameMapPath(state.name), JSON.stringify({ remoteId: state.remoteId }));
}

async function lookupRemoteId(name) {
  try {
    const raw = await fs.readFile(nameMapPath(name), "utf8");
    const parsed = JSON.parse(raw);
    return parsed.remoteId;
  } catch {
    return null;
  }
}

function endpointUrl(providerName, remoteId, port) {
  return `https://${providerName}-${remoteId}-${port}.agentkernel.invalid`;
}

function toResponse(state, extra = {}) {
  return {
    success: true,
    remote_id: state.remoteId,
    remote_namespace: state.namespace ?? null,
    remote_metadata: {
      ...state.remoteMetadata,
      last_known_status: state.running ? "running" : "stopped",
    },
    workspace_revision: state.workspaceRevision ?? null,
    endpoints: state.endpoints ?? [],
    running: Boolean(state.running),
    ...extra,
  };
}

async function createSandbox(providerName, request) {
  const remoteId = crypto.randomUUID();
  const fsRoot = path.join(rootDir, "filesystems", remoteId, "rootfs");
  const workspaceDir = path.join(fsRoot, "workspace");
  await fs.mkdir(workspaceDir, { recursive: true });

  const state = {
    provider: providerName,
    remoteId,
    name: request.sandbox_name,
    namespace: request.remote_namespace ?? null,
    profile: request.profile ?? "default",
    image: request.image ?? null,
    vcpus: request.vcpus ?? 1,
    memoryMb: request.memory_mb ?? 512,
    running: true,
    fsRoot,
    workspaceDir,
    workspaceRevision: hashString(""),
    remoteMetadata: {
      ...(request.remote_metadata ?? {}),
      profile_name: request.profile ?? request.image ?? "default",
    },
    endpoints: (request.ports ?? []).map((port) => ({
      container_port: port.container_port,
      protocol: port.protocol ?? "tcp",
      url: endpointUrl(providerName, remoteId, port.container_port),
    })),
  };

  await saveSandbox(state);
  return toResponse(state);
}

async function resumeSandbox(request) {
  const state = await loadSandbox(request);
  state.running = true;
  await saveSandbox(state);
  return toResponse(state);
}

async function statusSandbox(request) {
  const state = await loadSandbox(request);
  return toResponse(state);
}

async function stopSandbox(request) {
  const state = await loadSandbox(request);
  state.running = false;
  await saveSandbox(state);
  return toResponse(state);
}

async function destroySandbox(request) {
  const state = await loadSandbox(request);
  await fs.rm(path.join(rootDir, "filesystems", state.remoteId), {
    recursive: true,
    force: true,
  });
  await fs.rm(path.dirname(snapshotDir(state.remoteId, "placeholder")), {
    recursive: true,
    force: true,
  });
  await fs.rm(sandboxStatePath(state.remoteId), { force: true });
  await fs.rm(nameMapPath(state.name), { force: true });
  return { success: true, remote_id: state.remoteId, running: false };
}

async function execSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const command = request.command ?? [];
  if (command.length === 0) {
    throw new Error("exec requires a command");
  }

  const result = await spawnCommand(command, {
    cwd: resolveWorkdir(state, request.workdir),
    env: { ...process.env, ...Object.fromEntries(Object.entries(request.env ?? {})) },
    stdio: "pipe",
  });

  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state, {
    exit_code: result.exitCode,
    stdout: result.stdout,
    stderr: result.stderr,
  });
}

async function attachSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const shell = request.shell || process.env.SHELL || "/bin/sh";
  const env = { ...process.env, ...Object.fromEntries(Object.entries(request.env ?? {})) };
  const child = spawn(shell, {
    cwd: resolveWorkdir(state, "/workspace"),
    env,
    stdio: "inherit",
    shell: false,
  });

  const exitCode = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => resolve(code ?? -1));
  });

  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return exitCode;
}

async function writeSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.writeFile(target, Buffer.from(request.content_base64 ?? "", "base64"));
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function readSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  const content = await fs.readFile(target);
  return toResponse(state, { content_base64: content.toString("base64") });
}

async function removeSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.rm(target, { force: true });
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function mkdirSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.mkdir(target, { recursive: Boolean(request.recursive) });
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function syncPush(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const localPath = request.local_path;
  if (!localPath) {
    throw new Error("sync_push requires local_path");
  }

  const remoteRevision = await hashTree(state.workspaceDir);
  if (request.workspace_revision && remoteRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: remote workspace changed since last sync");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  await mirrorDirectory(localPath, state.workspaceDir, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function syncPull(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const localPath = request.local_path;
  if (!localPath) {
    throw new Error("sync_pull requires local_path");
  }

  const localRevision = await hashTree(localPath);
  if (request.workspace_revision && localRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: local workspace changed since last sync");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  await mirrorDirectory(state.workspaceDir, localPath, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function takeSnapshot(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const snapshotName = request.snapshot_name || `snapshot-${Date.now()}`;
  const destination = snapshotDir(state.remoteId, snapshotName);
  await fs.rm(destination, { recursive: true, force: true });
  await mirrorDirectory(state.fsRoot, destination, []);
  return toResponse(state, {
    remote_metadata: {
      ...state.remoteMetadata,
      snapshot_handle: snapshotName,
      last_known_status: state.running ? "running" : "stopped",
    },
  });
}

async function restoreSnapshot(request) {
  const state = await loadSandbox(request);
  const snapshotName = request.snapshot_name;
  if (!snapshotName) {
    throw new Error("restore requires snapshot_name");
  }
  const source = snapshotDir(state.remoteId, snapshotName);
  await fs.access(source);
  await mirrorDirectory(source, state.fsRoot, []);
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

function ensureRunning(state) {
  if (!state.running) {
    throw new Error(`remote sandbox '${state.name}' is not running`);
  }
}

function resolveWorkdir(state, requested) {
  if (!requested) {
    return state.workspaceDir;
  }
  return resolveSandboxPath(state, requested);
}

function resolveSandboxPath(state, requestedPath) {
  if (!requestedPath || !requestedPath.startsWith("/")) {
    throw new Error(`sandbox path must be absolute: ${requestedPath}`);
  }
  const rel = path.normalize(requestedPath).replace(/^(\.\.(\/|\\|$))+/, "");
  const target = path.resolve(state.fsRoot, `.${rel}`);
  if (!target.startsWith(path.resolve(state.fsRoot))) {
    throw new Error(`path escapes sandbox root: ${requestedPath}`);
  }
  return target;
}

async function spawnCommand(command, options) {
  return await new Promise((resolve, reject) => {
    const [program, ...args] = command;
    const child = spawn(program, args, options);
    let stdout = "";
    let stderr = "";
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("exit", (exitCode) => resolve({ exitCode: exitCode ?? -1, stdout, stderr }));
  });
}

async function loadIgnoreRules(baseDir) {
  const builtins = [".git", ".jj", "node_modules", "target", "dist", "build", ".DS_Store"];
  const rules = [...builtins];
  for (const file of [".agentkernelignore", ".gitignore"]) {
    try {
      const raw = await fs.readFile(path.join(baseDir, file), "utf8");
      for (const line of raw.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (trimmed && !trimmed.startsWith("#")) {
          rules.push(trimmed.replace(/^\.\//, ""));
        }
      }
    } catch {
      // ignore missing files
    }
  }
  return rules;
}

function shouldIgnore(relPath, rules) {
  return rules.some((rule) => {
    if (rule.endsWith("/")) {
      const prefix = rule.slice(0, -1);
      return relPath === prefix || relPath.startsWith(`${prefix}/`);
    }
    return relPath === rule || relPath.startsWith(`${rule}/`);
  });
}

async function mirrorDirectory(sourceDir, targetDir, ignoreRules) {
  await fs.mkdir(targetDir, { recursive: true });
  const sourceEntries = await collectEntries(sourceDir, ignoreRules);
  const targetEntries = await collectEntries(targetDir, ignoreRules);

  for (const relPath of targetEntries.keys()) {
    if (!sourceEntries.has(relPath)) {
      await fs.rm(path.join(targetDir, relPath), { recursive: true, force: true });
    }
  }

  for (const [relPath, entry] of sourceEntries.entries()) {
    const sourcePath = path.join(sourceDir, relPath);
    const targetPath = path.join(targetDir, relPath);
    if (entry.kind === "dir") {
      await fs.mkdir(targetPath, { recursive: true });
      continue;
    }
    await fs.mkdir(path.dirname(targetPath), { recursive: true });
    await fs.copyFile(sourcePath, targetPath);
  }
}

async function collectEntries(baseDir, ignoreRules) {
  const entries = new Map();
  await walk(baseDir, "", entries, ignoreRules);
  return entries;
}

async function walk(baseDir, relDir, entries, ignoreRules) {
  let children = [];
  try {
    children = await fs.readdir(path.join(baseDir, relDir), { withFileTypes: true });
  } catch {
    return;
  }

  for (const child of children) {
    const relPath = relDir ? path.join(relDir, child.name) : child.name;
    const normalized = relPath.split(path.sep).join("/");
    if (shouldIgnore(normalized, ignoreRules)) {
      continue;
    }
    if (child.isDirectory()) {
      entries.set(normalized, { kind: "dir" });
      await walk(baseDir, relPath, entries, ignoreRules);
    } else if (child.isFile()) {
      entries.set(normalized, { kind: "file" });
    }
  }
}

async function hashTree(baseDir) {
  const entries = await collectEntries(baseDir, []);
  const hasher = crypto.createHash("sha256");
  for (const relPath of [...entries.keys()].sort()) {
    const entry = entries.get(relPath);
    hasher.update(relPath);
    hasher.update(entry.kind);
    if (entry.kind === "file") {
      try {
        hasher.update(await fs.readFile(path.join(baseDir, relPath)));
      } catch {
        // ignore transient reads
      }
    }
  }
  return hasher.digest("hex");
}

function hashString(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}
