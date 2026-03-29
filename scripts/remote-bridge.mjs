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
let daytonaSdkPromise;

const rootDir = path.join(os.tmpdir(), "agentkernel-remote-bridge", provider);
await fs.mkdir(rootDir, { recursive: true });

try {
  if (mode === "mock") {
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
      case "delete_snapshot":
        respond(await deleteSnapshot(request));
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
  } else {
    await handleProviderRequest(provider, request);
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

async function handleProviderRequest(providerName, providerRequest) {
  if (providerName === "daytona") {
    switch (providerRequest.operation) {
      case "create":
        respond(await createDaytonaSandbox(providerRequest));
        return;
      case "resume":
        respond(await resumeDaytonaSandbox(providerRequest));
        return;
      case "status":
        respond(await statusDaytonaSandbox(providerRequest));
        return;
      case "stop":
        respond(await stopDaytonaSandbox(providerRequest));
        return;
      case "destroy":
        respond(await destroyDaytonaSandbox(providerRequest));
        return;
      case "exec":
        respond(await execDaytonaSandbox(providerRequest));
        return;
      case "attach":
        process.exit(await attachDaytonaSandbox(providerRequest));
        return;
      case "write_file":
        respond(await writeDaytonaFile(providerRequest));
        return;
      case "read_file":
        respond(await readDaytonaFile(providerRequest));
        return;
      case "remove_file":
        respond(await removeDaytonaFile(providerRequest));
        return;
      case "mkdir":
        respond(await mkdirDaytona(providerRequest));
        return;
      case "sync_push":
        respond(await syncPushDaytona(providerRequest));
        return;
      case "sync_pull":
        respond(await syncPullDaytona(providerRequest));
        return;
      case "snapshot":
        respond(await takeDaytonaSnapshot(providerRequest));
        return;
      case "delete_snapshot":
        respond(await deleteDaytonaSnapshot(providerRequest));
        return;
      case "restore":
        respond(await restoreDaytonaSnapshot(providerRequest));
        return;
      default:
        throw new Error(
          `unsupported Daytona bridge operation: ${providerRequest.operation}`,
        );
    }
  }

  respond({
    success: false,
    error:
      `Remote provider bridge for '${providerName}' is not configured. ` +
      `Set AGENTKERNEL_REMOTE_BRIDGE_MODE=mock for local testing or install a provider-aware bridge.`,
  });
}

async function loadDaytonaSdk() {
  if (!daytonaSdkPromise) {
    daytonaSdkPromise = import("@daytonaio/sdk").catch((error) => {
      throw new Error(
        "Daytona support requires '@daytonaio/sdk'. Run 'npm install --prefix scripts' first. " +
          `(${error.message})`,
      );
    });
  }
  return daytonaSdkPromise;
}

async function withDaytonaClient(fn) {
  const { Daytona } = await loadDaytonaSdk();
  const client = new Daytona({
    apiKey: process.env.DAYTONA_API_KEY,
    apiUrl: process.env.DAYTONA_API_URL,
    target: process.env.DAYTONA_TARGET || "us",
    organizationId:
      process.env.DAYTONA_ORGANIZATION_ID || process.env.DAYTONA_ORG_ID,
  });

  try {
    return await fn(client);
  } finally {
    if (client[Symbol.asyncDispose]) {
      await client[Symbol.asyncDispose]();
    }
  }
}

function daytonaEnvMap(values = {}) {
  return Object.fromEntries(
    Object.entries(values).map(([key, value]) => [key, String(value)]),
  );
}

function daytonaShellQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

function daytonaCommand(argv = []) {
  return argv.map(daytonaShellQuote).join(" ");
}

function daytonaMemoryGiB(memoryMb) {
  if (!memoryMb || Number.isNaN(Number(memoryMb))) {
    return undefined;
  }
  return Math.max(1, Math.ceil(Number(memoryMb) / 1024));
}

function daytonaPortSpec(port) {
  return `${port.container_port}/${port.protocol ?? "tcp"}`;
}

function daytonaRunning(state) {
  return state === "started" || state === "running";
}

function daytonaWorkspacePath(providerRequest) {
  return providerRequest.path || "/workspace";
}

function daytonaPosixPath(basePath, relPath = "") {
  const normalizedBase = path.posix.normalize(basePath || "/workspace");
  return relPath ? path.posix.join(normalizedBase, relPath) : normalizedBase;
}

async function ensureDaytonaFolder(sandbox, folderPath) {
  try {
    await sandbox.fs.createFolder(folderPath, "755");
  } catch {
    // Treat existing folders as success.
  }
}

function deleteSort(relPaths) {
  return [...relPaths].sort((left, right) => {
    const leftDepth = left.split("/").length;
    const rightDepth = right.split("/").length;
    if (leftDepth !== rightDepth) {
      return rightDepth - leftDepth;
    }
    return right.length - left.length;
  });
}

async function collectDaytonaEntries(sandbox, basePath, ignoreRules) {
  const entries = new Map();
  await walkDaytona(sandbox, daytonaPosixPath(basePath), "", entries, ignoreRules);
  return entries;
}

async function walkDaytona(sandbox, basePath, relDir, entries, ignoreRules) {
  const currentPath = relDir ? daytonaPosixPath(basePath, relDir) : basePath;
  let children = [];
  try {
    children = await sandbox.fs.listFiles(currentPath);
  } catch {
    return;
  }

  for (const child of children) {
    const relPath = relDir ? path.posix.join(relDir, child.name) : child.name;
    if (shouldIgnore(relPath, ignoreRules)) {
      continue;
    }
    if (child.isDir) {
      entries.set(relPath, { kind: "dir" });
      await walkDaytona(sandbox, basePath, relPath, entries, ignoreRules);
    } else {
      entries.set(relPath, { kind: "file" });
    }
  }
}

async function hashDaytonaTree(sandbox, basePath, ignoreRules = []) {
  const entries = await collectDaytonaEntries(sandbox, basePath, ignoreRules);
  const hasher = crypto.createHash("sha256");
  for (const relPath of [...entries.keys()].sort()) {
    const entry = entries.get(relPath);
    hasher.update(relPath);
    hasher.update(entry.kind);
    if (entry.kind === "file") {
      const content = await sandbox.fs.downloadFile(daytonaPosixPath(basePath, relPath));
      hasher.update(content);
    }
  }
  return hasher.digest("hex");
}

async function mirrorLocalToDaytona(sandbox, sourceDir, remoteBasePath, ignoreRules) {
  const sourceEntries = await collectEntries(sourceDir, ignoreRules);
  const remoteEntries = await collectDaytonaEntries(sandbox, remoteBasePath, ignoreRules);

  for (const relPath of deleteSort(remoteEntries.keys())) {
    if (!sourceEntries.has(relPath)) {
      const entry = remoteEntries.get(relPath);
      await sandbox.fs.deleteFile(
        daytonaPosixPath(remoteBasePath, relPath),
        entry?.kind === "dir",
      );
    }
  }

  await ensureDaytonaFolder(sandbox, daytonaPosixPath(remoteBasePath));

  const directories = [...sourceEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await ensureDaytonaFolder(sandbox, daytonaPosixPath(remoteBasePath, relPath));
  }

  const fileUploads = [...sourceEntries.entries()]
    .filter(([, entry]) => entry.kind === "file")
    .map(([relPath]) => ({
      source: path.join(sourceDir, relPath),
      destination: daytonaPosixPath(remoteBasePath, relPath),
    }));
  for (let index = 0; index < fileUploads.length; index += 32) {
    await sandbox.fs.uploadFiles(fileUploads.slice(index, index + 32));
  }
}

async function mirrorDaytonaToLocal(sandbox, remoteBasePath, targetDir, ignoreRules) {
  await fs.mkdir(targetDir, { recursive: true });
  const remoteEntries = await collectDaytonaEntries(sandbox, remoteBasePath, ignoreRules);
  const localEntries = await collectEntries(targetDir, ignoreRules);

  for (const relPath of deleteSort(localEntries.keys())) {
    if (!remoteEntries.has(relPath)) {
      await fs.rm(path.join(targetDir, relPath), { recursive: true, force: true });
    }
  }

  const directories = [...remoteEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await fs.mkdir(path.join(targetDir, relPath), { recursive: true });
  }

  const downloads = [...remoteEntries.entries()]
    .filter(([, entry]) => entry.kind === "file")
    .map(([relPath]) => ({
      source: daytonaPosixPath(remoteBasePath, relPath),
      destination: path.join(targetDir, relPath),
    }));

  for (let index = 0; index < downloads.length; index += 32) {
    const batch = downloads.slice(index, index + 32);
    const results = await sandbox.fs.downloadFiles(batch);
    const error = results.find((result) => result.error);
    if (error) {
      throw new Error(`failed to download ${error.source}: ${error.error}`);
    }
  }
}

async function getDaytonaSandbox(daytona, providerRequest) {
  const remoteId =
    providerRequest.remote_id || providerRequest.remote_metadata?.daytona_id;
  if (!remoteId) {
    throw new Error("missing remote_id for Daytona sandbox");
  }
  return daytona.get(remoteId);
}

async function daytonaEndpoints(sandbox, providerRequest) {
  const portSpecs =
    providerRequest.remote_metadata?.published_ports?.split(",").filter(Boolean) ??
    (providerRequest.ports || []).map(daytonaPortSpec);

  const endpoints = [];
  for (const spec of portSpecs) {
    const [portString, protocol = "tcp"] = spec.split("/");
    const port = Number.parseInt(portString, 10);
    if (!Number.isFinite(port)) {
      continue;
    }

    try {
      const preview = await sandbox.getPreviewLink(port);
      endpoints.push({
        container_port: port,
        protocol,
        url: preview.url,
      });
    } catch {
      // Leave the endpoint absent if Daytona cannot open a preview yet.
    }
  }
  return endpoints;
}

async function daytonaResponse(sandbox, providerRequest, extra = {}) {
  const endpoints =
    extra.endpoints ?? (await daytonaEndpoints(sandbox, providerRequest));

  return {
    success: true,
    remote_id: sandbox.id,
    remote_metadata: {
      ...(providerRequest.remote_metadata || {}),
      daytona_id: sandbox.id,
      sandbox_state: sandbox.state || "",
      profile_name:
        providerRequest.profile || providerRequest.image || sandbox.image || "default",
      published_ports:
        providerRequest.remote_metadata?.published_ports ||
        (providerRequest.ports || []).map(daytonaPortSpec).join(","),
      last_known_status: daytonaRunning(sandbox.state) ? "running" : "stopped",
    },
    endpoints,
    running: daytonaRunning(sandbox.state),
    ...extra,
  };
}

async function createDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await daytona.create({
      name: providerRequest.sandbox_name,
      image: providerRequest.image || "alpine:3.20",
      envVars: daytonaEnvMap(providerRequest.env),
      resources: {
        cpu: providerRequest.vcpus || 1,
        memory: daytonaMemoryGiB(providerRequest.memory_mb),
      },
      autoStopInterval: 15,
      public: Boolean(providerRequest.ports?.length),
    });
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function resumeDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    if (!daytonaRunning(sandbox.state)) {
      await sandbox.start();
    }
    await sandbox.refreshDataSafe();
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function statusDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.refreshDataSafe();
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function stopDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    if (daytonaRunning(sandbox.state)) {
      await sandbox.stop();
      await sandbox.refreshDataSafe();
    }
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function destroyDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.delete();
    return {
      success: true,
      remote_id: sandbox.id,
      remote_metadata: {
        ...(providerRequest.remote_metadata || {}),
        daytona_id: sandbox.id,
        last_known_status: "stopped",
      },
      running: false,
      endpoints: [],
    };
  });
}

async function execDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const result = await sandbox.process.executeCommand(
      daytonaCommand(providerRequest.command || []),
      providerRequest.workdir,
      daytonaEnvMap(providerRequest.env),
    );

    return daytonaResponse(sandbox, providerRequest, {
      exit_code: result.exitCode ?? 0,
      stdout:
        result.result || result.artifacts?.stdout || result.stdout || "",
      stderr: result.stderr || result.artifacts?.stderr || "",
    });
  });
}

async function writeDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.fs.uploadFile(
      Buffer.from(providerRequest.content_base64 || "", "base64"),
      providerRequest.path,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function readDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const content = await sandbox.fs.downloadFile(providerRequest.path);
    return daytonaResponse(sandbox, providerRequest, {
      content_base64: Buffer.from(content).toString("base64"),
    });
  });
}

async function removeDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.process.executeCommand(
      `rm -rf -- ${daytonaShellQuote(providerRequest.path)}`,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function mkdirDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const flag = providerRequest.recursive ? "-p " : "";
    await sandbox.process.executeCommand(
      `mkdir ${flag}-- ${daytonaShellQuote(providerRequest.path)}`,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function syncPushDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_push requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);
    const workspacePath = daytonaWorkspacePath(providerRequest);
    const remoteRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    if (providerRequest.workspace_revision && remoteRevision !== providerRequest.workspace_revision) {
      throw new Error("workspace sync conflict: remote workspace changed since last sync");
    }

    await mirrorLocalToDaytona(sandbox, localPath, workspacePath, ignoreRules);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function syncPullDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_pull requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);
    const localRevision = await hashTree(localPath, ignoreRules);
    if (providerRequest.workspace_revision && localRevision !== providerRequest.workspace_revision) {
      throw new Error("workspace sync conflict: local workspace changed since last sync");
    }

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorDaytonaToLocal(sandbox, workspacePath, localPath, ignoreRules);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function takeDaytonaSnapshot(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const snapshotName = providerRequest.snapshot_name || `snapshot-${Date.now()}`;
    const destination = snapshotDir(sandbox.id, snapshotName);
    await fs.rm(destination, { recursive: true, force: true });
    await fs.mkdir(destination, { recursive: true });

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorDaytonaToLocal(sandbox, workspacePath, destination, []);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, []);
    const response = await daytonaResponse(sandbox, providerRequest, {
      workspace_revision: workspaceRevision,
    });
    response.remote_metadata = {
      ...response.remote_metadata,
      snapshot_handle: encodeSnapshotHandle(sandbox.id, snapshotName),
    };
    return response;
  });
}

async function restoreDaytonaSnapshot(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const snapshotHandle = providerRequest.snapshot_name;
    if (!snapshotHandle) {
      throw new Error("restore requires snapshot_name");
    }

    const { remoteId, snapshotName } = decodeSnapshotHandle(
      snapshotHandle,
      sandbox.id,
    );
    const source = snapshotDir(remoteId, snapshotName);
    await fs.access(source);

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorLocalToDaytona(sandbox, source, workspacePath, []);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, []);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function deleteDaytonaSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(snapshotHandle);
  await fs.rm(snapshotDir(remoteId, snapshotName), {
    recursive: true,
    force: true,
  });
  return { success: true };
}

async function attachDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const pty = await sandbox.process.createPty({
      id: `agentkernel-${Date.now()}`,
      cwd: providerRequest.workdir,
      envs: daytonaEnvMap(providerRequest.env),
      cols: process.stdout.columns || 120,
      rows: process.stdout.rows || 30,
      onData: (data) => {
        process.stdout.write(Buffer.from(data));
      },
    });

    await pty.waitForConnection();

    const resize = async () => {
      if (process.stdout.isTTY) {
        await pty.resize(process.stdout.columns || 120, process.stdout.rows || 30);
      }
    };

    const onData = (chunk) => {
      void pty.sendInput(chunk);
    };

    if (process.stdin.isTTY) {
      process.stdin.setRawMode(true);
    }
    process.stdin.resume();
    process.stdin.on("data", onData);
    process.stdout.on("resize", resize);

    try {
      const result = await pty.wait();
      return result.exitCode ?? 0;
    } finally {
      process.stdin.off("data", onData);
      process.stdout.off("resize", resize);
      if (process.stdin.isTTY) {
        process.stdin.setRawMode(false);
      }
      await pty.disconnect().catch(() => {});
    }
  });
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

function encodeSnapshotHandle(remoteId, snapshotName) {
  return `${remoteId}:${snapshotName}`;
}

function decodeSnapshotHandle(handle, fallbackRemoteId) {
  const separator = handle.indexOf(":");
  if (separator === -1) {
    if (!fallbackRemoteId) {
      throw new Error("snapshot handle is missing its remote identifier");
    }
    return { remoteId: fallbackRemoteId, snapshotName: handle };
  }

  return {
    remoteId: handle.slice(0, separator),
    snapshotName: handle.slice(separator + 1),
  };
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

  const ignoreRules = await loadIgnoreRules(localPath);
  const remoteRevision = await hashTree(state.workspaceDir, ignoreRules);
  if (request.workspace_revision && remoteRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: remote workspace changed since last sync");
  }

  await mirrorDirectory(localPath, state.workspaceDir, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir, ignoreRules);
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

  const ignoreRules = await loadIgnoreRules(localPath);
  const localRevision = await hashTree(localPath, ignoreRules);
  if (request.workspace_revision && localRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: local workspace changed since last sync");
  }

  await mirrorDirectory(state.workspaceDir, localPath, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir, ignoreRules);
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
      snapshot_handle: encodeSnapshotHandle(state.remoteId, snapshotName),
      last_known_status: state.running ? "running" : "stopped",
    },
  });
}

async function restoreSnapshot(request) {
  const state = await loadSandbox(request);
  const snapshotHandle = request.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("restore requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(
    snapshotHandle,
    state.remoteId,
  );
  const source = snapshotDir(remoteId, snapshotName);
  await fs.access(source);
  await mirrorDirectory(source, state.fsRoot, []);
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function deleteSnapshot(request) {
  const snapshotHandle = request.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(snapshotHandle);
  await fs.rm(snapshotDir(remoteId, snapshotName), {
    recursive: true,
    force: true,
  });
  return { success: true };
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

async function hashTree(baseDir, ignoreRules = []) {
  const entries = await collectEntries(baseDir, ignoreRules);
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
